//! The server role: the USB devices on this machine, the user's share choices in `devices.json`,
//! and the device list the UI and the remote clients follow.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use koppel_config::files::{KoppelSettings, Role, SharedDevices};
use koppel_config::json::OrderedMap;
use koppel_config::{ConfigFile, ConfigFileError, Subscription};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::tasks::{BackgroundTasks, Handlers, lock};
use crate::{
    ActivityOutcome, ActivityReason, ActivitySink, Clock, DeviceView, EntityRef, ExportStatus,
    Listener, ServerDevices, UsbDevice, UsbError, UsbHost, WakeSignal, bus_id,
    interface_class_name, speed_label,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const STOP_WAIT: Duration = Duration::from_secs(5);
const WRONG_DEVICE: &str = "A different device is plugged into this port now, so it is not shared. Share it again to use it.";

/// Whether `devices.json` says to share this device: its bus ID is saved, and it is the VID:PID
/// that was shared there. A bus ID is a port, so a different device plugged into the same port
/// is not shared without asking. A share saved before the VID:PID was recorded matches by port.
pub fn shared_expects(shared: &SharedDevices, device: &UsbDevice) -> bool {
    let saved = shared
        .bus_ids
        .as_ref()
        .is_some_and(|ids| ids.contains(&device.bus_id));
    saved
        && shared
            .vid_pids
            .as_ref()
            .and_then(|v| v.get(&device.bus_id))
            .is_none_or(|vid_pid| *vid_pid == device.vid_pid())
}

fn saved_contains(shared: &SharedDevices, bus_id: &str) -> bool {
    shared
        .bus_ids
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|b| b == bus_id))
}

fn serves(role: Role) -> bool {
    matches!(role, Role::Server | Role::Both)
}

/// Why a share choice was not taken.
#[derive(Debug)]
pub enum SetSharedError {
    /// The bus ID is not a USB bus ID. Nothing was changed. The endpoint answers 400.
    InvalidBusId,
    /// The host could not list its devices.
    Host(UsbError),
    /// The kernel took the change, but `devices.json` could not be written, so the change was
    /// undone.
    Save(ConfigFileError),
}

impl fmt::Display for SetSharedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBusId => f.write_str("That is not a USB bus ID."),
            Self::Host(e) => write!(f, "{e}"),
            Self::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SetSharedError {}

/// Server role: tracks the USB devices on this machine, applies the user's share choices to the
/// host, and tells its listeners whenever what the host reports changes. Polls the host every two
/// seconds, which also catches plugging and unplugging. Cheap to clone.
///
/// A snapshot is taken, and given a sequence number, under a lock. It is delivered to the
/// listeners outside that lock, by one pump, in order, and a snapshot that a newer one overtook
/// while the listeners were busy is dropped. Listeners push to the network, so a slow or stuck
/// peer holds up only the pump, never an import, a share or the poll.
#[derive(Clone)]
pub struct UsbServer {
    inner: Arc<Inner>,
}

struct Inner {
    host: Arc<dyn UsbHost>,
    shared: ConfigFile<SharedDevices>,
    settings: ConfigFile<KoppelSettings>,
    activity: Arc<dyn ActivitySink>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    /// Taken while a snapshot is built and queued.
    refresh_gate: tokio::sync::Mutex<()>,
    /// Taken while share choices are applied to the host.
    apply_gate: tokio::sync::Mutex<()>,
    deliveries: mpsc::UnboundedSender<Delivery>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Delivery>>>,
    refresh_requested: WakeSignal,
    tasks: BackgroundTasks,
    listeners: Handlers<ServerDevices>,
    stopping: CancellationToken,
    subscription: Mutex<Option<Subscription>>,
}

#[derive(Default)]
struct State {
    faults: HashMap<String, String>,
    attached_to: HashMap<String, (String, OffsetDateTime)>,
    names: HashMap<String, String>,
    /// The bus IDs Koppel means to have shared right now. Saved shares come back into it at
    /// start-up only when `ShareOnStartup` is on.
    active: HashSet<String>,
    role: Role,
    present: Option<HashSet<String>>,
    last: Option<ServerDevices>,
    current: Arc<ServerDevices>,
    sequence: u64,
}

struct Delivery {
    sequence: u64,
    devices: Arc<ServerDevices>,
    plugs: Vec<PlugEvent>,
}

struct PlugEvent {
    bus_id: String,
    name: String,
    reason: ActivityReason,
    summary: String,
}

impl UsbServer {
    /// A server over `host`, with the share choices in `shared` (`devices.json`, opened with
    /// [`koppel_config::open::devices`]) and the role in `settings`. Nothing runs until
    /// [`Self::start`].
    pub fn new(
        host: Arc<dyn UsbHost>,
        shared: ConfigFile<SharedDevices>,
        settings: ConfigFile<KoppelSettings>,
        activity: Arc<dyn ActivitySink>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let (deliveries, receiver) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                host,
                shared,
                settings,
                activity,
                clock,
                state: Mutex::new(State::default()),
                refresh_gate: tokio::sync::Mutex::new(()),
                apply_gate: tokio::sync::Mutex::new(()),
                deliveries,
                receiver: Mutex::new(Some(receiver)),
                refresh_requested: WakeSignal::new(),
                tasks: BackgroundTasks::default(),
                listeners: Handlers::new("the device list"),
                stopping: CancellationToken::new(),
                subscription: Mutex::new(None),
            }),
        }
    }

    pub fn host(&self) -> &Arc<dyn UsbHost> {
        &self.inner.host
    }

    /// The latest snapshot of the device list.
    pub fn current(&self) -> Arc<ServerDevices> {
        lock(&self.inner.state).current.clone()
    }

    /// Why `devices.json` is not being used, or `None`. Koppel then started with nothing shared.
    pub fn shared_problem(&self) -> Option<String> {
        self.inner.shared.problem()
    }

    /// Calls `listener` with every new snapshot, in order, from one pump.
    pub fn on_changed<F, Fut>(&self, listener: F) -> Listener
    where
        F: Fn(Arc<ServerDevices>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.inner.listeners.add(listener)
    }

    pub fn remove_listener(&self, listener: Listener) {
        self.inner.listeners.remove(listener);
    }

    /// Devices a client may import: bound for export, and the ones the user shared.
    pub async fn exportable(&self) -> Result<Vec<UsbDevice>, UsbError> {
        let devices = self.inner.host.list_devices().await?;
        Ok(devices
            .into_iter()
            .filter(|d| self.inner.is_exportable(d))
            .collect())
    }

    /// Loads `devices.json`, shares what was saved and unbinds strays, takes the first look,
    /// then starts the poll. A host error here is logged and does not stop the start: the poll
    /// applies the saved shares on its next pass.
    pub async fn start(&self) -> Result<(), ConfigFileError> {
        let inner = &self.inner;
        inner.shared.start().await?;
        lock(&inner.state).role = inner.settings.current().role;
        let weak = Arc::downgrade(inner);
        let subscription = inner.settings.subscribe(move |next| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    inner.on_settings(&next).await;
                }
            }
        });
        *lock(&inner.subscription) = Some(subscription);

        let first = async {
            inner.reconcile().await?;
            inner.refresh(false).await
        };
        if let Err(e) = first.await {
            tracing::warn!(
                "Setting up the shared devices at start-up failed; the next poll tries again: {e}"
            );
        }

        let receiver = lock(&inner.receiver).take();
        if let Some(receiver) = receiver {
            inner.tasks.spawn(
                "deliver the device list",
                deliver(Arc::downgrade(inner), receiver, inner.stopping.clone()),
            );
        }
        inner.tasks.spawn(
            "poll the USB devices",
            poll(Arc::downgrade(inner), inner.stopping.clone()),
        );
        inner.tasks.spawn(
            "refresh on request",
            refresh_when_requested(Arc::downgrade(inner), inner.stopping.clone()),
        );
        Ok(())
    }

    /// Stops the poll and the pump, and waits a moment for background work.
    pub async fn stop(&self) {
        let inner = &self.inner;
        if let Some(subscription) = lock(&inner.subscription).take() {
            inner.settings.unsubscribe(subscription);
        }
        inner.shared.stop();
        inner.stopping.cancel();
        inner.tasks.wait(STOP_WAIT).await;
    }

    /// Applies the choice to the host, and only when that worked records it in `devices.json`,
    /// then tells the listeners what the host now reports. A host refusal shows as the device's
    /// fault and nothing is saved, so a share that never took effect is not applied by surprise
    /// at the next start.
    pub async fn set_shared(&self, bus_id: &str, share: bool) -> Result<(), SetSharedError> {
        if !bus_id::is_valid(bus_id) {
            return Err(SetSharedError::InvalidBusId);
        }
        let inner = &self.inner;
        let failure = {
            let _applying = inner.apply_gate.lock().await;
            let devices = inner
                .host
                .list_devices()
                .await
                .map_err(SetSharedError::Host)?;
            let device = devices.into_iter().find(|d| d.bus_id == bus_id);
            let failure = inner.apply_choice(bus_id, share, device.as_ref()).await;
            if failure.is_none() {
                inner
                    .save_choice(bus_id, share, device.as_ref())
                    .await
                    .map_err(SetSharedError::Save)?;
            }
            failure
        };

        if let Err(e) = inner.refresh(true).await {
            tracing::warn!("Reading USB devices failed: {e}");
        }

        let name = inner.name_of(bus_id, bus_id);
        let entity = EntityRef::device(bus_id, &name);
        let reason = if share {
            ActivityReason::Shared
        } else {
            ActivityReason::Unshared
        };
        match failure {
            Some(failure) => inner.activity.record(
                entity,
                reason,
                ActivityOutcome::Failed,
                format!(
                    "{name} was not {}",
                    if share { "shared" } else { "unshared" }
                ),
                Some(failure),
            ),
            None => inner.activity.record(
                entity,
                reason,
                ActivityOutcome::Succeeded,
                if share {
                    format!("{name} is shared")
                } else {
                    format!("{name} is no longer shared")
                },
                None,
            ),
        }
        Ok(())
    }

    /// Called by the data plane when a client imports or releases a device. The listeners hear
    /// about it a moment later, and nobody waits for that.
    pub fn set_attached(&self, bus_id: &str, client_id: Option<&str>) {
        {
            let mut state = lock(&self.inner.state);
            match client_id {
                Some(client) => {
                    let since = self.inner.clock.now();
                    state
                        .attached_to
                        .insert(bus_id.to_owned(), (client.to_owned(), since));
                }
                None => {
                    state.attached_to.remove(bus_id);
                }
            }
        }
        self.inner.refresh_requested.set();
    }
}

/// Takes snapshots off the queue in order and hands the newest to the listeners. Snapshots
/// overtaken while the listeners ran are dropped; their plug events are still recorded.
async fn deliver(
    inner: Weak<Inner>,
    mut receiver: mpsc::UnboundedReceiver<Delivery>,
    stopping: CancellationToken,
) {
    let mut delivered = 0u64;
    loop {
        let first = tokio::select! {
            next = receiver.recv() => next,
            _ = stopping.cancelled() => None,
        };
        let Some(first) = first else {
            return;
        };
        let mut plugs = Vec::new();
        let mut latest: Option<Arc<ServerDevices>> = None;
        let mut batch = vec![first];
        while let Ok(next) = receiver.try_recv() {
            batch.push(next);
        }
        for next in batch {
            plugs.extend(next.plugs);
            if next.sequence > delivered {
                delivered = next.sequence;
                latest = Some(next.devices);
            }
        }
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if let Some(devices) = latest {
            inner.listeners.raise(devices).await;
        }
        for plug in plugs {
            inner.activity.record(
                EntityRef::device(&plug.bus_id, &plug.name),
                plug.reason,
                ActivityOutcome::Succeeded,
                plug.summary,
                None,
            );
        }
    }
}

/// Every two seconds: re-share a device that came back (on a server only; a pure client never
/// binds anything to usbip-host), then look again. Whatever the host does, the poll goes on: it
/// is what notices a plugged device and re-shares one. A host that keeps failing is asked less
/// often, up to every fifteenth tick.
async fn poll(inner: Weak<Inner>, stopping: CancellationToken) {
    let mut timer =
        tokio::time::interval_at(tokio::time::Instant::now() + POLL_INTERVAL, POLL_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut failures = 0u32;
    let mut skip = 0u32;
    loop {
        tokio::select! {
            _ = timer.tick() => {}
            _ = stopping.cancelled() => return,
        }
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let pass = async {
            if serves(lock(&inner.state).role) {
                let _applying = inner.apply_gate.lock().await;
                inner.apply_intent(false).await?;
            }
            inner.refresh(false).await
        };
        match pass.await {
            Ok(()) => failures = 0,
            Err(e) => {
                tracing::warn!("Reading USB devices failed: {e}");
                skip = (1u32 << failures.min(4)).min(14);
                failures += 1;
            }
        }
    }
}

/// Refreshes for the callers that only ask for it, so none of them waits on the host.
async fn refresh_when_requested(inner: Weak<Inner>, stopping: CancellationToken) {
    loop {
        let signal = match inner.upgrade() {
            Some(inner) => inner,
            None => return,
        };
        tokio::select! {
            _ = signal.refresh_requested.wait() => {}
            _ = stopping.cancelled() => return,
        }
        if let Err(e) = signal.refresh(true).await {
            tracing::warn!("Reading USB devices failed: {e}");
        }
    }
}

impl Inner {
    /// Bound for export, and the device the user shared on that port.
    fn is_exportable(&self, device: &UsbDevice) -> bool {
        self.host.is_shared(device)
            && (!self.host.can_share() || shared_expects(&self.shared.current(), device))
    }

    fn name_of(&self, bus_id: &str, otherwise: &str) -> String {
        lock(&self.state)
            .names
            .get(bus_id)
            .cloned()
            .unwrap_or_else(|| otherwise.to_owned())
    }

    /// At start-up: shares what the user saved (when `ShareOnStartup` is on and this is a
    /// server), and unbinds whatever else is bound to usbip-host, such as a share a crash or an
    /// earlier run left behind.
    async fn reconcile(&self) -> Result<(), UsbError> {
        let _applying = self.apply_gate.lock().await;
        self.activate_saved(&self.settings.current());
        self.apply_intent(true).await
    }

    /// Makes the saved shares the ones Koppel means to have, when this machine is a server that
    /// re-shares on start.
    fn activate_saved(&self, settings: &KoppelSettings) {
        let saved = self.shared.current();
        let mut state = lock(&self.state);
        state.active.clear();
        if !serves(settings.role) || !settings.share_on_startup {
            return;
        }
        for bus_id in saved.bus_ids.iter().flatten() {
            state.active.insert(bus_id.clone());
        }
    }

    /// Brings the host in line with what Koppel means to share. A device that is bound but is not
    /// the one the user shared on that port is unbound, and says so. With `unshare_strays`, so is
    /// every other bound device Koppel does not mean to share. Call with the apply gate held.
    async fn apply_intent(&self, unshare_strays: bool) -> Result<(), UsbError> {
        let devices = self.host.list_devices().await?;
        if self.host.unavailable().is_some() || !self.host.can_share() {
            return Ok(());
        }
        let saved = self.shared.current();
        for device in &devices {
            let bus_id = device.bus_id.as_str();
            let (active, faulted) = {
                let state = lock(&self.state);
                (
                    state.active.contains(bus_id),
                    state.faults.contains_key(bus_id),
                )
            };
            if active && shared_expects(&saved, device) {
                if !self.host.is_shared(device) && !faulted {
                    self.try_apply(bus_id, true).await;
                }
            } else if self.host.is_shared(device)
                && self.host.can_unshare()
                && (unshare_strays || saved_contains(&saved, bus_id))
            {
                let wrong_device =
                    saved_contains(&saved, bus_id) && !shared_expects(&saved, device);
                let failure = self.try_apply(bus_id, false).await;
                if failure.is_none() && wrong_device {
                    lock(&self.state)
                        .faults
                        .insert(bus_id.to_owned(), WRONG_DEVICE.to_owned());
                }
            }
        }
        Ok(())
    }

    /// A settings change that moves the role in or out of the server side shares or unshares to
    /// match.
    async fn on_settings(self: &Arc<Self>, next: &KoppelSettings) {
        let was = {
            let mut state = lock(&self.state);
            let was = serves(state.role);
            state.role = next.role;
            was
        };
        if was == serves(next.role) {
            return;
        }

        if was {
            // Giving every device back can take a while. The settings change is confirmed to the
            // UI without it.
            let inner = self.clone();
            self.tasks
                .spawn("unshare on leaving the server role", async move {
                    {
                        let _applying = inner.apply_gate.lock().await;
                        // The role came back while this waited: the handler for that change shares again.
                        if !serves(lock(&inner.state).role) {
                            inner.unshare_everything().await;
                        }
                    }
                    if let Err(e) = inner.refresh(true).await {
                        tracing::warn!("Reading USB devices failed: {e}");
                    }
                });
            return;
        }

        {
            let _applying = self.apply_gate.lock().await;
            self.activate_saved(next);
            if let Err(e) = self.apply_intent(false).await {
                tracing::warn!("Sharing the saved devices failed: {e}");
            }
        }
        if let Err(e) = self.refresh(true).await {
            tracing::warn!("Reading USB devices failed: {e}");
        }
    }

    /// A machine that is no longer a server gives back every device Koppel shared. Call with the
    /// apply gate held.
    async fn unshare_everything(&self) {
        let saved = self.shared.current();
        let devices = self.host.list_devices().await;
        lock(&self.state).active.clear();
        let Ok(devices) = devices else {
            return;
        };
        if self.host.unavailable().is_some() || !self.host.can_unshare() {
            return;
        }
        for device in devices
            .iter()
            .filter(|d| self.host.is_shared(d) && saved_contains(&saved, &d.bus_id))
        {
            self.try_apply(&device.bus_id, false).await;
        }
    }

    /// What the host can do for the choice. Returns why it failed, or `None` when it took effect.
    async fn apply_choice(
        &self,
        bus_id: &str,
        share: bool,
        device: Option<&UsbDevice>,
    ) -> Option<String> {
        if let Some(unavailable) = self.host.unavailable() {
            return Some(self.fail(bus_id, unavailable));
        }
        let can = if share {
            self.host.can_share()
        } else {
            self.host.can_unshare()
        };
        if can {
            return self.try_apply(bus_id, share).await;
        }
        // The host shares by its own tools. A choice that already matches what it exports is
        // nothing to apply.
        if share != device.is_some_and(|d| self.host.is_shared(d)) {
            return Some(self.fail(bus_id, self.host.sharing_advice(bus_id, share)));
        }
        lock(&self.state).faults.remove(bus_id);
        None
    }

    /// Writes the choice to `devices.json` after it took effect. If the file cannot be written,
    /// the host change is undone.
    async fn save_choice(
        &self,
        bus_id: &str,
        share: bool,
        device: Option<&UsbDevice>,
    ) -> Result<(), ConfigFileError> {
        let vid_pid = device.map(UsbDevice::vid_pid);
        let saved = self
            .shared
            .update(|current| {
                let mut bus_ids = current.bus_ids.clone().unwrap_or_default();
                if share {
                    if !bus_ids.iter().any(|b| b == bus_id) {
                        bus_ids.push(bus_id.to_owned());
                    }
                } else {
                    bus_ids.retain(|b| b != bus_id);
                }
                let vid_pids = match (&vid_pid, share) {
                    (Some(vid_pid), true) => {
                        let mut map = current.vid_pids.clone().unwrap_or_else(OrderedMap::new);
                        map.insert(bus_id, vid_pid.clone());
                        Some(map)
                    }
                    _ => current.vid_pids.clone().map(|mut map| {
                        map.remove(bus_id);
                        map
                    }),
                };
                SharedDevices {
                    bus_ids: Some(bus_ids),
                    vid_pids,
                }
            })
            .await;
        if let Err(e) = saved {
            let can_undo = if share {
                self.host.can_unshare()
            } else {
                self.host.can_share()
            };
            if can_undo {
                self.try_apply(bus_id, !share).await;
            }
            return Err(e);
        }
        let mut state = lock(&self.state);
        if share {
            state.active.insert(bus_id.to_owned());
        } else {
            state.active.remove(bus_id);
        }
        Ok(())
    }

    async fn try_apply(&self, bus_id: &str, share: bool) -> Option<String> {
        let result = if share {
            self.host.share(bus_id).await
        } else {
            self.host.unshare(bus_id).await
        };
        match result {
            Ok(()) => {
                lock(&self.state).faults.remove(bus_id);
                None
            }
            Err(e) => Some(self.fail(bus_id, e.message().to_owned())),
        }
    }

    fn fail(&self, bus_id: &str, reason: String) -> String {
        lock(&self.state)
            .faults
            .insert(bus_id.to_owned(), reason.clone());
        tracing::warn!("Could not apply the share choice for {bus_id}: {reason}");
        reason
    }

    /// Builds a snapshot and queues it for the listeners when it changed, or always with `force`.
    /// Only the snapshot and its place in the queue are taken under the gate; listeners run in
    /// the pump.
    async fn refresh(&self, force: bool) -> Result<(), UsbError> {
        let _refreshing = self.refresh_gate.lock().await;
        let next = self.build().await?;
        let mut state = lock(&self.state);
        if !force && state.last.as_ref() == Some(&next) {
            return Ok(());
        }
        state.last = Some(next.clone());
        let next = Arc::new(next);
        state.current = next.clone();
        state.sequence += 1;
        let plugs = plug_changes(&mut state, &next);
        let _ = self.deliveries.send(Delivery {
            sequence: state.sequence,
            devices: next,
            plugs,
        });
        Ok(())
    }

    async fn build(&self) -> Result<ServerDevices, UsbError> {
        let saved = self.shared.current();
        let devices = self.host.list_devices().await?;
        let mut views = Vec::with_capacity(devices.len());
        for d in &devices {
            let name = d
                .product
                .clone()
                .unwrap_or_else(|| "Unknown device".to_owned());
            lock(&self.state)
                .names
                .insert(d.bus_id.clone(), name.clone());
            let state = if self.is_exportable(d) {
                if self.host.status(&d.bus_id).await? == ExportStatus::Used {
                    "attached"
                } else {
                    "shared"
                }
            } else {
                "local"
            };
            let (holder, fault) = {
                let s = lock(&self.state);
                (
                    s.attached_to
                        .get(&d.bus_id)
                        .cloned()
                        .filter(|_| state == "attached"),
                    s.faults.get(&d.bus_id).cloned(),
                )
            };
            let mut interfaces: Vec<String> = Vec::new();
            for interface in &d.interfaces {
                let name = interface_class_name(interface.class);
                if !interfaces.contains(&name) {
                    interfaces.push(name);
                }
            }
            views.push(DeviceView {
                bus_id: d.bus_id.clone(),
                vid_pid: d.vid_pid(),
                name,
                manufacturer: d.manufacturer.clone(),
                speed: speed_label(d.speed).to_owned(),
                state: state.to_owned(),
                attached_to: holder.as_ref().map(|(client, _)| client.clone()),
                attached_since: holder.map(|(_, since)| since),
                fault,
                serial: d.serial.clone(),
                driver: d.driver.clone(),
                interfaces: Some(interfaces),
            });
        }

        // A shared device that is gone stays visible, so the user knows it comes back shared. It
        // shows what was remembered about it: its VID:PID, and its name when this run has seen it.
        let intent: Vec<&String> = saved.bus_ids.iter().flatten().collect();
        let remembered = saved.vid_pids.as_ref();
        let gone: Vec<&String> = intent
            .iter()
            .copied()
            .filter(|bus_id| devices.iter().all(|d| d.bus_id != **bus_id))
            .collect();
        for bus_id in &gone {
            if has_moved(bus_id, &gone, remembered, &devices, &intent) {
                continue;
            }
            let state = lock(&self.state);
            views.push(DeviceView {
                bus_id: (*bus_id).clone(),
                vid_pid: remembered
                    .and_then(|r| r.get(bus_id))
                    .cloned()
                    .unwrap_or_default(),
                name: state
                    .names
                    .get(*bus_id)
                    .cloned()
                    .unwrap_or_else(|| "Unplugged device".to_owned()),
                state: "offline".to_owned(),
                fault: state.faults.get(*bus_id).cloned(),
                ..DeviceView::default()
            });
        }

        views.sort_by(|a, b| a.bus_id.cmp(&b.bus_id));
        Ok(ServerDevices {
            devices: views,
            unavailable: self.host.unavailable(),
        })
    }
}

/// A remembered device that was plugged into another port is listed there, so its old port is
/// not shown as unplugged as well. That holds when its VID:PID is present on another saved bus ID
/// and there are no more gone bus IDs with that VID:PID than present devices. With identical
/// devices the two cannot be told apart, so when more are gone than present, every gone row is
/// kept.
fn has_moved(
    bus_id: &str,
    gone: &[&String],
    remembered: Option<&OrderedMap<String>>,
    present: &[UsbDevice],
    intent: &[&String],
) -> bool {
    let Some(remembered) = remembered else {
        return false;
    };
    let Some(vid_pid) = remembered.get(bus_id) else {
        return false;
    };
    let elsewhere = present
        .iter()
        .filter(|d| d.vid_pid() == *vid_pid && intent.iter().any(|b| **b == d.bus_id))
        .count();
    let gone_same = gone
        .iter()
        .filter(|g| remembered.get(g) == Some(vid_pid))
        .count();
    elsewhere > 0 && gone_same <= elsewhere
}

/// Devices that appeared or disappeared since the last look. The first look only remembers.
fn plug_changes(state: &mut State, devices: &ServerDevices) -> Vec<PlugEvent> {
    let present: HashSet<String> = devices
        .devices
        .iter()
        .filter(|d| d.state != "offline")
        .map(|d| d.bus_id.clone())
        .collect();
    let Some(previous) = state.present.replace(present.clone()) else {
        return Vec::new();
    };
    let mut plugged: Vec<&String> = present.difference(&previous).collect();
    let mut unplugged: Vec<&String> = previous.difference(&present).collect();
    plugged.sort();
    unplugged.sort();
    let name = |bus_id: &str| {
        state
            .names
            .get(bus_id)
            .cloned()
            .unwrap_or_else(|| bus_id.to_owned())
    };
    let mut changes = Vec::new();
    for bus_id in plugged {
        let name = name(bus_id);
        changes.push(PlugEvent {
            summary: format!("{name} was plugged in on bus {bus_id}"),
            bus_id: bus_id.clone(),
            name,
            reason: ActivityReason::Plugged,
        });
    }
    for bus_id in unplugged {
        let name = name(bus_id);
        changes.push(PlugEvent {
            summary: format!("{name} was unplugged from bus {bus_id}"),
            bus_id: bus_id.clone(),
            name,
            reason: ActivityReason::Unplugged,
        });
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(bus_id: &str, vendor_id: u16, product_id: u16) -> UsbDevice {
        UsbDevice {
            bus_id: bus_id.into(),
            vendor_id,
            product_id,
            ..UsbDevice::default()
        }
    }

    #[test]
    fn a_different_device_on_a_shared_port_is_not_shared_again() {
        let mut vid_pids = OrderedMap::new();
        vid_pids.insert("1-1", "1209:0001".to_owned());
        let shared = SharedDevices {
            bus_ids: Some(vec!["1-1".into()]),
            vid_pids: Some(vid_pids),
        };

        assert!(shared_expects(&shared, &device("1-1", 0x1209, 0x0001)));
        assert!(!shared_expects(&shared, &device("1-1", 0x046d, 0xc548)));
        assert!(!shared_expects(&shared, &device("2-1", 0x1209, 0x0001)));
    }

    #[test]
    fn shares_saved_before_vid_pid_was_recorded_still_match_by_port() {
        let shared = SharedDevices {
            bus_ids: Some(vec!["1-1".into()]),
            vid_pids: None,
        };

        assert!(shared_expects(&shared, &device("1-1", 0x064f, 0x2af9)));
    }
}

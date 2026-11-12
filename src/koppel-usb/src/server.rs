//! The server role: the USB devices on this machine, the user's share choices in `devices.json`,
//! and the device list the UI and the remote clients follow.
//!
//! One actor task owns everything that changes ([`Actor`]): which devices Koppel means to share,
//! what went wrong with each, who holds which. The share choices, the poll, a role change and a
//! client's attach all reach it as commands, one after the other, so two of them can never
//! change the host at once. What it learns it publishes through a `watch` channel, and a
//! listener sees the newest snapshot each time it changes.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexmap::IndexMap;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::tasks::{Backoff, Listener, Tasks, listen, lock};
use crate::{
    ActivityOutcome, ActivityReason, ActivitySink, BusId, Clock, DeviceState, DeviceView,
    EntityRef, ExportStatus, Observed, Role, ServerDevices, SharedDevices, Store, StoreError,
    Subscription, UsbDevice, UsbError, UsbHost, UsbSettings, interface_class_name, speed_label,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const STOP_WAIT: Duration = Duration::from_secs(5);
/// A host that keeps failing is asked after 4 seconds, then 8, 16 and at last every 30.
const POLL_BACKOFF_MIN: Duration = Duration::from_secs(4);
const POLL_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Whether `devices.json` says to share this device: its bus ID is saved, and it is the VID:PID
/// that was shared there. A bus ID is a port, so a different device plugged into the same port
/// is not shared without asking. A share saved before the VID:PID was recorded matches by port.
pub fn shared_expects(shared: &SharedDevices, device: &UsbDevice) -> bool {
    let saved = saved_contains(shared, &device.bus_id);
    saved
        && shared
            .vid_pids
            .get(device.bus_id.as_str())
            .is_none_or(|vid_pid| *vid_pid == device.vid_pid())
}

fn saved_contains(shared: &SharedDevices, bus_id: &BusId) -> bool {
    shared.bus_ids.iter().any(|b| b == bus_id.as_str())
}

/// Why a share choice was not taken.
#[derive(Debug, thiserror::Error)]
pub enum SetSharedError {
    /// The bus ID is not a USB bus ID. Nothing was changed. The endpoint answers 400.
    #[error("That is not a USB bus ID.")]
    InvalidBusId,
    /// The host could not list its devices.
    #[error("{0}")]
    Host(#[source] UsbError),
    /// The kernel took the change, but `devices.json` could not be written, so the change was
    /// undone.
    #[error("{0}")]
    Save(#[source] StoreError),
    /// The server stopped before it could answer.
    #[error("The USB server is not running.")]
    Stopped,
}

enum Command {
    /// Applies the saved shares and takes the first look.
    Init {
        reply: oneshot::Sender<()>,
    },
    SetShared {
        bus_id: BusId,
        share: bool,
        reply: oneshot::Sender<Result<(), SetSharedError>>,
    },
    /// A client imported (`Some`) or released (`None`) a device.
    Attached {
        bus_id: BusId,
        client: Option<(String, OffsetDateTime)>,
    },
    SettingsChanged {
        role: Role,
        reply: oneshot::Sender<()>,
    },
    Stop {
        reply: oneshot::Sender<()>,
    },
}

struct Shared {
    host: Arc<dyn UsbHost>,
    shared: Arc<dyn Store<SharedDevices>>,
    settings: Arc<dyn Observed<UsbSettings>>,
    activity: Arc<dyn ActivitySink>,
    clock: Arc<dyn Clock>,
    commands: mpsc::UnboundedSender<Command>,
    snapshot: watch::Receiver<Arc<ServerDevices>>,
    listeners: Tasks,
    actor_stop: CancellationToken,
}

impl Shared {
    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Bound for export, and the device the user shared on that port.
    fn is_exportable(&self, device: &UsbDevice) -> bool {
        self.host.is_shared(device)
            && (!self.host.capabilities().share || shared_expects(&self.shared.current(), device))
    }
}

/// What a [`UsbServer`] is made from.
pub struct ServerDeps {
    pub host: Arc<dyn UsbHost>,
    /// `devices.json`.
    pub shared: Arc<dyn Store<SharedDevices>>,
    pub settings: Arc<dyn Observed<UsbSettings>>,
    pub activity: Arc<dyn ActivitySink>,
    pub clock: Arc<dyn Clock>,
}

/// Server role: tracks the USB devices on this machine, applies the user's share choices to the
/// host, and tells its listeners whenever what the host reports changes. Polls the host every two
/// seconds, which also catches plugging and unplugging. Cheap to clone.
///
/// A snapshot is built by the actor, between two commands, and published. Listeners push to the
/// network, so a slow or stuck peer holds up only its own listener, never an import, a share or
/// the poll.
#[derive(Clone)]
pub struct UsbServer {
    shared: Arc<Shared>,
    run: Arc<Mutex<Option<Actor>>>,
    actor: Arc<Mutex<Option<JoinHandle<()>>>>,
    subscription: Arc<Mutex<Option<Subscription>>>,
}

impl UsbServer {
    /// A server over `deps.host`, with the share choices in `devices.json` and the role in the
    /// settings. Nothing runs until [`Self::start`].
    pub fn new(deps: ServerDeps) -> Self {
        let (commands_tx, commands) = mpsc::unbounded_channel();
        let (snapshot_tx, snapshot) = watch::channel(Arc::new(ServerDevices::default()));
        let shared = Arc::new(Shared {
            host: deps.host,
            shared: deps.shared,
            settings: deps.settings,
            activity: deps.activity,
            clock: deps.clock,
            commands: commands_tx,
            snapshot,
            listeners: Tasks::default(),
            actor_stop: CancellationToken::new(),
        });
        let actor = Actor {
            shared: shared.clone(),
            commands,
            snapshot: snapshot_tx,
            faults: HashMap::new(),
            attached_to: HashMap::new(),
            names: HashMap::new(),
            active: HashSet::new(),
            role: Role::default(),
            present: None,
            last: None,
            backoff: Backoff::new(POLL_BACKOFF_MIN, POLL_BACKOFF_MAX),
            not_before: None,
        };
        Self {
            shared,
            run: Arc::new(Mutex::new(Some(actor))),
            actor: Arc::new(Mutex::new(None)),
            subscription: Arc::new(Mutex::new(None)),
        }
    }

    pub fn host(&self) -> &Arc<dyn UsbHost> {
        &self.shared.host
    }

    /// The latest snapshot of the device list.
    pub fn current(&self) -> Arc<ServerDevices> {
        self.shared.snapshot.borrow().clone()
    }

    /// Why `devices.json` is not being used, or `None`. Koppel then started with nothing shared.
    pub fn shared_problem(&self) -> Option<String> {
        self.shared.shared.problem()
    }

    /// Calls `listener` with every new snapshot, newest first when it falls behind.
    pub fn on_changed<F, Fut>(&self, listener: F) -> Listener
    where
        F: Fn(Arc<ServerDevices>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        listen(
            &self.shared.listeners,
            "the device list",
            self.shared.snapshot.clone(),
            listener,
        )
    }

    pub fn remove_listener(&self, listener: Listener) {
        listener.stop();
    }

    /// Devices a client may import: bound for export, and the ones the user shared.
    pub async fn exportable(&self) -> Result<Vec<UsbDevice>, UsbError> {
        let devices = self.shared.host.list_devices().await?;
        Ok(devices
            .into_iter()
            .filter(|d| self.shared.is_exportable(d))
            .collect())
    }

    /// Loads `devices.json`, shares what was saved and unbinds strays, takes the first look,
    /// then starts the poll. A host error here is logged and does not stop the start: the poll
    /// applies the saved shares on its next pass.
    pub async fn start(&self) -> Result<(), StoreError> {
        let shared = &self.shared;
        shared.shared.start().await?;
        let Some(actor) = lock(&self.run).take() else {
            return Ok(());
        };
        *lock(&self.actor) = Some(tokio::spawn(actor.run()));

        let (reply, ready) = oneshot::channel();
        shared.send(Command::Init { reply });
        let _ = ready.await;

        let weak = Arc::downgrade(shared);
        let subscription = shared.settings.subscribe(Box::new(move |next| {
            let weak = weak.clone();
            Box::pin(async move {
                if let Some(shared) = weak.upgrade() {
                    let (reply, applied) = oneshot::channel();
                    shared.send(Command::SettingsChanged {
                        role: next.role,
                        reply,
                    });
                    let _ = applied.await;
                }
            })
        }));
        *lock(&self.subscription) = Some(subscription);
        Ok(())
    }

    /// Stops the poll and the listeners, and waits a moment for background work.
    pub async fn stop(&self) {
        let shared = &self.shared;
        drop(lock(&self.subscription).take());
        shared.shared.stop();
        let (reply, stopped) = oneshot::channel();
        shared.send(Command::Stop { reply });
        let _ = tokio::time::timeout(STOP_WAIT, stopped).await;
        shared.listeners.stop(STOP_WAIT).await;
        shared.actor_stop.cancel();
        let actor = lock(&self.actor).take();
        if let Some(actor) = actor {
            let _ = tokio::time::timeout(STOP_WAIT, actor).await;
        }
    }

    /// Applies the choice to the host, and only when that worked records it in `devices.json`,
    /// then tells the listeners what the host now reports. A host refusal shows as the device's
    /// fault and nothing is saved, so a share that never took effect is not applied by surprise
    /// at the next start.
    pub async fn set_shared(&self, bus_id: &str, share: bool) -> Result<(), SetSharedError> {
        let bus_id: BusId = bus_id.parse().map_err(|_| SetSharedError::InvalidBusId)?;
        let (reply, done) = oneshot::channel();
        self.shared.send(Command::SetShared {
            bus_id,
            share,
            reply,
        });
        done.await.map_err(|_| SetSharedError::Stopped)?
    }

    /// Called by the data plane when a client imports or releases a device. The listeners hear
    /// about it a moment later, and nobody waits for that.
    pub fn set_attached(&self, bus_id: &BusId, client_id: Option<&str>) {
        let client = client_id.map(|c| (c.to_owned(), self.shared.clock.now()));
        self.shared.send(Command::Attached {
            bus_id: bus_id.clone(),
            client,
        });
    }
}

struct Actor {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
    snapshot: watch::Sender<Arc<ServerDevices>>,
    faults: HashMap<BusId, String>,
    attached_to: HashMap<BusId, (String, OffsetDateTime)>,
    names: HashMap<BusId, String>,
    /// The bus IDs Koppel means to have shared right now. Saved shares come back into it at
    /// start-up only when `ShareOnStartup` is on.
    active: HashSet<BusId>,
    role: Role,
    present: Option<HashSet<BusId>>,
    last: Option<ServerDevices>,
    /// How long a failing host is left alone, and until when it is now.
    backoff: Backoff,
    not_before: Option<tokio::time::Instant>,
}

impl Actor {
    async fn run(mut self) {
        let mut timer =
            tokio::time::interval_at(tokio::time::Instant::now() + POLL_INTERVAL, POLL_INTERVAL);
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let stop = self.shared.actor_stop.clone();
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(Command::Stop { reply }) => {
                        let _ = reply.send(());
                    }
                    Some(command) => self.handle(command).await,
                    None => return,
                },
                _ = timer.tick() => self.poll().await,
                () = stop.cancelled() => return,
            }
        }
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Init { reply } => {
                self.role = self.shared.settings.current().role;
                let first = async {
                    self.activate_saved(&self.shared.settings.current());
                    let mut devices = self.shared.host.list_devices().await?;
                    if self.apply_intent(&devices, true).await {
                        devices = self.shared.host.list_devices().await?;
                    }
                    self.refresh(&devices, false).await
                };
                if let Err(e) = first.await {
                    tracing::warn!(
                        "Setting up the shared devices at start-up failed; the next poll tries again: {e}"
                    );
                }
                let _ = reply.send(());
            }
            Command::SetShared {
                bus_id,
                share,
                reply,
            } => {
                let _ = reply.send(self.set_shared(bus_id, share).await);
            }
            Command::Attached { bus_id, client } => {
                match client {
                    Some(client) => {
                        self.attached_to.insert(bus_id, client);
                    }
                    None => {
                        self.attached_to.remove(&bus_id);
                    }
                }
                self.look(true).await;
            }
            Command::SettingsChanged { role, reply } => self.on_settings(role, reply).await,
            Command::Stop { .. } => {}
        }
    }

    /// Every two seconds: re-share a device that came back (on a server only; a pure client never
    /// binds anything to usbip-host), then look again. Whatever the host does, the poll goes on: it
    /// is what notices a plugged device and re-shares one. A host that keeps failing is asked less
    /// often, up to every 30 seconds. One scan of the host serves both the sharing and the
    /// snapshot; only when the sharing changed the host is it looked at again.
    async fn poll(&mut self) {
        if self
            .not_before
            .is_some_and(|until| self.shared.clock.monotonic() < until)
        {
            return;
        }
        let pass = async {
            let mut devices = self.shared.host.list_devices().await?;
            if self.role.serves() && self.apply_intent(&devices, false).await {
                devices = self.shared.host.list_devices().await?;
            }
            self.refresh(&devices, false).await
        };
        match pass.await {
            Ok(()) => {
                self.backoff.reset();
                self.not_before = None;
            }
            Err(e) => {
                tracing::warn!("Reading USB devices failed: {e}");
                self.not_before = Some(self.shared.clock.monotonic() + self.backoff.next_wait());
            }
        }
    }

    /// Looks at the host and publishes, always with `force`, otherwise when it changed.
    async fn look(&mut self, force: bool) {
        let looked = async {
            let devices = self.shared.host.list_devices().await?;
            self.refresh(&devices, force).await
        };
        if let Err(e) = looked.await {
            tracing::warn!("Reading USB devices failed: {e}");
        }
    }

    /// A settings change that moves the role in or out of the server side shares or unshares to
    /// match. The settings update does not wait for giving every device back, which can take a
    /// while.
    async fn on_settings(&mut self, role: Role, reply: oneshot::Sender<()>) {
        let was = self.role.serves();
        self.role = role;
        if was == role.serves() {
            let _ = reply.send(());
            return;
        }
        if was {
            let _ = reply.send(());
            self.unshare_everything().await;
            self.look(true).await;
            return;
        }
        self.activate_saved(&self.shared.settings.current());
        let applied = async {
            let mut devices = self.shared.host.list_devices().await?;
            if self.apply_intent(&devices, false).await {
                devices = self.shared.host.list_devices().await?;
            }
            Ok::<_, UsbError>(devices)
        };
        match applied.await {
            Ok(devices) => {
                if let Err(e) = self.refresh(&devices, true).await {
                    tracing::warn!("Reading USB devices failed: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("Sharing the saved devices failed: {e}");
                self.look(true).await;
            }
        }
        let _ = reply.send(());
    }

    async fn set_shared(&mut self, bus_id: BusId, share: bool) -> Result<(), SetSharedError> {
        let devices = self
            .shared
            .host
            .list_devices()
            .await
            .map_err(SetSharedError::Host)?;
        let device = devices.into_iter().find(|d| d.bus_id == bus_id);
        let failure = self.apply_choice(&bus_id, share, device.as_ref()).await;
        if failure.is_none() {
            self.save_choice(&bus_id, share, device.as_ref())
                .await
                .map_err(SetSharedError::Save)?;
        }

        self.look(true).await;

        let name = self.name_of(&bus_id);
        let entity = EntityRef::device(&bus_id, &name);
        let reason = if share {
            ActivityReason::Shared
        } else {
            ActivityReason::Unshared
        };
        match failure {
            Some(failure) => self.shared.activity.record(
                entity,
                reason,
                ActivityOutcome::Failed,
                format!(
                    "{name} was not {}",
                    if share { "shared" } else { "unshared" }
                ),
                Some(failure),
            ),
            None => self.shared.activity.record(
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

    fn name_of(&self, bus_id: &BusId) -> String {
        self.names
            .get(bus_id)
            .cloned()
            .unwrap_or_else(|| bus_id.to_string())
    }

    /// Makes the saved shares the ones Koppel means to have, when this machine is a server that
    /// re-shares on start.
    fn activate_saved(&mut self, settings: &UsbSettings) {
        let saved = self.shared.shared.current();
        self.active.clear();
        if !settings.role.serves() || !settings.share_on_startup {
            return;
        }
        for bus_id in &saved.bus_ids {
            if let Ok(bus_id) = bus_id.parse() {
                self.active.insert(bus_id);
            }
        }
    }

    /// Brings the host in line with what Koppel means to share. A device that is bound but is not
    /// the one the user shared on that port is unbound, and says so. With `unshare_strays`, so is
    /// every other bound device Koppel does not mean to share. Returns true when it asked the
    /// host to change anything.
    async fn apply_intent(&mut self, devices: &[UsbDevice], unshare_strays: bool) -> bool {
        let host = self.shared.host.clone();
        if host.unavailable().is_some() || !host.capabilities().share {
            return false;
        }
        let saved = self.shared.shared.current();
        let mut changed = false;
        for device in devices {
            let bus_id = &device.bus_id;
            let active = self.active.contains(bus_id);
            let faulted = self.faults.contains_key(bus_id);
            if active && shared_expects(&saved, device) {
                if !host.is_shared(device) && !faulted {
                    self.try_apply(bus_id, true).await;
                    changed = true;
                }
            } else if host.is_shared(device)
                && host.capabilities().unshare
                && (unshare_strays || saved_contains(&saved, bus_id))
            {
                let wrong_device =
                    saved_contains(&saved, bus_id) && !shared_expects(&saved, device);
                let failure = self.try_apply(bus_id, false).await;
                changed = true;
                if failure.is_none() && wrong_device {
                    self.faults
                        .insert(bus_id.clone(), UsbError::WrongDevice.to_string());
                }
            }
        }
        changed
    }

    /// A machine that is no longer a server gives back every device Koppel shared.
    async fn unshare_everything(&mut self) {
        let saved = self.shared.shared.current();
        let devices = self.shared.host.list_devices().await;
        self.active.clear();
        let Ok(devices) = devices else {
            return;
        };
        if self.shared.host.unavailable().is_some() || !self.shared.host.capabilities().unshare {
            return;
        }
        let shared: Vec<BusId> = devices
            .iter()
            .filter(|d| self.shared.host.is_shared(d) && saved_contains(&saved, &d.bus_id))
            .map(|d| d.bus_id.clone())
            .collect();
        for bus_id in shared {
            self.try_apply(&bus_id, false).await;
        }
    }

    /// What the host can do for the choice. Returns why it failed, or `None` when it took effect.
    async fn apply_choice(
        &mut self,
        bus_id: &BusId,
        share: bool,
        device: Option<&UsbDevice>,
    ) -> Option<String> {
        let host = self.shared.host.clone();
        if let Some(unavailable) = host.unavailable() {
            return Some(self.fail(bus_id, unavailable));
        }
        let can = if share {
            host.capabilities().share
        } else {
            host.capabilities().unshare
        };
        if can {
            return self.try_apply(bus_id, share).await;
        }
        // The host shares by its own tools. A choice that already matches what it exports is
        // nothing to apply.
        if share != device.is_some_and(|d| host.is_shared(d)) {
            return Some(self.fail(bus_id, host.sharing_advice(bus_id, share)));
        }
        self.faults.remove(bus_id);
        None
    }

    /// Writes the choice to `devices.json` after it took effect. If the file cannot be written,
    /// the host change is undone.
    async fn save_choice(
        &mut self,
        bus_id: &BusId,
        share: bool,
        device: Option<&UsbDevice>,
    ) -> Result<(), StoreError> {
        let vid_pid = device.map(UsbDevice::vid_pid);
        let saved = self
            .shared
            .shared
            .update(Box::new(|current| {
                let mut next = current.clone();
                if share {
                    if !next.bus_ids.iter().any(|b| b == bus_id.as_str()) {
                        next.bus_ids.push(bus_id.to_string());
                    }
                } else {
                    next.bus_ids.retain(|b| b != bus_id.as_str());
                }
                match (&vid_pid, share) {
                    (Some(vid_pid), true) => {
                        next.vid_pids
                            .insert(bus_id.as_str().to_owned(), vid_pid.clone());
                    }
                    _ => {
                        next.vid_pids.shift_remove(bus_id.as_str());
                    }
                }
                next
            }))
            .await;
        if let Err(e) = saved {
            let can_undo = if share {
                self.shared.host.capabilities().unshare
            } else {
                self.shared.host.capabilities().share
            };
            if can_undo {
                self.try_apply(bus_id, !share).await;
            }
            return Err(e);
        }
        if share {
            self.active.insert(bus_id.clone());
        } else {
            self.active.remove(bus_id);
        }
        Ok(())
    }

    async fn try_apply(&mut self, bus_id: &BusId, share: bool) -> Option<String> {
        let result = if share {
            self.shared.host.share(bus_id).await
        } else {
            self.shared.host.unshare(bus_id).await
        };
        match result {
            Ok(()) => {
                self.faults.remove(bus_id);
                None
            }
            Err(e) => {
                tracing::debug!("Applying the share choice for {bus_id} failed: {e:?}");
                Some(self.fail(bus_id, e.to_string()))
            }
        }
    }

    fn fail(&mut self, bus_id: &BusId, reason: String) -> String {
        self.faults.insert(bus_id.clone(), reason.clone());
        tracing::warn!("Could not apply the share choice for {bus_id}: {reason}");
        reason
    }

    /// Builds a snapshot from the devices just listed and publishes it when it changed, or always
    /// with `force`.
    async fn refresh(&mut self, devices: &[UsbDevice], force: bool) -> Result<(), UsbError> {
        let next = self.build(devices).await;
        if !force && self.last.as_ref() == Some(&next) {
            return Ok(());
        }
        self.last = Some(next.clone());
        let next = Arc::new(next);
        let plugs = self.plug_changes(&next);
        self.snapshot.send_replace(next);
        for plug in plugs {
            self.shared.activity.record(
                EntityRef::device(&plug.bus_id, &plug.name),
                plug.reason,
                ActivityOutcome::Succeeded,
                plug.summary,
                None,
            );
        }
        Ok(())
    }

    async fn build(&mut self, devices: &[UsbDevice]) -> ServerDevices {
        let saved = self.shared.shared.current();
        let mut views = Vec::with_capacity(devices.len());
        for d in devices {
            let name = d
                .product
                .clone()
                .unwrap_or_else(|| "Unknown device".to_owned());
            self.names.insert(d.bus_id.clone(), name.clone());
            let mut fault = self.faults.get(&d.bus_id).cloned();
            let state = if self.shared.is_exportable(d) {
                match self.shared.host.status(&d.bus_id).await {
                    Ok(ExportStatus::Used) => DeviceState::Attached,
                    Ok(_) => DeviceState::Shared,
                    Err(e) => {
                        // One device that cannot be read does not take the others with it.
                        tracing::warn!("Could not read the state of {}: {e}", d.bus_id);
                        fault.get_or_insert_with(|| {
                            format!("Koppel could not read the state of this device: {e}")
                        });
                        DeviceState::Shared
                    }
                }
            } else {
                DeviceState::Local
            };
            let holder = self
                .attached_to
                .get(&d.bus_id)
                .cloned()
                .filter(|_| state == DeviceState::Attached);
            let mut interfaces: Vec<String> = Vec::new();
            for interface in &d.interfaces {
                let name = interface_class_name(interface.class);
                if !interfaces.contains(&name) {
                    interfaces.push(name);
                }
            }
            views.push(DeviceView {
                vid_pid: d.vid_pid(),
                name,
                manufacturer: d.manufacturer.clone(),
                speed: speed_label(d.speed).to_owned(),
                state,
                attached_to: holder.as_ref().map(|(client, _)| client.clone()),
                attached_since: holder.map(|(_, since)| since),
                fault,
                serial: d.serial.clone(),
                driver: d.driver.clone(),
                interfaces: Some(interfaces),
                ..DeviceView::new(d.bus_id.clone())
            });
        }

        // A shared device that is gone stays visible, so the user knows it comes back shared. It
        // shows what was remembered about it: its VID:PID, and its name when this run has seen it.
        let intent: Vec<BusId> = saved
            .bus_ids
            .iter()
            .filter_map(|b| b.parse().ok())
            .collect();
        let remembered = &saved.vid_pids;
        let gone: Vec<&BusId> = intent
            .iter()
            .filter(|bus_id| devices.iter().all(|d| d.bus_id != **bus_id))
            .collect();
        for bus_id in &gone {
            if has_moved(bus_id, &gone, remembered, devices, &intent) {
                continue;
            }
            views.push(DeviceView {
                vid_pid: remembered.get(bus_id.as_str()).cloned().unwrap_or_default(),
                name: self
                    .names
                    .get(*bus_id)
                    .cloned()
                    .unwrap_or_else(|| "Unplugged device".to_owned()),
                state: DeviceState::Offline,
                fault: self.faults.get(*bus_id).cloned(),
                ..DeviceView::new((*bus_id).clone())
            });
        }

        views.sort_by(|a, b| a.bus_id.cmp(&b.bus_id));
        ServerDevices {
            devices: views,
            unavailable: self.shared.host.unavailable(),
        }
    }

    /// Devices that appeared or disappeared since the last look. The first look only remembers.
    fn plug_changes(&mut self, devices: &ServerDevices) -> Vec<PlugEvent> {
        let present: HashSet<BusId> = devices
            .devices
            .iter()
            .filter(|d| d.state != DeviceState::Offline)
            .map(|d| d.bus_id.clone())
            .collect();
        let Some(previous) = self.present.replace(present.clone()) else {
            return Vec::new();
        };
        let mut plugged: Vec<&BusId> = present.difference(&previous).collect();
        let mut unplugged: Vec<&BusId> = previous.difference(&present).collect();
        plugged.sort();
        unplugged.sort();
        let mut changes = Vec::new();
        for (bus_id, plugged_in) in plugged
            .into_iter()
            .map(|b| (b, true))
            .chain(unplugged.into_iter().map(|b| (b, false)))
        {
            let name = self.name_of(bus_id);
            changes.push(PlugEvent {
                summary: if plugged_in {
                    format!("{name} was plugged in on bus {bus_id}")
                } else {
                    format!("{name} was unplugged from bus {bus_id}")
                },
                bus_id: bus_id.clone(),
                name,
                reason: if plugged_in {
                    ActivityReason::Plugged
                } else {
                    ActivityReason::Unplugged
                },
            });
        }
        changes
    }
}

struct PlugEvent {
    bus_id: BusId,
    name: String,
    reason: ActivityReason,
    summary: String,
}

/// A remembered device that was plugged into another port is listed there, so its old port is
/// not shown as unplugged as well. That holds when its VID:PID is present on another saved bus ID
/// and there are no more gone bus IDs with that VID:PID than present devices. With identical
/// devices the two cannot be told apart, so when more are gone than present, every gone row is
/// kept.
fn has_moved(
    bus_id: &BusId,
    gone: &[&BusId],
    remembered: &IndexMap<String, String>,
    present: &[UsbDevice],
    intent: &[BusId],
) -> bool {
    let Some(vid_pid) = remembered.get(bus_id.as_str()) else {
        return false;
    };
    let elsewhere = present
        .iter()
        .filter(|d| d.vid_pid() == *vid_pid && intent.contains(&d.bus_id))
        .count();
    let gone_same = gone
        .iter()
        .filter(|g| remembered.get(g.as_str()) == Some(vid_pid))
        .count();
    elsewhere > 0 && gone_same <= elsewhere
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(bus_id: &str, vendor_id: u16, product_id: u16) -> UsbDevice {
        UsbDevice {
            vendor_id,
            product_id,
            ..UsbDevice::new(bus_id.parse().unwrap())
        }
    }

    #[test]
    fn a_different_device_on_a_shared_port_is_not_shared_again() {
        let mut vid_pids = IndexMap::new();
        vid_pids.insert("1-1".to_owned(), "1209:0001".to_owned());
        let shared = SharedDevices {
            bus_ids: vec!["1-1".into()],
            vid_pids,
        };

        assert!(shared_expects(&shared, &device("1-1", 0x1209, 0x0001)));
        assert!(!shared_expects(&shared, &device("1-1", 0x046d, 0xc548)));
        assert!(!shared_expects(&shared, &device("2-1", 0x1209, 0x0001)));
    }

    #[test]
    fn shares_saved_before_vid_pid_was_recorded_still_match_by_port() {
        let shared = SharedDevices {
            bus_ids: vec!["1-1".into()],
            vid_pids: IndexMap::new(),
        };

        assert!(shared_expects(&shared, &device("1-1", 0x064f, 0x2af9)));
    }
}

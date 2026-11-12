//! The client's actor: the one task that owns the link, the server's device list and the slots.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use koppel_config::files::{ClientConnection, Role};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::slots::{
    AttachKey, Attachment, DriverHold, Finish, Kind, Mode, Outcome, Plan, RegisterError, Slots,
};
use super::url::{authority, continues, url_host};
use super::{Command, Deps, Epoch, Link, LinkEvent, LinkEvents, LinkStatus, Shared, View, attach};
use crate::tasks::Tasks;
use crate::{
    ActivityOutcome, ActivityReason, BusId, ClientDeviceView, ClientDevices, DeviceState,
    EntityRef, LinkState, RemoteDevices,
};

/// What a change of settings or connection amounted to.
pub(crate) enum Changed {
    Nothing,
    /// The same server, with `was` as the connection before.
    SameServer(Box<ClientConnection>),
    /// A new server, new credentials or a role that is no longer client: everything was released.
    Released,
}

/// Why a release runs, so its end is handled right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The person detached the device and waits for it.
    User,
    /// The connection changed or Koppel is stopping.
    Epoch,
    /// An attach finished after it was told to stop.
    Late,
}

struct CurrentLink {
    id: u64,
    link: Arc<dyn Link>,
    /// The connection it was made for, which moves on as the link learns.
    connection: ClientConnection,
}

pub(crate) struct Actor {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
    view_tx: watch::Sender<Arc<View>>,
    devices_tx: watch::Sender<Arc<ClientDevices>>,
    stop: CancellationToken,

    slots: Slots,
    epoch: Arc<Epoch>,
    link: Option<CurrentLink>,
    next_link: u64,
    remote: Arc<RemoteDevices>,
    remote_generation: u64,
    listed: bool,
    link_state: LinkState,
    link_message: Option<String>,
    applied: (bool, ClientConnection),
    gateway_port: Option<u16>,
    gateway_fault: Option<String>,
    /// Cancelled once every release a change started is done. A new attach waits for it.
    release_done: CancellationToken,
    /// Release steps a change started that have not ended, and the list generation to keep.
    epoch_releases: u32,
    clear_remote_if: Option<u64>,
    waiters: HashMap<AttachKey, Vec<oneshot::Sender<()>>>,
    detaching: HashSet<BusId>,
    last_try: HashMap<BusId, Instant>,
    /// The last reason an automatic attach failed, so a repeat is not logged again.
    reported: HashMap<AttachKey, String>,
    stopping: bool,
}

/// The shared half and the actor, made together.
pub(crate) fn build(deps: Deps) -> (Arc<Shared>, Actor) {
    let (commands_tx, commands) = mpsc::unbounded_channel();
    let epoch = Arc::new(Epoch {
        id: 1,
        token: CancellationToken::new(),
    });
    let remote = Arc::new(RemoteDevices::default());
    let view = Arc::new(View {
        epoch: epoch.clone(),
        link: None,
        listed: false,
        gateway_port: None,
        gateway_fault: None,
        remote: remote.clone(),
    });
    let (view_tx, view_rx) = watch::channel(view);
    let (devices_tx, devices_rx) = watch::channel(Arc::new(ClientDevices {
        link: LinkState::Unconfigured,
        link_message: None,
        devices: Vec::new(),
        unavailable: None,
        gateway: false,
        gateway_fault: None,
    }));
    let shared = Arc::new(Shared {
        settings: deps.settings,
        connection: deps.connection,
        remembered: deps.remembered,
        attacher: deps.attacher,
        driver: deps.driver,
        links: deps.links,
        activity: deps.activity,
        clock: deps.clock,
        options: deps.options,
        commands: commands_tx,
        view: view_rx,
        devices: devices_rx,
        workers: Tasks::default(),
        listeners: Tasks::default(),
        actor_stop: CancellationToken::new(),
        attachment_ids: AtomicU64::new(1),
        attach_starts: AtomicUsize::new(0),
        changes_applied: AtomicUsize::new(0),
    });
    let released = CancellationToken::new();
    released.cancel();
    let actor = Actor {
        shared: shared.clone(),
        commands,
        view_tx,
        devices_tx,
        stop: shared.actor_stop.clone(),
        slots: Slots::new(),
        epoch,
        link: None,
        next_link: 1,
        remote,
        remote_generation: 0,
        listed: false,
        link_state: LinkState::Unconfigured,
        link_message: None,
        applied: (false, ClientConnection::default()),
        gateway_port: None,
        gateway_fault: None,
        release_done: released,
        epoch_releases: 0,
        clear_remote_if: None,
        waiters: HashMap::new(),
        detaching: HashSet::new(),
        last_try: HashMap::new(),
        reported: HashMap::new(),
        stopping: false,
    };
    (shared, actor)
}

impl Shared {
    /// A settings or connection change reached the client. Returns once the actor has applied
    /// it, which is when the settings update, and the UI's confirmation, may go on.
    pub(super) async fn on_changed(&self) {
        let (reply, answer) = oneshot::channel();
        self.send(Command::SettingsChanged { reply });
        let Ok(Changed::SameServer(was)) = answer.await else {
            return;
        };
        self.carry_over_remembered(&was).await;
    }

    /// The server switched to its next certificate: the devices remembered for it stay
    /// remembered under the new fingerprint, so they attach again after a restart. Nothing to do
    /// when only the next one changed.
    async fn carry_over_remembered(&self, before: &ClientConnection) {
        let after = self.connection.current().server_fingerprint.clone();
        let Some(after) = after else {
            return;
        };
        if Some(&after) == before.server_fingerprint.as_ref() {
            return;
        }
        let old = before.server_fingerprint.clone();
        let carried = self
            .remembered
            .update(|r| {
                let mut next = r.clone();
                if r.server_fingerprint == old {
                    next.server_fingerprint = Some(after.clone());
                }
                next
            })
            .await;
        if let Err(e) = carried {
            tracing::error!("Could not apply a change of connection or role: {e}");
        }
    }
}

impl Actor {
    pub(crate) async fn run(mut self) {
        self.applied = self.settled();
        self.apply_link();
        self.publish();

        let every = self.shared.options.retry_interval;
        // A remembered device that could not be attached yet, because it was in use or the server
        // was still starting, is tried again while the link is up.
        let mut timer = tokio::time::interval_at(Instant::now() + every, every);
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => self.handle(command),
                    None => return,
                },
                _ = timer.tick() => {
                    if !self.stopping {
                        self.attach_remembered();
                        self.publish();
                    }
                }
                _ = self.stop.cancelled() => return,
            }
        }
    }

    fn settled(&self) -> (bool, ClientConnection) {
        (
            self.shared.settings.current().role != Role::Server,
            (*self.shared.connection.current()).clone(),
        )
    }

    fn key_of(&self, bus_id: &BusId) -> AttachKey {
        AttachKey {
            epoch: self.epoch.id,
            bus_id: bus_id.clone(),
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Attach { bus_id, automatic } => {
                if !automatic {
                    let key = self.key_of(&bus_id);
                    self.slots.clear_fault(&key);
                    self.reported.remove(&key);
                }
                self.start_attach(&bus_id, automatic);
            }
            Command::Detach { bus_id, reply } => self.detach(bus_id, reply),
            Command::Forgotten { bus_id, fault } => {
                self.detaching.remove(&bus_id);
                if let Some(fault) = fault {
                    let key = self.key_of(&bus_id);
                    self.slots.record_fault(&key, fault);
                }
                self.publish();
            }
            Command::Link { id, event } => self.on_link_event(id, event),
            Command::SettingsChanged { reply } => {
                let _ = reply.send(self.on_changed());
            }
            Command::Gateway { port, fault } => {
                self.gateway_port = port;
                self.gateway_fault = fault;
                self.publish();
            }
            Command::Book { key, reply } => {
                let _ = reply.send(self.slots.claim_booking(&key));
            }
            Command::Register {
                key,
                attempt,
                attachment,
                reply,
            } => {
                let current = key.epoch == self.epoch.id && !self.stopping;
                let result = if current {
                    self.slots.register(&key, attempt, attachment)
                } else {
                    Err(RegisterError::Cancelled)
                };
                if result.is_ok() {
                    self.reported.remove(&key);
                }
                let _ = reply.send(result);
                self.publish();
            }
            Command::Unregister {
                key,
                attachment,
                reply,
            } => {
                let removed = self.slots.relay_ended(&key, attachment).is_some();
                let _ = reply.send(removed);
                self.publish();
            }
            Command::RelayEnded { key, attachment } => {
                if let Some(ended) = self.slots.relay_ended(&key, attachment)
                    && ended.kind == Kind::Kernel
                    && let Some(port) = ended.port
                {
                    // The link dropped without a detach; free the port.
                    let _ = self.shared.attacher.detach(port);
                }
                self.publish();
            }
            Command::AttachFinished {
                key,
                attempt,
                automatic,
                outcome,
            } => self.attach_finished(&key, attempt, automatic, outcome),
            Command::ReleaseFinished {
                key,
                origin,
                failure,
                hold,
            } => self.release_finished(&key, origin, failure, hold),
            Command::Fault { key, reason } => {
                if key.epoch == self.epoch.id {
                    let bus_id = key.bus_id.clone();
                    if !self.slots.is_busy(&key) {
                        tracing::warn!("Attaching {bus_id} failed: {reason}");
                        self.slots.record_fault(&key, reason.clone());
                        self.shared.activity.record(
                            self.shared.device(&bus_id),
                            ActivityReason::Attached,
                            ActivityOutcome::Failed,
                            format!("{} was not attached", self.shared.name_of(&bus_id)),
                            Some(reason),
                        );
                    }
                    self.publish();
                }
            }
            Command::Stop { reply } => {
                self.stop_everything();
                let _ = reply.send(());
            }
        }
    }

    // The link.

    fn on_link_event(&mut self, id: u64, event: LinkEvent) {
        if !self.link.as_ref().is_some_and(|l| l.id == id) {
            return;
        }
        match event {
            LinkEvent::Status(LinkStatus::Connecting) => self.set_link(LinkState::Connecting, None),
            LinkEvent::Status(LinkStatus::Connected) => {
                self.set_link(LinkState::Connected, None);
                self.attach_remembered();
            }
            LinkEvent::Status(LinkStatus::Down(message)) => {
                tracing::warn!("The link to the server failed: {message}");
                self.set_link(LinkState::Error, Some(message));
            }
            LinkEvent::Devices(devices) => {
                self.remote = Arc::new(sanitize(devices));
                self.remote_generation += 1;
                self.listed = true;
                self.publish();
                self.attach_remembered();
            }
        }
    }

    fn set_link(&mut self, link: LinkState, message: Option<String>) {
        self.link_message.clone_from(&message);
        let previous = std::mem::replace(&mut self.link_state, link);
        if previous != link
            && (link == LinkState::Connected
                || (previous == LinkState::Connected
                    && matches!(link, LinkState::Error | LinkState::Connecting)))
        {
            let server = self
                .shared
                .connection
                .current()
                .server_url
                .as_deref()
                .map_or_else(|| "the server".to_owned(), url_host);
            let entity = EntityRef::link(&server);
            if link == LinkState::Connected {
                self.shared.activity.record(
                    entity,
                    ActivityReason::LinkUp,
                    ActivityOutcome::Succeeded,
                    format!("Linked to {server}"),
                    None,
                );
            } else {
                self.shared.activity.record(
                    entity,
                    ActivityReason::LinkDown,
                    ActivityOutcome::Failed,
                    format!("Lost the link to {server}"),
                    message,
                );
            }
        }
        self.publish();
    }

    /// Makes the link the saved connection needs, or none. The same server at the address the
    /// link is at keeps its link, so a certificate it switched to or an address it followed
    /// changes nothing here. A link that is replaced stops in the background: stopping waits for
    /// the link's own handlers, and one of them may be the save that led here.
    fn apply_link(&mut self) {
        let connection = (*self.shared.connection.current()).clone();
        let wanted = self.shared.settings.current().role != Role::Server
            && connection.is_complete()
            && !self.stopping;
        if wanted
            && let Some(current) = self.link.as_mut()
            && continues(&connection, &current.connection)
            && authority(connection.server_url.as_deref().unwrap_or_default())
                == authority(&current.link.address())
        {
            current.connection = connection;
            return;
        }
        let made = wanted.then(|| {
            let id = self.next_link;
            self.next_link += 1;
            let events = LinkEvents {
                client: Arc::downgrade(&self.shared),
                id,
            };
            CurrentLink {
                id,
                link: self.shared.links.connect(&connection, events),
                connection,
            }
        });
        let started = made.as_ref().map(|m| m.link.clone());
        self.replace_link(made);
        // Current before it starts, so the first thing it reports is heard.
        match started {
            Some(link) => {
                self.publish();
                link.start();
            }
            None => {
                self.remote = Arc::new(RemoteDevices::default());
                self.remote_generation += 1;
                self.listed = false;
                self.set_link(LinkState::Unconfigured, None);
            }
        }
    }

    fn replace_link(&mut self, next: Option<CurrentLink>) {
        if let Some(previous) = std::mem::replace(&mut self.link, next) {
            self.shared
                .workers
                .spawn("stop the old link", previous.link.stop());
        }
    }

    /// A new server, new credentials, or a role that is no longer client or both ends every
    /// attachment first, so nothing stays attached to a server the user moved away from, and then
    /// restarts the link. A change that touches neither leaves the link alone, and so does the
    /// same server announcing or switching to its next certificate. The same server at another
    /// address keeps the attachments, and the link too when the link followed the server there
    /// itself; an address the person typed starts the link again there.
    ///
    /// The release runs in the background, because the driver's own command may take 30 seconds
    /// and the settings update, and with it the UI's confirmation, waits for this handler. An
    /// attach that starts meanwhile waits for the release.
    fn on_changed(&mut self) -> Changed {
        let now = self.settled();
        if self.applied == now {
            return Changed::Nothing;
        }
        let was = self.applied.1.clone();
        let same_server = self.applied.0 == now.0 && continues(&now.1, &was);
        self.applied = now;

        if same_server {
            // The attachments carry on, and so does the link unless it is at another address.
            self.apply_link();
            return Changed::SameServer(Box::new(was));
        }

        self.shared.changes_applied.fetch_add(1, Ordering::SeqCst);
        self.end_epoch(true);
        self.clear_remote_if = Some(self.remote_generation);
        self.apply_link();
        self.publish();
        Changed::Released
    }

    /// Ends the connection: cancels the attaches in flight, which check the epoch before and
    /// after they register, and releases every slot. With `renew`, the next attach belongs to a
    /// new epoch and waits for the releases.
    fn end_epoch(&mut self, renew: bool) {
        self.epoch.token.cancel();
        if renew {
            self.epoch = Arc::new(Epoch {
                id: self.epoch.id + 1,
                token: CancellationToken::new(),
            });
            if self.release_done.is_cancelled() {
                self.release_done = CancellationToken::new();
            }
        }
        self.slots.clear_faults();
        self.last_try.clear();
        self.reported.clear();
        for key in self.slots.keys() {
            let plan = self.slots.begin_release(&key);
            if !plan.is_empty() {
                self.epoch_releases += 1;
                self.spawn_release(key, plan, Origin::Epoch);
            }
        }
        self.finish_epoch_releases();
    }

    /// Clears the old server's device list once its releases are done, unless a new list has
    /// arrived since, and lets the attaches that waited go.
    fn finish_epoch_releases(&mut self) {
        if self.epoch_releases > 0 {
            return;
        }
        if let Some(generation) = self.clear_remote_if.take()
            && self.remote_generation == generation
        {
            // The new link may have listed its devices by now; only the old list is dropped.
            self.remote = Arc::new(RemoteDevices::default());
            self.remote_generation += 1;
            self.listed = false;
        }
        self.release_done.cancel();
        self.publish();
    }

    // Attaching.

    /// Attaches in the background. A failed automatic attach is tried again later, and only a new
    /// reason for failing goes to the activity log. Asking again for a device that is attached or
    /// on its way is a no-op.
    fn start_attach(&mut self, bus_id: &BusId, automatic: bool) {
        if self.stopping {
            return;
        }
        let key = self.key_of(bus_id);
        let uses_driver = self.shared.uses_driver(&self.shared_view());
        let mode = if uses_driver {
            Mode::Driver
        } else {
            Mode::Kernel
        };
        let Some((attempt, cancel)) = self.slots.begin_attach(&key, mode, &self.epoch.token) else {
            return;
        };
        self.last_try
            .insert(bus_id.clone(), self.shared.clock.monotonic());
        self.shared.attach_starts.fetch_add(1, Ordering::SeqCst);
        let shared = self.shared.clone();
        let releasing = self.release_done.clone();
        self.shared
            .workers
            .spawn(format!("attach {bus_id}"), async move {
                let outcome = attach::run(&shared, &key, attempt, mode, &cancel, &releasing).await;
                shared.send(Command::AttachFinished {
                    key,
                    attempt,
                    automatic,
                    outcome,
                });
            });
        self.publish();
    }

    fn shared_view(&self) -> View {
        View {
            epoch: self.epoch.clone(),
            link: None,
            listed: self.listed,
            gateway_port: self.gateway_port,
            gateway_fault: self.gateway_fault.clone(),
            remote: self.remote.clone(),
        }
    }

    fn attach_finished(
        &mut self,
        key: &AttachKey,
        attempt: u64,
        automatic: bool,
        outcome: Outcome,
    ) {
        match self.slots.finish_attach(key, attempt, outcome) {
            Finish::Nothing => {
                if self.slots.attachment(key).is_some() {
                    self.reported.remove(key);
                }
            }
            Finish::Failed { reason } => {
                let repeated = automatic && self.reported.get(key) == Some(&reason);
                self.reported.insert(key.clone(), reason.clone());
                let bus_id = &key.bus_id;
                tracing::warn!("Attaching {bus_id} failed: {reason}");
                if !repeated {
                    self.shared.activity.record(
                        self.shared.device(bus_id),
                        ActivityReason::Attached,
                        ActivityOutcome::Failed,
                        format!("{} was not attached", self.shared.name_of(bus_id)),
                        Some(reason),
                    );
                }
            }
            Finish::ReleaseHold(hold) => {
                let origin = if key.epoch == self.epoch.id {
                    Origin::Late
                } else {
                    self.epoch_releases += 1;
                    Origin::Epoch
                };
                let plan = Plan {
                    attachment: None,
                    hold: Some(hold),
                };
                self.spawn_release(key.clone(), plan, origin);
            }
        }
        self.publish();
    }

    /// Attaches the remembered devices that the server offers and nobody is using.
    fn attach_remembered(&mut self) {
        if self.stopping {
            return;
        }
        let remembered = self.shared.remembered.current();
        let fingerprint = self.shared.connection.current().server_fingerprint.clone();
        if self.link_state != LinkState::Connected
            || remembered.server_fingerprint.is_none()
            || remembered.server_fingerprint != fingerprint
        {
            return;
        }
        let now = self.shared.clock.monotonic();
        let every = self.shared.options.retry_interval;
        let candidates: Vec<BusId> = self
            .remote
            .devices
            .iter()
            .filter(|d| d.state == DeviceState::Shared && attached_expects(&remembered, d))
            .filter(|d| {
                !self.detaching.contains(&d.bus_id)
                    && !self.slots.is_busy(&self.key_of(&d.bus_id))
                    && self
                        .last_try
                        .get(&d.bus_id)
                        .is_none_or(|last| now.duration_since(*last) >= every)
            })
            .map(|d| d.bus_id.clone())
            .collect();
        for bus_id in candidates {
            self.start_attach(&bus_id, true);
        }
    }

    // Releasing.

    fn detach(&mut self, bus_id: BusId, reply: oneshot::Sender<()>) {
        let key = self.key_of(&bus_id);
        self.detaching.insert(bus_id);
        self.reported.remove(&key);
        let plan = self.slots.begin_release(&key);
        if plan.is_empty() {
            // An attach still on its way notices this once the driver returns, and releases
            // what it made.
            let _ = reply.send(());
        } else {
            self.waiters.entry(key.clone()).or_default().push(reply);
            self.spawn_release(key, plan, Origin::User);
        }
        self.publish();
    }

    fn spawn_release(&self, key: AttachKey, plan: Plan, origin: Origin) {
        let shared = self.shared.clone();
        self.shared
            .workers
            .spawn(format!("release {}", key.bus_id), async move {
                let (failure, hold) = attach::release(&shared, &key, plan).await;
                shared.send(Command::ReleaseFinished {
                    key,
                    origin,
                    failure,
                    hold,
                });
            });
    }

    fn release_finished(
        &mut self,
        key: &AttachKey,
        origin: Origin,
        failure: Option<String>,
        hold: Option<DriverHold>,
    ) {
        self.slots.release_finished(key, failure, hold);
        if let Some(waiting) = self.waiters.remove(key) {
            for waiter in waiting {
                let _ = waiter.send(());
            }
        }
        if origin == Origin::Epoch {
            self.epoch_releases = self.epoch_releases.saturating_sub(1);
            self.finish_epoch_releases();
        }
        self.publish();
    }

    /// Ends the epoch and the link, and lets go of everything the driver holds, all at once.
    fn stop_everything(&mut self) {
        self.stopping = true;
        self.end_epoch(false);
        self.replace_link(None);
        self.link_state = LinkState::Unconfigured;
        self.publish();
    }

    // What the UI and the workers see.

    fn publish(&self) {
        let key = |d: &crate::DeviceView| self.key_of(&d.bus_id);
        let devices = self
            .remote
            .devices
            .iter()
            .map(|d| {
                let key = key(d);
                let attachment: Option<&Attachment> = self.slots.attachment(&key);
                ClientDeviceView {
                    device: d.clone(),
                    attached_here: attachment.is_some(),
                    port: attachment.and_then(|a| a.port),
                    attached_since: attachment.map(|a| a.since),
                    fault: self.slots.fault(&key).map(str::to_owned),
                    attached_via: attachment.and_then(|a| match &a.kind {
                        Kind::Visitor { from } => Some(from.clone()),
                        _ => None,
                    }),
                }
            })
            .collect();
        let uses_driver = self.shared.uses_driver(&self.shared_view());
        let devices = ClientDevices {
            link: self.link_state,
            link_message: self.link_message.clone(),
            devices,
            unavailable: if uses_driver {
                None
            } else {
                self.gateway_fault
                    .clone()
                    .or_else(|| self.shared.attacher.unavailable())
            },
            gateway: self.gateway_port.is_some() && !uses_driver,
            gateway_fault: self.gateway_fault.clone(),
        };
        self.devices_tx.send_if_modified(|current| {
            if **current == devices {
                false
            } else {
                *current = Arc::new(devices);
                true
            }
        });
        self.view_tx.send_replace(Arc::new(View {
            epoch: self.epoch.clone(),
            link: self.link.as_ref().map(|l| l.link.clone()),
            listed: self.listed,
            gateway_port: self.gateway_port,
            gateway_fault: self.gateway_fault.clone(),
            remote: self.remote.clone(),
        }));
    }
}

fn attached_expects(
    remembered: &koppel_config::files::AttachedDevices,
    device: &crate::DeviceView,
) -> bool {
    remembered
        .bus_ids
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|b| b == device.bus_id.as_str()))
        && remembered
            .vid_pids
            .as_ref()
            .and_then(|v| v.get(device.bus_id.as_str()))
            .is_none_or(|vid_pid| *vid_pid == device.vid_pid)
}

/// What a server sends is only trusted once it is checked: the same bus ID twice is one device.
/// A row with a bus ID that is not one was already left out when the list was read.
fn sanitize(received: RemoteDevices) -> RemoteDevices {
    let mut seen = HashSet::new();
    RemoteDevices {
        devices: received
            .devices
            .into_iter()
            .filter(|d| seen.insert(d.bus_id.clone()))
            .collect(),
    }
}

/// The address the driver is pointed at.
pub(super) const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceView;

    #[test]
    fn the_same_bus_id_twice_is_one_device() {
        let id: BusId = "1-1.4".parse().unwrap();
        let received = RemoteDevices {
            devices: vec![
                DeviceView::new(id.clone()),
                DeviceView {
                    name: "twice".into(),
                    ..DeviceView::new(id)
                },
            ],
        };

        let devices = sanitize(received).devices;

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "");
    }
}

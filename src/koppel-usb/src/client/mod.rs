//! The client role: the link to the saved server, its device list, attaching and detaching, and
//! `attached.json`.
//!
//! One actor task owns every piece of state that changes ([`actor`]): the link, the server's
//! device list, and what each device is doing ([`slots`]). Everything else asks it through a
//! command and reads what it publishes through `watch` channels. Slow work (opening a stream,
//! the driver's command, undoing an attach) runs in tasks of its own and reports back with a
//! command, so the actor never waits on I/O and never holds a lock across an `await`.

mod actor;
mod attach;
mod gateway_session;
mod slots;
mod url;

use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use self::slots::{AttachKey, Attachment, DriverHold, Outcome, RegisterError};
use crate::tasks::{Listener, Tasks, listen, lock};
use crate::{
    ActivitySink, AttachedDevices, BoxFuture, BusId, ClientDevices, Clock, Connection, DeviceView,
    Handler, Observed, Platform, RemoteDevices, Store, StoreError, StreamOpener, Subscription,
    UsbDevice, UsbSettings,
};

/// How long stopping waits for the driver to let go of every device. The driver's own command
/// has a limit of 30 seconds, and the devices are released at the same time, not one by one.
const RELEASE_WAIT: Duration = Duration::from_secs(35);
const STOP_WAIT: Duration = Duration::from_secs(5);

/// Skakel's wording for a connection that dropped.
const CONNECTION_LOST: &str = "The connection to the server was lost.";

/// The client's timings.
#[derive(Debug, Clone, Copy)]
pub struct ClientOptions {
    /// One deadline for everything before USB traffic flows: opening the stream (the token,
    /// connecting, TLS), the import reply and the device record. A server that accepts and then
    /// says nothing ends here. `Usb:AttachTimeoutSeconds`, 20 seconds unless set.
    pub attach_timeout: Duration,
    /// How often a remembered device that is not attached yet is tried again.
    pub retry_interval: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            attach_timeout: Duration::from_secs(20),
            retry_interval: Duration::from_secs(5),
        }
    }
}

/// What a [`UsbClient`] is made from.
pub struct Deps {
    pub settings: Arc<dyn Observed<UsbSettings>>,
    pub connection: Arc<dyn Observed<Connection>>,
    /// `attached.json`.
    pub remembered: Arc<dyn Store<AttachedDevices>>,
    /// What attaches on this machine: this kernel, or a USB/IP driver.
    pub platform: Platform,
    /// Makes the link to the server.
    pub links: Arc<dyn Links>,
    pub activity: Arc<dyn ActivitySink>,
    pub clock: Arc<dyn Clock>,
    pub options: ClientOptions,
}

/// Where the link to the server stands, as the link reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    Connecting,
    Connected,
    /// The link failed or was closed, with the reason in Skakel's fixed wording. A revoke or a
    /// rotation on the server says to pair again.
    Down(String),
}

/// Makes the link to the saved server. The transport implements it (Skakel's client, in koppel-web).
pub trait Links: Send + Sync {
    /// A link to the server in `connection`, not started yet. It reports through `events` once
    /// started, and never before.
    fn connect(&self, connection: &Connection, events: LinkEvents) -> Arc<dyn Link>;
}

/// The link to one saved server: it keeps the server's device list coming and opens a stream for
/// each device. The hub, the attaches and the gateway share it, with its token and its pinned
/// certificate.
pub trait Link: StreamOpener {
    /// Starts linking. Status and device lists come back through the [`LinkEvents`].
    fn start(&self);

    /// The server's address as the link uses it now, such as `https://10.0.0.5:5240`. It changes
    /// when the link follows the server to another address.
    fn address(&self) -> String;

    /// Stops the link. Runs in the background, so it may wait for the link's own handlers.
    fn stop(&self) -> BoxFuture<'static, ()>;
}

/// How a link reports back to the client. What a link reports after it was replaced is ignored.
#[derive(Clone)]
pub struct LinkEvents {
    client: Weak<Shared>,
    id: u64,
}

impl LinkEvents {
    /// The link's state changed. Renewing it before its token runs out is not a change.
    pub async fn status(&self, status: LinkStatus) {
        self.send(LinkEvent::Status(status));
    }

    /// The server sent its device list (Koppel's `DevicesChanged`).
    pub async fn devices(&self, devices: RemoteDevices) {
        self.send(LinkEvent::Devices(devices));
    }

    fn send(&self, event: LinkEvent) {
        if let Some(shared) = self.client.upgrade() {
            shared.send(Command::Link { id: self.id, event });
        }
    }
}

pub(crate) enum LinkEvent {
    Status(LinkStatus),
    Devices(RemoteDevices),
}

/// Why an attach was not started.
#[derive(Debug, thiserror::Error)]
pub enum BeginAttachError {
    /// The bus ID is not a USB bus ID. The endpoint answers 400.
    #[error("That is not a USB bus ID.")]
    InvalidBusId,
    /// `attached.json` could not be written.
    #[error("{0}")]
    Save(StoreError),
}

/// A server's answer to an import is for another device than the one asked for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportMismatch {
    #[error("The server answered with a different device than {0}.")]
    OtherPort(BusId),
    #[error(
        "The device on {bus_id} is not the one that was picked ({picked}); the server sent {sent}."
    )]
    OtherDevice {
        bus_id: BusId,
        picked: String,
        sent: String,
    },
}

/// The server answers an import with a device record. It must describe the device the user
/// picked: a reply for another bus ID, or for another VID:PID than the list showed, is refused.
pub fn check_import(
    device: &UsbDevice,
    bus_id: &BusId,
    picked: Option<&DeviceView>,
) -> Result<(), ImportMismatch> {
    if device.bus_id != *bus_id {
        return Err(ImportMismatch::OtherPort(bus_id.clone()));
    }
    if let Some(picked) = picked
        && device.vid_pid() != picked.vid_pid
    {
        return Err(ImportMismatch::OtherDevice {
            bus_id: bus_id.clone(),
            picked: picked.vid_pid.clone(),
            sent: device.vid_pid(),
        });
    }
    Ok(())
}

/// Why an attach step did not finish. `Display` is the reason the device shows; the source of
/// an I/O failure stays for the log.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AttachError {
    #[error("Not connected to a server.")]
    NotConnected,
    #[error("{}", CONNECTION_LOST)]
    ConnectionLost(#[source] std::io::Error),
    #[error("The server is not offering {0}.")]
    NotOffered(BusId),
    #[error("The server could not export the device. Someone else may be using it.")]
    Refused,
    #[error("The server did not answer within {0} seconds.")]
    Timeout(u64),
    #[error("{}", .0.describe())]
    Open(crate::OpenError),
    #[error(transparent)]
    Mismatch(#[from] ImportMismatch),
    #[error("The server sent a device record that is not valid.")]
    BadRecord,
    #[error("This machine is a server, not a client.")]
    ServerRole,
    #[error("The connection changed while attaching.")]
    ConnectionChanged,
    #[error("{0} is already attached.")]
    AlreadyAttached(BusId),
    #[error("{0}")]
    GatewayDown(String),
    #[error("Could not set up the connection to the kernel: {0}")]
    Loopback(#[source] std::io::Error),
    #[error(transparent)]
    Usb(#[from] crate::UsbError),
}

impl From<std::io::Error> for AttachError {
    fn from(source: std::io::Error) -> Self {
        Self::ConnectionLost(source)
    }
}

/// Why an attach step did not finish: it is not wanted any more, or it went wrong.
pub(crate) enum Failure {
    /// A change, a detach or Koppel stopping.
    Cancelled,
    Error(AttachError),
}

impl<E: Into<AttachError>> From<E> for Failure {
    fn from(error: E) -> Self {
        Self::Error(error.into())
    }
}

impl Failure {
    fn outcome(self) -> Outcome {
        match self {
            Self::Cancelled => Outcome::Cancelled,
            Self::Error(error) => {
                tracing::debug!("An attach step failed: {error:?}");
                Outcome::Failed(error.to_string())
            }
        }
    }
}

/// The connection the client is on: what makes a device's slot belong to one connection.
/// Cancelled when the connection changes or Koppel stops, which ends every attach that began
/// under it.
pub(crate) struct Epoch {
    pub id: u64,
    pub token: CancellationToken,
}

/// What the actor publishes for the tasks that work for it.
pub(crate) struct View {
    pub epoch: Arc<Epoch>,
    pub link: Option<Arc<dyn Link>>,
    /// True once the server sent its device list on this link, so its data plane is worth trying.
    pub listed: bool,
    pub gateway_port: Option<u16>,
    pub gateway_fault: Option<String>,
    pub remote: Arc<RemoteDevices>,
}

pub(crate) enum Command {
    Attach {
        bus_id: BusId,
        automatic: bool,
        /// Told once the request was acted on, so a caller that asks twice can rely on the count.
        reply: Option<oneshot::Sender<()>>,
    },
    Detach {
        bus_id: BusId,
        reply: oneshot::Sender<()>,
    },
    /// `attached.json` no longer lists the device, or it could not be saved.
    Forgotten {
        bus_id: BusId,
        fault: Option<String>,
        reply: oneshot::Sender<()>,
    },
    Link {
        id: u64,
        event: LinkEvent,
    },
    SettingsChanged {
        reply: oneshot::Sender<actor::Changed>,
    },
    Gateway {
        port: Option<u16>,
        fault: Option<String>,
    },
    /// The gateway serves the driver's import of `key`, once.
    Book {
        key: AttachKey,
        reply: oneshot::Sender<Option<u64>>,
    },
    Register {
        key: AttachKey,
        attempt: Option<u64>,
        attachment: Attachment,
        reply: oneshot::Sender<Result<(), RegisterError>>,
    },
    /// A gateway session's relay ended.
    Unregister {
        key: AttachKey,
        attachment: u64,
        reply: oneshot::Sender<bool>,
    },
    /// The relay of this client's own attach ended.
    RelayEnded {
        key: AttachKey,
        attachment: u64,
    },
    AttachFinished {
        key: AttachKey,
        attempt: u64,
        automatic: bool,
        outcome: Outcome,
    },
    ReleaseFinished {
        key: AttachKey,
        origin: actor::Origin,
        failure: Option<String>,
        hold: Option<DriverHold>,
    },
    /// A gateway request failed.
    Fault {
        key: AttachKey,
        reason: String,
    },
    Stop {
        reply: oneshot::Sender<()>,
    },
}

/// What the actor, its workers and the public methods share.
pub(crate) struct Shared {
    settings: Arc<dyn Observed<UsbSettings>>,
    connection: Arc<dyn Observed<Connection>>,
    remembered: Arc<dyn Store<AttachedDevices>>,
    platform: Platform,
    links: Arc<dyn Links>,
    activity: Arc<dyn ActivitySink>,
    clock: Arc<dyn Clock>,
    options: ClientOptions,
    commands: mpsc::UnboundedSender<Command>,
    view: watch::Receiver<Arc<View>>,
    devices: watch::Receiver<Arc<ClientDevices>>,
    /// Attach workers, relays, releases: what stopping waits for.
    workers: Tasks,
    /// The tasks that call the listeners.
    listeners: Tasks,
    /// Ends the actor, once the workers are done.
    actor_stop: CancellationToken,
    attachment_ids: AtomicU64,
    attach_starts: AtomicUsize,
    changes_applied: AtomicUsize,
}

impl Shared {
    fn send(&self, command: Command) {
        // The actor is gone once the client has stopped; what is left to say is moot.
        let _ = self.commands.send(command);
    }

    fn view(&self) -> Arc<View> {
        self.view.borrow().clone()
    }

    fn next_attachment_id(&self) -> u64 {
        self.attachment_ids.fetch_add(1, Ordering::Relaxed)
    }

    /// True when attaching goes through a driver on this machine (usbip-win2 on Windows) by way
    /// of the gateway, rather than through this kernel's virtual host controller.
    fn uses_driver(&self, view: &View) -> bool {
        view.gateway_port.is_some() && self.platform.driver_unavailable().is_none()
    }

    /// The device the server's list offers under this bus ID, as the user saw it when picking.
    fn offered(&self, bus_id: &BusId) -> Option<DeviceView> {
        self.view()
            .remote
            .devices
            .iter()
            .find(|d| d.bus_id == *bus_id)
            .cloned()
    }

    fn name_of(&self, bus_id: &BusId) -> String {
        self.view()
            .remote
            .devices
            .iter()
            .find(|d| d.bus_id == *bus_id)
            .map_or_else(|| bus_id.to_string(), |d| d.name.clone())
    }

    fn device(&self, bus_id: &BusId) -> crate::EntityRef {
        crate::EntityRef::device(bus_id, &self.name_of(bus_id))
    }

    /// Adds the device to `attached.json`, or takes it out.
    async fn remember(&self, bus_id: &BusId, attached: bool) -> Result<(), StoreError> {
        let fingerprint = self.connection.current().server_fingerprint.clone();
        let vid_pid = self.offered(bus_id).map(|d| d.vid_pid);
        self.remembered
            .update(Box::new(|current| {
                let mut next = if current.server_fingerprint != fingerprint {
                    AttachedDevices {
                        server_fingerprint: fingerprint.clone(),
                        ..AttachedDevices::default()
                    }
                } else {
                    current.clone()
                };
                if attached {
                    if !next.bus_ids.iter().any(|b| b == bus_id.as_str()) {
                        next.bus_ids.push(bus_id.to_string());
                    }
                } else {
                    next.bus_ids.retain(|b| b != bus_id.as_str());
                }
                match (&vid_pid, attached) {
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
            .await
    }

    /// The attach deadline around `work`; running out of time is a readable failure.
    async fn under_deadline<T>(
        &self,
        work: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        match tokio::time::timeout(self.options.attach_timeout, work).await {
            Ok(result) => result,
            Err(_) => Err(AttachError::Timeout(self.options.attach_timeout.as_secs()).into()),
        }
    }
}

/// Client role: follows the link to the saved server for its device list, and attaches and
/// detaches devices. An attach opens a stream on the link, imports the device over USB/IP, then
/// hands one end of a loopback pair to the kernel's virtual host controller (Linux) or has
/// usbip-win2's driver import it through the gateway (Windows), and relays.
///
/// The devices the person attached are remembered in `attached.json` and attached again whenever
/// the server offers them and they are not attached here: after a restart, an upgrade, a dropped
/// link, or the device being plugged back in on the server. Detaching forgets the device. Every
/// background step catches what goes wrong, shows it on the device and tries again later. Cheap
/// to clone.
#[derive(Clone)]
pub struct UsbClient {
    shared: Arc<Shared>,
    run: Arc<Mutex<Option<actor::Actor>>>,
    settings_subscription: Arc<Mutex<Option<Subscription>>>,
    connection_subscription: Arc<Mutex<Option<Subscription>>>,
    actor: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl UsbClient {
    /// A client over the settings, the saved connection and `attached.json`. Nothing runs until
    /// [`Self::start`].
    pub fn new(deps: Deps) -> Self {
        let (shared, actor) = actor::build(deps);
        Self {
            shared,
            run: Arc::new(Mutex::new(Some(actor))),
            settings_subscription: Arc::new(Mutex::new(None)),
            connection_subscription: Arc::new(Mutex::new(None)),
            actor: Arc::new(Mutex::new(None)),
        }
    }

    /// What the client's UI shows now.
    pub fn current(&self) -> ClientDevices {
        let mut devices = (**self.shared.devices.borrow()).clone();
        // The kernel module may have been loaded since the last time the actor looked.
        let view = self.shared.view();
        let uses_driver = self.shared.uses_driver(&view);
        devices.unavailable = if uses_driver {
            None
        } else {
            view.gateway_fault
                .clone()
                .or_else(|| self.shared.platform.attach_unavailable())
        };
        devices.gateway = view.gateway_port.is_some() && !uses_driver;
        devices
    }

    /// Why `attached.json` is not being used, or `None`. Koppel then started with nothing
    /// remembered.
    pub fn remembered_problem(&self) -> Option<String> {
        self.shared.remembered.problem()
    }

    /// Calls `listener` with what the UI shows, each time it may have changed. A value a newer
    /// one overtook while the listener was busy is skipped.
    pub fn on_changed<F, Fut>(&self, listener: F) -> Listener
    where
        F: Fn(Arc<ClientDevices>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        listen(
            &self.shared.listeners,
            "the client's devices",
            self.shared.devices.clone(),
            listener,
        )
    }

    pub fn remove_listener(&self, listener: Listener) {
        listener.stop();
    }

    /// How many attaches actually started, for tests that prove a repeated request starts none.
    pub fn attach_starts(&self) -> usize {
        self.shared.attach_starts.load(Ordering::SeqCst)
    }

    /// How many role or connection changes were applied, for tests that prove an unrelated
    /// setting applies none.
    pub fn changes_applied(&self) -> usize {
        self.shared.changes_applied.load(Ordering::SeqCst)
    }

    /// The port the gateway listens on, or `None` while it does not.
    pub fn gateway_port(&self) -> Option<u16> {
        self.shared.view().gateway_port
    }

    /// The link to the saved server, or `None` while there is none.
    pub fn link(&self) -> Option<Arc<dyn Link>> {
        self.shared.view().link.clone()
    }

    /// Called by the gateway with its port when it starts listening, and `None` when it stops.
    /// `fault` says why it is not listening although it should be.
    pub async fn set_gateway(&self, port: Option<u16>, fault: Option<String>) {
        self.shared.send(Command::Gateway { port, fault });
    }

    /// Loads `attached.json`, follows the settings and the saved connection, makes the link, and
    /// tries the remembered devices again every few seconds while it is up.
    pub async fn start(&self) -> Result<(), StoreError> {
        let shared = &self.shared;
        shared.remembered.start().await?;
        let Some(actor) = lock(&self.run).take() else {
            return Ok(());
        };
        *lock(&self.actor) = Some(tokio::spawn(actor.run()));

        fn changed<T>(shared: &Arc<Shared>) -> Handler<T> {
            let weak = Arc::downgrade(shared);
            Box::new(move |_| {
                let weak = weak.clone();
                Box::pin(async move {
                    if let Some(shared) = weak.upgrade() {
                        shared.on_changed().await;
                    }
                })
            })
        }
        let settings = shared.settings.subscribe(changed(shared));
        let connection = shared.connection.subscribe(changed(shared));
        *lock(&self.settings_subscription) = Some(settings);
        *lock(&self.connection_subscription) = Some(connection);
        Ok(())
    }

    /// Stops the link and every relay and, on Windows, detaches every device Koppel attached
    /// through the driver. The driver would otherwise keep trying to attach them to a gateway
    /// port that nobody holds any more, and anything that bound that port would get them. The
    /// devices stay remembered, so they attach again when Koppel starts. The drivers let go of
    /// their devices at the same time, and stopping waits for them up to a limit.
    pub async fn stop(&self) {
        let shared = &self.shared;
        drop(lock(&self.settings_subscription).take());
        drop(lock(&self.connection_subscription).take());
        shared.remembered.stop();

        let (reply, done) = oneshot::channel();
        shared.send(Command::Stop { reply });
        if tokio::time::timeout(STOP_WAIT, done).await.is_err() {
            tracing::warn!("The USB client did not stop in time");
        }
        // Relays end on their own once told to; the driver's releases are what takes time.
        shared.listeners.stop(STOP_WAIT).await;
        shared.workers.wait(RELEASE_WAIT).await;
        shared.actor_stop.cancel();
        let actor = lock(&self.actor).take();
        if let Some(actor) = actor {
            let _ = tokio::time::timeout(STOP_WAIT, actor).await;
        }
    }

    /// Remembers the device, then starts attaching it in the background. The outcome reaches the
    /// UI through the listeners. A device that is already attached or being attached is left alone.
    pub async fn begin_attach(&self, bus_id: &str) -> Result<(), BeginAttachError> {
        let bus_id: BusId = bus_id.parse().map_err(|_| BeginAttachError::InvalidBusId)?;
        self.shared
            .remember(&bus_id, true)
            .await
            .map_err(BeginAttachError::Save)?;
        let (reply, started) = oneshot::channel();
        self.shared.send(Command::Attach {
            bus_id,
            automatic: false,
            reply: Some(reply),
        });
        let _ = started.await;
        Ok(())
    }

    /// Releases the device first and forgets it after, so a failed save of `attached.json` is a
    /// fault on the device and never leaves it attached. Until it is forgotten the retry skips it.
    /// A bus ID that is not one has nothing to release.
    pub async fn detach(&self, bus_id: &str) {
        let Ok(bus_id) = bus_id.parse::<BusId>() else {
            return;
        };
        let shared = &self.shared;
        let (reply, released) = oneshot::channel();
        shared.send(Command::Detach {
            bus_id: bus_id.clone(),
            reply,
        });
        let _ = released.await;
        let fault = match shared.remember(&bus_id, false).await {
            Ok(()) => None,
            Err(e) => {
                tracing::warn!("Detaching {bus_id} failed: {e}");
                Some(format!(
                    "The device was detached, but Koppel could not save that, so it may attach again after a restart: {e}"
                ))
            }
        };
        // Back only once the UI can see the fault.
        let (reply, shown) = oneshot::channel();
        shared.send(Command::Forgotten {
            bus_id,
            fault,
            reply,
        });
        let _ = shown.await;
    }

    /// Serves one plain USB/IP connection from a USB/IP driver on this machine, such as
    /// usbip-win2, either Koppel's own attach or (in a container) the host's `usbip attach`. The
    /// request goes to the server through a stream on the link, as an attach does. An import is
    /// tracked, so the UI shows the device attached here and can detach it.
    pub async fn serve_gateway(
        &self,
        local: TcpStream,
        peer: SocketAddr,
        stopping: &CancellationToken,
    ) {
        self.shared.serve_gateway(local, peer, stopping).await;
    }
}

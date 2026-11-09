//! The client role: the link to the saved server, its device list, attaching and detaching, and
//! `attached.json`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use koppel_config::files::{AttachedDevices, ClientConnection, KoppelSettings, Role};
use koppel_config::json::OrderedMap;
use koppel_config::{ConfigFile, ConfigFileError, Subscription};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::tasks::{BackgroundTasks, Handlers, lock};
use crate::{
    ActivityOutcome, ActivityReason, ActivitySink, BoxFuture, ClientDeviceView, ClientDevices,
    Clock, CloseReason, DeviceStream, DeviceView, EntityRef, Listener, RemoteDevices, StreamOpener,
    UsbAttacher, UsbDevice, UsbIpDriver, bus_id, loopback_pair, protocol, relay,
};

const STOP_WAIT: Duration = Duration::from_secs(5);

/// Skakel's wording for a connection that dropped.
const CONNECTION_LOST: &str = "The connection to the server was lost.";
const NOT_CONNECTED: &str = "Not connected to a server.";

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

/// Where the link to the server stands, as the link reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    Connecting,
    Connected,
    /// The link failed or was closed, with the reason in Skakel's fixed wording. A revoke or a
    /// rotation on the server says to pair again.
    Down(String),
}

/// Makes the link to the saved server. The transport implements it (Skakel's client, #85).
pub trait Links: Send + Sync {
    /// A link to the server in `connection`, not started yet. It reports through `events` once
    /// started, and never before.
    fn connect(&self, connection: &ClientConnection, events: LinkEvents) -> Arc<dyn Link>;
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
    client: Weak<Inner>,
    id: u64,
}

impl LinkEvents {
    /// The link's state changed. Renewing it before its token runs out is not a change.
    pub async fn status(&self, status: LinkStatus) {
        if let Some(inner) = self.client.upgrade() {
            inner.on_link_status(self.id, status).await;
        }
    }

    /// The server sent its device list (Koppel's `DevicesChanged`).
    pub async fn devices(&self, devices: RemoteDevices) {
        if let Some(inner) = self.client.upgrade() {
            inner.on_devices(self.id, devices).await;
        }
    }
}

/// Why an attach was not started.
#[derive(Debug)]
pub enum BeginAttachError {
    /// The bus ID is not a USB bus ID. The endpoint answers 400.
    InvalidBusId,
    /// `attached.json` could not be written.
    Save(ConfigFileError),
}

impl fmt::Display for BeginAttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBusId => f.write_str("That is not a USB bus ID."),
            Self::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BeginAttachError {}

/// The server answers an import with a device record. It must describe the device the user
/// picked: a reply for another bus ID, or for another VID:PID than the list showed, is refused.
pub fn check_import(
    device: &UsbDevice,
    bus_id: &str,
    picked: Option<&DeviceView>,
) -> Result<(), String> {
    if device.bus_id != bus_id {
        return Err(format!(
            "The server answered with a different device than {bus_id}."
        ));
    }
    if let Some(picked) = picked
        && device.vid_pid() != picked.vid_pid
    {
        return Err(format!(
            "The device on {bus_id} is not the one that was picked ({}); the server sent {}.",
            picked.vid_pid,
            device.vid_pid()
        ));
    }
    Ok(())
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
    inner: Arc<Inner>,
}

struct Inner {
    settings: ConfigFile<KoppelSettings>,
    connection: ConfigFile<ClientConnection>,
    remembered: ConfigFile<AttachedDevices>,
    attacher: Arc<dyn UsbAttacher>,
    driver: Arc<dyn UsbIpDriver>,
    links: Arc<dyn Links>,
    activity: Arc<dyn ActivitySink>,
    clock: Arc<dyn Clock>,
    options: ClientOptions,
    state: Mutex<State>,
    tasks: BackgroundTasks,
    listeners: Handlers<ClientDevices>,
    stopping: CancellationToken,
    settings_subscription: Mutex<Option<Subscription>>,
    connection_subscription: Mutex<Option<Subscription>>,
    attach_starts: AtomicUsize,
    changes_applied: AtomicUsize,
    next_id: AtomicU64,
}

/// A device on a server. A bus ID is only a port on one server, so attachments are told apart by
/// the server as well: the same bus ID on another server is another device. The server is named
/// by the client ID it issued, which stays the same when the server switches to a new certificate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AttachKey {
    server: String,
    bus_id: String,
}

/// One attachment. Held in an `Arc`, so removing "this attach's own entry" means this very one.
struct Attachment {
    /// The virtual port, or the driver's port for a driver attach; `None` for another gateway attach.
    port: Mutex<Option<u32>>,
    stop: CancellationToken,
    since: OffsetDateTime,
    /// Where another gateway attach came from.
    via: Option<String>,
    /// This client's own attach, made through the driver and the gateway.
    driver: bool,
}

/// What the driver holds for one device: the gateway port it was pointed at and its own port.
#[derive(Debug, Clone, Copy)]
struct DriverAttachment {
    gateway_port: u16,
    driver_port: u32,
}

/// Koppel's request to the driver to import one device, which the gateway accepts exactly once.
#[derive(Default)]
struct PendingImport {
    claimed: AtomicBool,
}

impl PendingImport {
    fn try_claim(&self) -> bool {
        !self.claimed.swap(true, Ordering::SeqCst)
    }
}

struct CurrentLink {
    id: u64,
    link: Arc<dyn Link>,
    /// The connection it was made for, which moves on as the link learns.
    connection: ClientConnection,
}

struct State {
    attachments: HashMap<AttachKey, Arc<Attachment>>,
    attaching: HashSet<AttachKey>,
    pending: HashMap<AttachKey, Arc<PendingImport>>,
    in_flight: HashMap<AttachKey, (u64, CancellationToken)>,
    detaching: HashSet<String>,
    driver_ports: HashMap<AttachKey, DriverAttachment>,
    last_try: HashMap<String, OffsetDateTime>,
    faults: HashMap<String, String>,
    remote: Arc<RemoteDevices>,
    remote_generation: u64,
    /// True once the server sent its device list on this link, so its data plane is worth trying.
    listed: bool,
    link_state: &'static str,
    link_message: Option<String>,
    /// Cancelled when a change ends the attaches in flight.
    epoch: CancellationToken,
    link: Option<CurrentLink>,
    applied: (bool, ClientConnection),
    /// Cancelled once the release a change started is done. The next attach waits for it.
    releasing: CancellationToken,
    gateway_port: Option<u16>,
    gateway_fault: Option<String>,
}

/// Why an attach step did not finish.
enum Failure {
    /// The attach is not wanted any more: a change or Koppel stopping.
    Cancelled,
    /// A reason a person can read.
    Error(String),
}

impl From<String> for Failure {
    fn from(reason: String) -> Self {
        Self::Error(reason)
    }
}

impl From<std::io::Error> for Failure {
    fn from(_: std::io::Error) -> Self {
        Self::Error(CONNECTION_LOST.to_owned())
    }
}

/// The stream to the server for one imported device, and the device record it sent.
struct RemoteImport {
    session: Box<dyn DeviceStream>,
    record: [u8; protocol::DEVICE_SIZE],
}

fn released() -> CancellationToken {
    let done = CancellationToken::new();
    done.cancel();
    done
}

impl UsbClient {
    /// A client over the settings, the saved connection and `attached.json` (opened with
    /// [`koppel_config::open::attached`]). `attacher` attaches through this kernel and `driver`
    /// through a USB/IP driver on this machine; `links` makes the link to the server. Nothing
    /// runs until [`Self::start`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: ConfigFile<KoppelSettings>,
        connection: ConfigFile<ClientConnection>,
        remembered: ConfigFile<AttachedDevices>,
        attacher: Arc<dyn UsbAttacher>,
        driver: Arc<dyn UsbIpDriver>,
        links: Arc<dyn Links>,
        activity: Arc<dyn ActivitySink>,
        clock: Arc<dyn Clock>,
        options: ClientOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                settings,
                connection,
                remembered,
                attacher,
                driver,
                links,
                activity,
                clock,
                options,
                state: Mutex::new(State {
                    attachments: HashMap::new(),
                    attaching: HashSet::new(),
                    pending: HashMap::new(),
                    in_flight: HashMap::new(),
                    detaching: HashSet::new(),
                    driver_ports: HashMap::new(),
                    last_try: HashMap::new(),
                    faults: HashMap::new(),
                    remote: Arc::new(RemoteDevices::default()),
                    remote_generation: 0,
                    listed: false,
                    link_state: "unconfigured",
                    link_message: None,
                    epoch: CancellationToken::new(),
                    link: None,
                    applied: (false, ClientConnection::default()),
                    releasing: released(),
                    gateway_port: None,
                    gateway_fault: None,
                }),
                tasks: BackgroundTasks::default(),
                listeners: Handlers::new("the client's devices"),
                stopping: CancellationToken::new(),
                settings_subscription: Mutex::new(None),
                connection_subscription: Mutex::new(None),
                attach_starts: AtomicUsize::new(0),
                changes_applied: AtomicUsize::new(0),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// What the client's UI shows now.
    pub fn current(&self) -> ClientDevices {
        self.inner.current()
    }

    /// Why `attached.json` is not being used, or `None`. Koppel then started with nothing
    /// remembered.
    pub fn remembered_problem(&self) -> Option<String> {
        self.inner.remembered.problem()
    }

    /// Calls `listener` with what the UI shows, each time it may have changed.
    pub fn on_changed<F, Fut>(&self, listener: F) -> Listener
    where
        F: Fn(Arc<ClientDevices>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.inner.listeners.add(listener)
    }

    pub fn remove_listener(&self, listener: Listener) {
        self.inner.listeners.remove(listener);
    }

    /// How many attaches actually started, for tests that prove a repeated request starts none.
    pub fn attach_starts(&self) -> usize {
        self.inner.attach_starts.load(Ordering::SeqCst)
    }

    /// How many role or connection changes were applied, for tests that prove an unrelated
    /// setting applies none.
    pub fn changes_applied(&self) -> usize {
        self.inner.changes_applied.load(Ordering::SeqCst)
    }

    /// The port the gateway listens on, or `None` while it does not.
    pub fn gateway_port(&self) -> Option<u16> {
        lock(&self.inner.state).gateway_port
    }

    /// The link to the saved server, or `None` while there is none.
    pub fn link(&self) -> Option<Arc<dyn Link>> {
        lock(&self.inner.state)
            .link
            .as_ref()
            .map(|l| l.link.clone())
    }

    /// Called by the gateway with its port when it starts listening, and `None` when it stops.
    /// `fault` says why it is not listening although it should be.
    pub async fn set_gateway(&self, port: Option<u16>, fault: Option<String>) {
        {
            let mut state = lock(&self.inner.state);
            state.gateway_port = port;
            state.gateway_fault = fault;
        }
        self.inner.raise().await;
    }

    /// Loads `attached.json`, follows the settings and the saved connection, makes the link, and
    /// tries the remembered devices again every few seconds while it is up.
    pub async fn start(&self) -> Result<(), ConfigFileError> {
        let inner = &self.inner;
        inner.remembered.start().await?;
        lock(&inner.state).applied = inner.settled();

        let changed = |inner: &Arc<Inner>| {
            let weak = Arc::downgrade(inner);
            move || {
                let weak = weak.clone();
                async move {
                    if let Some(inner) = weak.upgrade() {
                        inner.on_changed().await;
                    }
                }
            }
        };
        let on_settings = changed(inner);
        let on_connection = changed(inner);
        let settings = inner.settings.subscribe(move |_| on_settings());
        let connection = inner.connection.subscribe(move |_| on_connection());
        *lock(&inner.settings_subscription) = Some(settings);
        *lock(&inner.connection_subscription) = Some(connection);

        inner.apply_link().await;

        let weak = Arc::downgrade(inner);
        let stopping = inner.stopping.clone();
        let every = inner.options.retry_interval;
        inner.tasks.spawn("attach remembered devices", async move {
            // A remembered device that could not be attached yet, because it was in use or the
            // server was still starting, is tried again while the link is up.
            let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
            timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = timer.tick() => {}
                    _ = stopping.cancelled() => return,
                }
                match weak.upgrade() {
                    Some(inner) => inner.attach_remembered(),
                    None => return,
                }
            }
        });
        Ok(())
    }

    /// Stops the link and every relay and, on Windows, detaches every device Koppel attached
    /// through the driver. The driver would otherwise keep trying to attach them to a gateway
    /// port that nobody holds any more, and anything that bound that port would get them. The
    /// devices stay remembered, so they attach again when Koppel starts.
    pub async fn stop(&self) {
        let inner = &self.inner;
        if let Some(subscription) = lock(&inner.settings_subscription).take() {
            inner.settings.unsubscribe(subscription);
        }
        if let Some(subscription) = lock(&inner.connection_subscription).take() {
            inner.connection.unsubscribe(subscription);
        }
        inner.remembered.stop();
        inner.end_epoch();
        inner.stopping.cancel();
        inner.replace_link(None);

        // Relays end on their own once told to, so stopping does not wait for them.
        let attachments: Vec<Arc<Attachment>> =
            lock(&inner.state).attachments.values().cloned().collect();
        for attachment in attachments {
            attachment.stop.cancel();
        }
        let held: Vec<AttachKey> = lock(&inner.state).driver_ports.keys().cloned().collect();
        for key in held {
            if let Err(e) = inner.release_driver(&key).await {
                tracing::warn!("Detaching {} failed: {e}", key.bus_id);
            }
        }
        inner.tasks.wait(STOP_WAIT).await;
    }

    /// Remembers the device, then starts attaching it in the background. The outcome reaches the
    /// UI through the listeners. A device that is already attached or being attached is left alone.
    pub async fn begin_attach(&self, bus_id: &str) -> Result<(), BeginAttachError> {
        if !bus_id::is_valid(bus_id) {
            return Err(BeginAttachError::InvalidBusId);
        }
        self.inner
            .remember(bus_id, true)
            .await
            .map_err(BeginAttachError::Save)?;
        lock(&self.inner.state).faults.remove(bus_id);
        self.inner.start_attach(bus_id, false);
        Ok(())
    }

    /// Releases the device first and forgets it after, so a failed save of `attached.json` is a
    /// fault on the device and never leaves it attached. Until it is forgotten the retry skips it.
    pub async fn detach(&self, bus_id: &str) {
        let inner = &self.inner;
        let key = inner.key_of(bus_id);
        let (attachment, held) = {
            let mut state = lock(&inner.state);
            state.detaching.insert(bus_id.to_owned());
            state.faults.remove(bus_id);
            // An attach still on its way notices this once the driver returns, and releases what it made.
            if let Some((_, attach)) = state.in_flight.get(&key) {
                attach.cancel();
            }
            let attachment = state.attachments.remove(&key);
            let held = state.driver_ports.contains_key(&key);
            (attachment, held)
        };
        if attachment.is_some() || held {
            inner.release(&key, attachment).await;
        }
        if let Err(e) = inner.remember(bus_id, false).await {
            lock(&inner.state).faults.insert(
                bus_id.to_owned(),
                format!("The device was detached, but Koppel could not save that, so it may attach again after a restart: {e}"),
            );
            tracing::warn!("Detaching {bus_id} failed: {e}");
        }
        lock(&inner.state).detaching.remove(bus_id);
        inner.raise().await;
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
        self.inner.serve_gateway(local, peer, stopping).await;
    }
}

/// The authority of a URL such as `https://Host:5240/x`, in lower case and without the scheme's
/// default port, so two spellings of one address compare equal.
fn authority(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, a)| a)
        .to_ascii_lowercase();
    let default = match scheme.to_ascii_lowercase().as_str() {
        "http" => ":80",
        _ => ":443",
    };
    authority
        .strip_suffix(default)
        .map(str::to_owned)
        .unwrap_or(authority)
}

/// The host part of a URL, as the activity log names a server.
fn url_host(url: &str) -> String {
    let authority = authority(url);
    if authority.starts_with('[') {
        return authority
            .split_once(']')
            .map_or(authority.clone(), |(host, _)| format!("{host}]"));
    }
    authority.split(':').next().unwrap_or_default().to_owned()
}

/// A SHA-256 fingerprint compared on its hex digits only, in any case.
fn same_fingerprint(a: Option<&str>, b: Option<&str>) -> bool {
    let digits = |f: Option<&str>| -> String {
        f.unwrap_or_default()
            .chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    digits(a) == digits(b)
}

/// The same server and credentials as `previous`: the same client ID and secret, and a
/// certificate that is either the one pinned before or the one the server said it would switch to.
fn continues(now: &ClientConnection, previous: &ClientConnection) -> bool {
    now.client_id == previous.client_id
        && now.client_secret == previous.client_secret
        && now.server_fingerprint.is_some()
        && (same_fingerprint(
            now.server_fingerprint.as_deref(),
            previous.server_fingerprint.as_deref(),
        ) || same_fingerprint(
            now.server_fingerprint.as_deref(),
            previous.next_server_fingerprint.as_deref(),
        ))
}

fn attached_expects(remembered: &AttachedDevices, device: &DeviceView) -> bool {
    remembered
        .bus_ids
        .as_ref()
        .is_some_and(|ids| ids.contains(&device.bus_id))
        && remembered
            .vid_pids
            .as_ref()
            .and_then(|v| v.get(&device.bus_id))
            .is_none_or(|vid_pid| *vid_pid == device.vid_pid)
}

impl Inner {
    fn key_of(&self, bus_id: &str) -> AttachKey {
        AttachKey {
            server: self
                .connection
                .current()
                .client_id
                .clone()
                .unwrap_or_default(),
            bus_id: bus_id.to_owned(),
        }
    }

    fn settled(&self) -> (bool, ClientConnection) {
        (
            self.settings.current().role != Role::Server,
            (*self.connection.current()).clone(),
        )
    }

    /// True when attaching goes through a driver on this machine (usbip-win2 on Windows) by way
    /// of the gateway, rather than through this kernel's virtual host controller.
    fn uses_driver(&self, state: &State) -> bool {
        state.gateway_port.is_some() && self.driver.unavailable().is_none()
    }

    fn current(&self) -> ClientDevices {
        let server = self
            .connection
            .current()
            .client_id
            .clone()
            .unwrap_or_default();
        let state = lock(&self.state);
        let uses_driver = self.uses_driver(&state);
        let devices = state
            .remote
            .devices
            .iter()
            .map(|d| {
                let key = AttachKey {
                    server: server.clone(),
                    bus_id: d.bus_id.clone(),
                };
                let attachment = state.attachments.get(&key);
                ClientDeviceView {
                    device: d.clone(),
                    attached_here: attachment.is_some(),
                    port: attachment.and_then(|a| *lock(&a.port)),
                    attached_since: attachment.map(|a| a.since),
                    fault: state.faults.get(&d.bus_id).cloned(),
                    attached_via: attachment.and_then(|a| a.via.clone()),
                }
            })
            .collect();
        ClientDevices {
            link: state.link_state.to_owned(),
            link_message: state.link_message.clone(),
            devices,
            unavailable: if uses_driver {
                None
            } else {
                state
                    .gateway_fault
                    .clone()
                    .or_else(|| self.attacher.unavailable())
            },
            gateway: state.gateway_port.is_some() && !uses_driver,
            gateway_fault: state.gateway_fault.clone(),
        }
    }

    async fn raise(&self) {
        self.listeners.raise(Arc::new(self.current())).await;
    }

    fn name_of(&self, bus_id: &str) -> String {
        lock(&self.state)
            .remote
            .devices
            .iter()
            .find(|d| d.bus_id == bus_id)
            .map_or_else(|| bus_id.to_owned(), |d| d.name.clone())
    }

    fn device(&self, bus_id: &str) -> EntityRef {
        EntityRef::device(bus_id, &self.name_of(bus_id))
    }

    /// The device the server's list offers under this bus ID, as the user saw it when picking.
    fn offered(&self, bus_id: &str) -> Option<DeviceView> {
        lock(&self.state)
            .remote
            .devices
            .iter()
            .find(|d| d.bus_id == bus_id)
            .cloned()
    }

    /// Takes the attachment out only when it is still this very one.
    fn remove_if_same(&self, key: &AttachKey, attachment: &Arc<Attachment>) -> bool {
        let mut state = lock(&self.state);
        if state
            .attachments
            .get(key)
            .is_some_and(|a| Arc::ptr_eq(a, attachment))
        {
            state.attachments.remove(key);
            true
        } else {
            false
        }
    }

    /// Undoes an attach: the driver's or the kernel's port, then the relay. A driver attach is
    /// undone by the driver, which would otherwise attach it again. Another gateway attach has no
    /// port here; stopping its relay unplugs it on the other side.
    async fn release(&self, key: &AttachKey, attachment: Option<Arc<Attachment>>) {
        let undone = async {
            let mut detached = self.release_driver(key).await?;
            let kernel_port = attachment
                .as_ref()
                .filter(|a| a.via.is_none() && !a.driver)
                .and_then(|a| *lock(&a.port));
            if !detached && let Some(port) = kernel_port {
                self.attacher.detach(port)?;
                detached = true;
            }
            if detached {
                self.activity.record(
                    self.device(&key.bus_id),
                    ActivityReason::Detached,
                    ActivityOutcome::Succeeded,
                    format!("{} was detached", self.name_of(&key.bus_id)),
                    None,
                );
            }
            Ok::<_, crate::UsbError>(())
        };
        if let Err(e) = undone.await {
            lock(&self.state)
                .faults
                .insert(key.bus_id.clone(), e.message().to_owned());
            tracing::warn!("Detaching {} failed: {e}", key.bus_id);
        }
        if let Some(attachment) = attachment {
            attachment.stop.cancel();
        }
    }

    /// Stops the driver's attempts to attach the device and detaches its port. False when the
    /// driver holds none.
    async fn release_driver(&self, key: &AttachKey) -> Result<bool, crate::UsbError> {
        // Taking the entry first means a detach and an attach that finds itself unwanted never
        // both release it.
        let Some(held) = lock(&self.state).driver_ports.remove(key) else {
            return Ok(false);
        };
        let loopback = Ipv4Addr::LOCALHOST.to_string();
        if let Err(e) = self
            .driver
            .detach(&loopback, held.gateway_port, &key.bus_id, held.driver_port)
            .await
        {
            // Still attached, so the next release tries again.
            lock(&self.state)
                .driver_ports
                .entry(key.clone())
                .or_insert(held);
            return Err(e);
        }
        Ok(true)
    }

    /// Detaches and stops every attachment, without forgetting any of them.
    async fn release_all(&self) {
        let attachments: Vec<(AttachKey, Arc<Attachment>)> = lock(&self.state)
            .attachments
            .iter()
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect();
        for (key, attachment) in attachments {
            if self.remove_if_same(&key, &attachment) {
                self.release(&key, Some(attachment)).await;
            }
        }
        let held: Vec<AttachKey> = lock(&self.state).driver_ports.keys().cloned().collect();
        for key in held {
            self.release(&key, None).await;
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
    async fn on_changed(self: &Arc<Self>) {
        let now = self.settled();
        let (was, release) = {
            let mut state = lock(&self.state);
            if state.applied == now {
                return;
            }
            let was = state.applied.1.clone();
            let same_server = state.applied.0 == now.0 && continues(&now.1, &was);
            state.applied = now;
            let release = (!same_server).then(|| {
                let previous = std::mem::replace(&mut state.releasing, CancellationToken::new());
                (previous, state.releasing.clone())
            });
            (was, release)
        };

        let Some((previous, done)) = release else {
            // The same server: the attachments carry on, and so does the link unless it is at
            // another address.
            self.apply_link().await;
            self.carry_over_remembered(&was).await;
            return;
        };

        self.changes_applied.fetch_add(1, Ordering::SeqCst);
        self.end_epoch();
        let before = {
            let mut state = lock(&self.state);
            state.faults.clear();
            state.last_try.clear();
            state.remote_generation
        };
        let inner = self.clone();
        self.tasks.spawn("release after a change", async move {
            previous.cancelled().await;
            inner.release_all().await;
            {
                // The new link may have listed its devices by now; only the old list is dropped.
                let mut state = lock(&inner.state);
                if state.remote_generation == before {
                    state.remote = Arc::new(RemoteDevices::default());
                    state.remote_generation += 1;
                    state.listed = false;
                }
            }
            done.cancel();
            inner.raise().await;
        });

        self.apply_link().await;
        self.raise().await;
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

    /// Cancels the attaches in flight. They check this before and after they register.
    fn end_epoch(&self) {
        let ended = std::mem::replace(&mut lock(&self.state).epoch, CancellationToken::new());
        ended.cancel();
    }

    /// Makes the link the saved connection needs, or none. The same server at the address the
    /// link is at keeps its link, so a certificate it switched to or an address it followed
    /// changes nothing here. A link that is replaced stops in the background: stopping waits for
    /// the link's own handlers, and one of them may be the save that led here.
    async fn apply_link(self: &Arc<Self>) {
        let connection = (*self.connection.current()).clone();
        let wanted = self.settings.current().role != Role::Server
            && connection.is_complete()
            && !self.stopping.is_cancelled();
        let made = {
            let mut state = lock(&self.state);
            if wanted
                && let Some(current) = state.link.as_mut()
                && continues(&connection, &current.connection)
                && authority(connection.server_url.as_deref().unwrap_or_default())
                    == authority(&current.link.address())
            {
                current.connection = connection;
                return;
            }
            let made = wanted.then(|| {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let events = LinkEvents {
                    client: Arc::downgrade(self),
                    id,
                };
                CurrentLink {
                    id,
                    link: self.links.connect(&connection, events),
                    connection,
                }
            });
            let started = made.as_ref().map(|m| m.link.clone());
            self.replace_link_locked(&mut state, made);
            started
        };
        // Current before it starts, so the first thing it reports is heard.
        match made {
            Some(link) => link.start(),
            None => {
                {
                    let mut state = lock(&self.state);
                    state.remote = Arc::new(RemoteDevices::default());
                    state.remote_generation += 1;
                    state.listed = false;
                }
                self.set_link("unconfigured", None).await;
            }
        }
    }

    fn replace_link(&self, next: Option<CurrentLink>) {
        let mut state = lock(&self.state);
        self.replace_link_locked(&mut state, next);
    }

    fn replace_link_locked(&self, state: &mut State, next: Option<CurrentLink>) {
        if let Some(previous) = std::mem::replace(&mut state.link, next) {
            self.tasks.spawn("stop the old link", previous.link.stop());
        }
    }

    fn is_current(&self, id: u64) -> bool {
        lock(&self.state).link.as_ref().is_some_and(|l| l.id == id)
    }

    /// Where the link stands, as the UI shows it.
    async fn on_link_status(self: &Arc<Self>, id: u64, status: LinkStatus) {
        if !self.is_current(id) {
            return;
        }
        match status {
            LinkStatus::Connecting => self.set_link("connecting", None).await,
            LinkStatus::Connected => {
                self.set_link("connected", None).await;
                self.attach_remembered();
            }
            LinkStatus::Down(message) => {
                tracing::warn!("The link to the server failed: {message}");
                self.set_link("error", Some(message)).await;
            }
        }
    }

    async fn on_devices(self: &Arc<Self>, id: u64, devices: RemoteDevices) {
        if !self.is_current(id) {
            return;
        }
        let devices = sanitize(devices);
        {
            let mut state = lock(&self.state);
            state.remote = Arc::new(devices);
            state.remote_generation += 1;
            state.listed = true;
        }
        self.raise().await;
        self.attach_remembered();
    }

    async fn set_link(&self, link: &'static str, message: Option<String>) {
        let previous = {
            let mut state = lock(&self.state);
            state.link_message = message.clone();
            std::mem::replace(&mut state.link_state, link)
        };
        if previous != link
            && (link == "connected"
                || (previous == "connected" && (link == "error" || link == "connecting")))
        {
            let server = self
                .connection
                .current()
                .server_url
                .as_deref()
                .map_or_else(|| "the server".to_owned(), url_host);
            let entity = EntityRef::link(&server);
            if link == "connected" {
                self.activity.record(
                    entity,
                    ActivityReason::LinkUp,
                    ActivityOutcome::Succeeded,
                    format!("Linked to {server}"),
                    None,
                );
            } else {
                self.activity.record(
                    entity,
                    ActivityReason::LinkDown,
                    ActivityOutcome::Failed,
                    format!("Lost the link to {server}"),
                    message,
                );
            }
        }
        self.raise().await;
    }

    /// Attaches in the background. A failed automatic attach is tried again later, and only a new
    /// reason for failing goes to the activity log. Asking again for a device that is attached or
    /// on its way is a no-op.
    fn start_attach(self: &Arc<Self>, bus_id: &str, automatic: bool) {
        let key = self.key_of(bus_id);
        let (epoch, releasing) = {
            let mut state = lock(&self.state);
            if state.attachments.contains_key(&key) || !state.attaching.insert(key.clone()) {
                return;
            }
            state.last_try.insert(bus_id.to_owned(), self.clock.now());
            (state.epoch.clone(), state.releasing.clone())
        };
        self.attach_starts.fetch_add(1, Ordering::SeqCst);

        let inner = self.clone();
        let bus_id = bus_id.to_owned();
        self.tasks.spawn(format!("attach {bus_id}"), async move {
            // The epoch ends on a change and when Koppel stops; a detach cancels this one alone.
            let cancel = epoch.child_token();
            let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
            lock(&inner.state)
                .in_flight
                .insert(key.clone(), (id, cancel.clone()));

            let attach = async {
                // A change that came first is still releasing what it ended; do not attach under its feet.
                tokio::select! {
                    _ = releasing.cancelled() => {}
                    _ = cancel.cancelled() => return Err(Failure::Cancelled),
                }
                let uses_driver = inner.uses_driver(&lock(&inner.state));
                if uses_driver {
                    inner.attach_through_driver(&key, &cancel).await
                } else {
                    inner.attach(&key, &cancel).await
                }
            };
            if let Err(Failure::Error(reason)) = attach.await {
                inner.report_failure(Some(&bus_id), reason, automatic).await;
            }

            let mut state = lock(&inner.state);
            if state.in_flight.get(&key).is_some_and(|(i, _)| *i == id) {
                state.in_flight.remove(&key);
            }
            state.attaching.remove(&key);
        });
    }

    /// What goes wrong with a server, whichever path found out: a readable reason in the log,
    /// and for a device in its fault and the activity log.
    async fn report_failure(&self, bus_id: Option<&str>, reason: String, automatic: bool) {
        let Some(bus_id) = bus_id else {
            tracing::warn!("Gateway request for the device list failed: {reason}");
            return;
        };
        // The driver may have attached it by itself in the meantime, which is what was wanted.
        let key = self.key_of(bus_id);
        let repeated = {
            let mut state = lock(&self.state);
            if state.attachments.contains_key(&key) {
                return;
            }
            let repeated = automatic && state.faults.get(bus_id) == Some(&reason);
            state.faults.insert(bus_id.to_owned(), reason.clone());
            repeated
        };
        tracing::warn!("Attaching {bus_id} failed: {reason}");
        if !repeated {
            self.activity.record(
                self.device(bus_id),
                ActivityReason::Attached,
                ActivityOutcome::Failed,
                format!("{} was not attached", self.name_of(bus_id)),
                Some(reason),
            );
        }
        self.raise().await;
    }

    /// Attaches the remembered devices that the server offers and nobody is using.
    fn attach_remembered(self: &Arc<Self>) {
        let remembered = self.remembered.current();
        let fingerprint = self.connection.current().server_fingerprint.clone();
        let server = self
            .connection
            .current()
            .client_id
            .clone()
            .unwrap_or_default();
        let candidates: Vec<String> = {
            let state = lock(&self.state);
            if state.link_state != "connected"
                || remembered.server_fingerprint.is_none()
                || remembered.server_fingerprint != fingerprint
            {
                return;
            }
            let now = self.clock.now();
            state
                .remote
                .devices
                .iter()
                .filter(|d| d.state == "shared" && attached_expects(&remembered, d))
                .filter(|d| {
                    let key = AttachKey {
                        server: server.clone(),
                        bus_id: d.bus_id.clone(),
                    };
                    !state.detaching.contains(&d.bus_id)
                        && !state.attachments.contains_key(&key)
                        && state
                            .last_try
                            .get(&d.bus_id)
                            .is_none_or(|last| now - *last >= self.options.retry_interval)
                })
                .map(|d| d.bus_id.clone())
                .collect()
        };
        for bus_id in candidates {
            self.start_attach(&bus_id, true);
        }
    }

    /// Adds the device to `attached.json`, or takes it out.
    async fn remember(&self, bus_id: &str, attached: bool) -> Result<(), ConfigFileError> {
        let fingerprint = self.connection.current().server_fingerprint.clone();
        let vid_pid = self.offered(bus_id).map(|d| d.vid_pid);
        self.remembered
            .update(|current| {
                let mut next = if current.server_fingerprint != fingerprint {
                    AttachedDevices {
                        server_fingerprint: fingerprint.clone(),
                        ..AttachedDevices::empty()
                    }
                } else {
                    current.clone()
                };
                let mut bus_ids = next.bus_ids.take().unwrap_or_default();
                if attached {
                    if !bus_ids.iter().any(|b| b == bus_id) {
                        bus_ids.push(bus_id.to_owned());
                    }
                } else {
                    bus_ids.retain(|b| b != bus_id);
                }
                next.bus_ids = Some(bus_ids);
                next.vid_pids = match (&vid_pid, attached) {
                    (Some(vid_pid), true) => {
                        let mut map = next.vid_pids.take().unwrap_or_else(OrderedMap::new);
                        map.insert(bus_id, vid_pid.clone());
                        Some(map)
                    }
                    _ => next.vid_pids.take().map(|mut map| {
                        map.remove(bus_id);
                        map
                    }),
                };
                next
            })
            .await
            .map(|_| ())
    }

    /// Runs `work` under the attach deadline; running out of time is a readable failure.
    async fn under_deadline<T>(
        &self,
        work: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        match tokio::time::timeout(self.options.attach_timeout, work).await {
            Ok(result) => result,
            Err(_) => Err(Failure::Error(format!(
                "The server did not answer within {} seconds.",
                self.options.attach_timeout.as_secs()
            ))),
        }
    }

    /// Opens a stream to the server on the link. USB/IP follows inside it. The caller sets the
    /// deadline.
    async fn open_session(&self) -> Result<Box<dyn DeviceStream>, Failure> {
        let link = {
            let state = lock(&self.state);
            match (&state.link, state.listed) {
                (Some(link), true) => link.link.clone(),
                _ => return Err(Failure::Error(NOT_CONNECTED.to_owned())),
            }
        };
        link.open().await.map_err(|e| Failure::Error(e.describe()))
    }

    /// Connects and imports `bus_id`, all within one deadline. Returns the stream and the
    /// server's device record.
    async fn import(&self, bus_id: &str) -> Result<RemoteImport, Failure> {
        self.under_deadline(async {
            let mut session = self.open_session().await?;
            let handshake = async {
                session.write_all(&protocol::import_request(bus_id)).await?;
                let header = protocol::read_header(&mut *session).await?;
                if header.code != protocol::OP_REP_IMPORT || header.status != protocol::STATUS_OK {
                    return Err(Failure::Error(
                        "The server could not export the device. Someone else may be using it."
                            .to_owned(),
                    ));
                }
                let mut record = [0u8; protocol::DEVICE_SIZE];
                session.read_exact(&mut record).await?;
                Ok(record)
            };
            match handshake.await {
                Ok(record) => Ok(RemoteImport { session, record }),
                Err(e) => {
                    session.close(CloseReason::Done).await;
                    Err(e)
                }
            }
        })
        .await
    }

    /// Asks the driver to attach from the gateway. The gateway sees the import and tracks the
    /// device; this adds the driver's port, which detaching needs.
    async fn attach_through_driver(
        &self,
        key: &AttachKey,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let (listed, gateway) = {
            let state = lock(&self.state);
            (
                state.listed,
                state
                    .gateway_port
                    .ok_or_else(|| state.gateway_fault.clone()),
            )
        };
        if !self.connection.current().is_complete() || !listed {
            return Err(Failure::Error(NOT_CONNECTED.to_owned()));
        }
        let gateway_port = gateway.map_err(|fault| {
            Failure::Error(
                fault.unwrap_or_else(|| "The local USB/IP gateway is not running.".to_owned()),
            )
        })?;

        let booking = Arc::new(PendingImport::default());
        lock(&self.state)
            .pending
            .insert(key.clone(), booking.clone());
        let result = async {
            // The driver's own command has a time limit, and killing it half way could leave the
            // attach half done, so it is not cancelled; a change that came meanwhile is undone below.
            let loopback = Ipv4Addr::LOCALHOST.to_string();
            let driver_port = self
                .driver
                .attach(&loopback, gateway_port, &key.bus_id)
                .await
                .map_err(|e| Failure::Error(e.message().to_owned()))?;
            let attachment = {
                let mut state = lock(&self.state);
                state.driver_ports.insert(
                    key.clone(),
                    DriverAttachment {
                        gateway_port,
                        driver_port,
                    },
                );
                state.attachments.get(key).filter(|a| a.driver).cloned()
            };
            if let Some(attachment) = &attachment {
                *lock(&attachment.port) = Some(driver_port);
                tracing::info!("Attached {} on virtual port {driver_port}", key.bus_id);
            }

            // Detached, or the connection changed, while the driver was at it: give back what it
            // made. The release takes the driver's entry out, and if it fails the entry stays so a
            // later release tries again.
            if cancel.is_cancelled() {
                let current = lock(&self.state).attachments.remove(key);
                self.release(key, current.or(attachment)).await;
            }
            self.raise().await;
            Ok(())
        }
        .await;
        let mut state = lock(&self.state);
        if state
            .pending
            .get(key)
            .is_some_and(|p| Arc::ptr_eq(p, &booking))
        {
            state.pending.remove(key);
        }
        result
    }

    /// Attaches through this kernel's virtual host controller.
    async fn attach(
        self: &Arc<Self>,
        key: &AttachKey,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        if let Some(reason) = self.attacher.unavailable() {
            return Err(Failure::Error(reason));
        }
        let bus_id = key.bus_id.as_str();
        let picked = self
            .offered(bus_id)
            .ok_or_else(|| Failure::Error(format!("The server is not offering {bus_id}.")))?;
        let import = tokio::select! {
            import = self.import(bus_id) => import?,
            _ = cancel.cancelled() => return Err(Failure::Cancelled),
        };
        let RemoteImport { session, record } = import;
        let mut session = Some(session);
        let result = self
            .attach_imported(key, &picked, &mut session, &record, cancel)
            .await;
        if let Some(session) = session {
            session.close(CloseReason::Done).await;
        }
        result
    }

    /// The rest of an attach once the server answered. Takes the session only when it hands it
    /// to the relay.
    async fn attach_imported(
        self: &Arc<Self>,
        key: &AttachKey,
        picked: &DeviceView,
        session: &mut Option<Box<dyn DeviceStream>>,
        record: &[u8; protocol::DEVICE_SIZE],
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let bus_id = key.bus_id.as_str();
        let device = protocol::read_device(record);
        check_import(&device, bus_id, Some(picked))?;
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        if self.key_of(bus_id) != *key {
            return Err(Failure::Error(
                "The connection changed while attaching.".to_owned(),
            ));
        }

        let (kernel_end, relay_end) = tokio::select! {
            pair = loopback_pair() => pair.map_err(|e| Failure::Error(e.to_string()))?,
            _ = cancel.cancelled() => return Err(Failure::Cancelled),
        };
        let port = self
            .attacher
            .attach(&kernel_end, device.dev_id(), device.speed)
            .map_err(|e| Failure::Error(e.message().to_owned()))?;

        let attachment = Arc::new(Attachment {
            port: Mutex::new(Some(port)),
            stop: CancellationToken::new(),
            since: self.clock.now(),
            via: None,
            driver: false,
        });
        {
            let mut state = lock(&self.state);
            if state.attachments.contains_key(key) {
                drop(state);
                // Something else attached it first; give the port back.
                let _ = self.attacher.detach(port);
                return Err(Failure::Error(format!("{bus_id} is already attached.")));
            }
            state.attachments.insert(key.clone(), attachment.clone());
        }

        let mut session = session.take().expect("the session is handed over once");
        let inner = self.clone();
        let relayed = attachment.clone();
        let relay_key = key.clone();
        self.tasks.spawn(format!("relay {bus_id}"), async move {
            relay(&mut *session, relay_end, &relayed.stop).await;
            drop(kernel_end);
            session.close(CloseReason::Done).await;
            // Only this attach's own entry goes, and only then is the port freed: a detach and a
            // new attach may have taken the key, and the port, in the meantime.
            if inner.remove_if_same(&relay_key, &relayed) {
                // The link dropped without a detach; free the port.
                let _ = inner.attacher.detach(port);
            }
            inner.raise().await;
        });

        lock(&self.state).faults.remove(bus_id);
        tracing::info!("Attached {bus_id} on virtual port {port}");
        self.activity.record(
            self.device(bus_id),
            ActivityReason::Attached,
            ActivityOutcome::Succeeded,
            format!(
                "{} is attached here on virtual port {port}",
                self.name_of(bus_id)
            ),
            None,
        );
        self.raise().await;

        if cancel.is_cancelled() && self.remove_if_same(key, &attachment) {
            // The connection changed, or a detach came, while this attach finished.
            self.release(key, Some(attachment)).await;
        }
        Ok(())
    }

    async fn serve_gateway(
        &self,
        mut local: TcpStream,
        peer: SocketAddr,
        stopping: &CancellationToken,
    ) {
        let from = match peer.ip() {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => v6.to_canonical().to_string(),
        };

        // A local peer that connects and says nothing must not hold the session open for ever.
        let request = async {
            let header = protocol::read_header(&mut local).await?;
            let reply_code = match header.code {
                protocol::OP_REQ_DEVLIST => protocol::OP_REP_DEVLIST,
                protocol::OP_REQ_IMPORT => protocol::OP_REP_IMPORT,
                _ => return Ok(None),
            };
            let bus_id = if header.code == protocol::OP_REQ_IMPORT {
                Some(protocol::read_bus_id(&mut local).await?)
            } else {
                None
            };
            Ok::<_, std::io::Error>(Some((reply_code, bus_id)))
        };
        let request = tokio::select! {
            r = tokio::time::timeout(self.options.attach_timeout, request) => r,
            _ = stopping.cancelled() => return,
        };
        let (reply_code, bus_id) = match request {
            Ok(Ok(Some(request))) => request,
            Ok(Ok(None)) => return,
            Ok(Err(e)) => {
                tracing::debug!("Gateway session ended: {e}");
                return;
            }
            Err(_) => {
                tracing::warn!(
                    "Gateway request for the request failed: the driver did not finish its request in time"
                );
                return;
            }
        };
        let refuse = protocol::header(reply_code, protocol::STATUS_ERROR);

        if let Some(bus_id) = &bus_id
            && !bus_id::is_valid(bus_id)
        {
            tracing::warn!("Gateway request for an invalid bus ID failed: not a USB bus ID");
            let _ = local.write_all(&refuse).await;
            return;
        }

        // With a driver on this machine the gateway serves only what Koppel itself asked the
        // driver for: one import of one device per request. Another local user's "usbip attach"
        // and the driver's own attempts to attach again after a drop are turned away; Koppel
        // attaches again itself while it is wanted.
        let server = self
            .connection
            .current()
            .client_id
            .clone()
            .unwrap_or_default();
        let booked = {
            let state = lock(&self.state);
            if self.uses_driver(&state) {
                let claimed = bus_id.as_ref().and_then(|b| {
                    let key = AttachKey {
                        server: server.clone(),
                        bus_id: b.clone(),
                    };
                    state.pending.get(&key).filter(|p| p.try_claim()).cloned()
                });
                Some(claimed)
            } else {
                None
            }
        };
        let booked = match booked {
            Some(None) => {
                tracing::warn!(
                    "Gateway request for {} failed: Koppel did not ask the driver for it",
                    bus_id.as_deref().unwrap_or("the device list")
                );
                let _ = local.write_all(&refuse).await;
                return;
            }
            Some(claimed) => claimed,
            None => None,
        };

        let Some(bus_id) = bus_id else {
            // A device list, passed through from the server as it came.
            let listed = async {
                if self.settings.current().role == Role::Server {
                    return Err(Failure::Error(
                        "This machine is a server, not a client.".to_owned(),
                    ));
                }
                self.under_deadline(async {
                    let mut session = self.open_session().await?;
                    let copied = async {
                        session.write_all(&protocol::dev_list_request()).await?;
                        tokio::io::copy(&mut session, &mut local).await?;
                        Ok::<_, std::io::Error>(())
                    }
                    .await;
                    session.close(CloseReason::Done).await;
                    copied.map_err(Failure::from)
                })
                .await
            };
            if let Err(Failure::Error(reason)) = listed.await {
                self.report_failure(None, reason, false).await;
                let _ = local.write_all(&refuse).await;
            }
            return;
        };

        let imported = async {
            if self.settings.current().role == Role::Server {
                return Err(Failure::Error(
                    "This machine is a server, not a client.".to_owned(),
                ));
            }
            let import = self.import(&bus_id).await?;
            let offered = self.offered(&bus_id);
            if let Err(reason) = check_import(
                &protocol::read_device(&import.record),
                &bus_id,
                offered.as_ref(),
            ) {
                import.session.close(CloseReason::Done).await;
                return Err(Failure::Error(reason));
            }
            Ok(import)
        };
        let import = match imported.await {
            Ok(import) => import,
            Err(failure) => {
                if let Failure::Error(reason) = failure {
                    self.report_failure(Some(&bus_id), reason, false).await;
                }
                let _ = local.write_all(&refuse).await;
                return;
            }
        };

        let RemoteImport {
            mut session,
            record,
        } = import;
        let key = self.key_of(&bus_id);
        // An import this client's own attach asked the driver for is "attached here", not a visitor.
        let mine = booked.is_some();
        let attachment = Arc::new(Attachment {
            port: Mutex::new(None),
            stop: stopping.child_token(),
            since: self.clock.now(),
            via: (!mine).then(|| from.clone()),
            driver: mine,
        });
        let added = {
            let mut state = lock(&self.state);
            let free = !state.attachments.contains_key(&key);
            if free {
                state.attachments.insert(key.clone(), attachment.clone());
            }
            free
        };
        if !added {
            tracing::warn!("Gateway request for {bus_id} failed: already attached");
            let _ = local
                .write_all(&protocol::header(
                    protocol::OP_REP_IMPORT,
                    protocol::STATUS_ERROR,
                ))
                .await;
            session.close(CloseReason::Done).await;
            return;
        }

        // Pass the server's device record on as it came, interface count and all.
        let mut reply = protocol::header(protocol::OP_REP_IMPORT, protocol::STATUS_OK).to_vec();
        reply.extend_from_slice(&record);
        if local.write_all(&reply).await.is_ok() {
            lock(&self.state).faults.remove(&bus_id);
            tracing::info!("Attached {bus_id} through the gateway for {from}");
            let name = self.name_of(&bus_id);
            self.activity.record(
                self.device(&bus_id),
                ActivityReason::Attached,
                ActivityOutcome::Succeeded,
                if mine {
                    format!("{name} is attached here")
                } else {
                    format!("{name} is attached through the gateway for {from}")
                },
                None,
            );
            self.raise().await;
            relay(&mut *session, &mut local, &attachment.stop).await;
        }

        if self.remove_if_same(&key, &attachment) {
            // Released on the other side, or the link dropped. A detach here records itself.
            let name = self.name_of(&bus_id);
            self.activity.record(
                self.device(&bus_id),
                ActivityReason::Detached,
                ActivityOutcome::Succeeded,
                if mine {
                    format!("{name} was detached when its connection ended")
                } else {
                    format!("{name} was released by {from}")
                },
                None,
            );
        }
        session.close(CloseReason::Done).await;
        self.raise().await;
    }
}

/// What a server sends is only trusted once it is checked: a bus ID that is not one is dropped,
/// so it never reaches the UI, a log line, or a command line.
fn sanitize(received: RemoteDevices) -> RemoteDevices {
    let total = received.devices.len();
    let mut seen = HashSet::new();
    let devices: Vec<DeviceView> = received
        .devices
        .into_iter()
        .filter(|d| bus_id::is_valid(&d.bus_id) && seen.insert(d.bus_id.clone()))
        .collect();
    if devices.len() != total {
        tracing::warn!(
            "Ignored {} devices from the server with a bus ID that is not valid",
            total - devices.len()
        );
    }
    RemoteDevices { devices }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dongle() -> UsbDevice {
        UsbDevice {
            bus_id: "1-1.4".into(),
            vendor_id: 0x064f,
            product_id: 0x2af9,
            ..UsbDevice::default()
        }
    }

    #[test]
    fn an_import_reply_for_another_bus_id_or_another_device_is_refused() {
        let picked = DeviceView {
            bus_id: "1-1.4".into(),
            vid_pid: "064f:2af9".into(),
            ..DeviceView::default()
        };

        assert!(check_import(&dongle(), "1-1.4", Some(&picked)).is_ok());
        let other_port = check_import(
            &UsbDevice {
                bus_id: "1-1.5".into(),
                ..dongle()
            },
            "1-1.4",
            Some(&picked),
        );
        let other_device = check_import(
            &UsbDevice {
                vendor_id: 0x0403,
                product_id: 0x6001,
                ..dongle()
            },
            "1-1.4",
            Some(&picked),
        );
        assert!(other_port.unwrap_err().contains("different device"));
        assert!(other_device.unwrap_err().contains("0403:6001"));
    }

    #[test]
    fn the_same_server_and_credentials_continue_across_a_certificate_switch() {
        let before = ClientConnection {
            server_url: Some("https://server:5240".into()),
            server_fingerprint: Some("AA:BB".into()),
            client_id: Some("c1".into()),
            client_secret: Some("s".into()),
            next_server_fingerprint: Some("CC:DD".into()),
        };
        let switched = ClientConnection {
            server_fingerprint: Some("ccdd".into()),
            next_server_fingerprint: None,
            ..before.clone()
        };
        let other = ClientConnection {
            client_secret: Some("t".into()),
            ..before.clone()
        };

        assert!(continues(&switched, &before));
        assert!(!continues(&other, &before));
    }

    #[test]
    fn addresses_compare_without_case_or_the_default_port() {
        assert_eq!(authority("https://Server:443/x"), "server");
        assert_eq!(authority("https://server:5240"), "server:5240");
        assert_eq!(url_host("https://10.0.0.5:5240"), "10.0.0.5");
        assert_eq!(url_host("https://[fe80::1]:5240"), "[fe80::1]");
    }

    #[test]
    fn devices_with_a_bus_id_that_is_not_one_are_dropped() {
        let received = RemoteDevices {
            devices: vec![
                DeviceView {
                    bus_id: "1-1.4".into(),
                    ..DeviceView::default()
                },
                DeviceView {
                    bus_id: "1-1.5 --stop".into(),
                    ..DeviceView::default()
                },
                DeviceView {
                    bus_id: "1-1.4".into(),
                    name: "twice".into(),
                    ..DeviceView::default()
                },
            ],
        };

        let devices = sanitize(received).devices;

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].bus_id, "1-1.4");
        assert_eq!(devices[0].name, "");
    }
}

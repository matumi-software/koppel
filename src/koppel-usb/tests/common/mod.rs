//! Fakes for the USB tests: a kernel that plays both ends, a usbip-win2 driver, and a link that
//! joins a client to a server in memory. Each test file uses part of it.
#![allow(dead_code)]

pub mod files;

use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::net::{Shutdown, TcpStream as StdTcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koppel_config::files::{AttachedDevices, ClientConnection, KoppelSettings, Role};
use koppel_config::{CodecError, ConfigFile, FileCodec, KoppelPaths, Options, PlainText, open};
use koppel_usb::peer_check::AnyPeer;
use koppel_usb::stream::memory;
use koppel_usb::{
    ActivityOutcome, ActivityReason, ActivitySink, BoxFuture, BusId, ClientOptions, CloseReason,
    Deps, DeviceStream, EntityRef, Export, ExportStatus, GatewayOptions, Link, LinkEvents,
    LinkState, LinkStatus, Links, Listener, OpenError, Platform, RemoteClient, ServerDeps,
    StreamOpener, SystemClock, UsbAttacher, UsbClient, UsbDataPlane, UsbDevice, UsbError, UsbHost,
    UsbInterface, UsbIpDriver, UsbIpGateway, UsbServer, UsbSpeed, VhciPort, protocol,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The dongle every test shares: a CodeMeter stick on 1-1.4.
pub fn dongle() -> UsbDevice {
    UsbDevice {
        path: "/sys/devices/fake/1-1.4".into(),
        bus_num: 1,
        dev_num: 5,
        speed: UsbSpeed::Full,
        vendor_id: 0x064f,
        product_id: 0x2af9,
        bcd_device: 0x0100,
        configuration_value: 1,
        num_configurations: 1,
        interfaces: vec![UsbInterface::new(0xff, 0, 0)],
        manufacturer: Some("WIBU-SYSTEMS AG".into()),
        product: Some("CodeMeter-Stick".into()),
        serial: Some("SERIAL-123".into()),
        driver: Some("usbip-host".into()),
        ..UsbDevice::new("1-1.4".parse().unwrap())
    }
}

/// Polls `condition` until it holds, for up to 30 seconds.
pub async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let until = tokio::time::Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(
            tokio::time::Instant::now() < until,
            "{what} did not happen in 30 seconds"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// As [`eventually`], for a condition that has to await.
pub async fn eventually_async<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let until = tokio::time::Instant::now() + Duration::from_secs(30);
    while !condition().await {
        assert!(
            tokio::time::Instant::now() < until,
            "{what} did not happen in 30 seconds"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A folder of its own under the temp folder, removed afterwards.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "koppel-usb-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A gate a fake waits at until the test opens it.
#[derive(Clone, Default)]
pub struct Gate(CancellationToken);

impl Gate {
    pub fn open(&self) {
        self.0.cancel();
    }

    pub async fn wait(&self) {
        self.0.cancelled().await;
    }
}

/// The activity log, kept for the test to read.
#[derive(Default)]
pub struct Log(Mutex<Vec<(ActivityReason, ActivityOutcome, String)>>);

impl Log {
    pub fn events(&self) -> Vec<(ActivityReason, ActivityOutcome, String)> {
        lock(&self.0).clone()
    }
}

impl ActivitySink for Log {
    fn record(
        &self,
        _: EntityRef,
        reason: ActivityReason,
        outcome: ActivityOutcome,
        summary: String,
        _: Option<String>,
    ) {
        lock(&self.0).push((reason, outcome, summary));
    }
}

/// Stands in for the Linux kernel on both ends. The "server kernel" echoes whatever arrives on an
/// exported stream, the way usbip-host would answer URBs. The "client kernel" keeps the attached
/// socket for the test, which plays the USB driver. The server side is stateful: sharing binds
/// the device to usbip-host, unsharing gives it back, and a test can make either refuse.
pub struct FakeKernel {
    state: Mutex<KernelState>,
    /// The client kernel takes a pipe in memory instead of a socket, so an attach opens no real
    /// socket and a test can run on the paused clock.
    memory: bool,
    /// The devices a client has imported.
    used: Arc<Mutex<HashSet<BusId>>>,
    reset_started: Semaphore,
    gates: Mutex<Gates>,
}

#[derive(Default)]
struct Gates {
    reset: Option<Gate>,
    unshare: Option<Gate>,
}

struct KernelState {
    bound: HashSet<String>,
    devices: Vec<UsbDevice>,
    reset: bool,
    reset_before_export: bool,
    share_failure: Option<String>,
    shared: Vec<String>,
    unshared: Vec<String>,
    list_failures: usize,
    list_failure: String,
    list_failed: bool,
    listings: usize,
    status_failures: Vec<(String, String)>,
    attached: Vec<StdTcpStream>,
    /// The ends of in-memory attaches; `None` once the test took one to play the driver on.
    attached_memory: Vec<Option<tokio::io::DuplexStream>>,
    detached_ports: Vec<u32>,
}

impl Default for FakeKernel {
    fn default() -> Self {
        Self {
            state: Mutex::new(KernelState {
                bound: HashSet::from(["1-1.4".to_owned()]),
                devices: vec![dongle()],
                reset: false,
                reset_before_export: false,
                share_failure: None,
                shared: Vec::new(),
                unshared: Vec::new(),
                list_failures: 0,
                list_failure: String::new(),
                list_failed: false,
                listings: 0,
                status_failures: Vec::new(),
                attached: Vec::new(),
                attached_memory: Vec::new(),
                detached_ports: Vec::new(),
            }),
            memory: false,
            used: Arc::new(Mutex::new(HashSet::new())),
            reset_started: Semaphore::new(0),
            gates: Mutex::new(Gates::default()),
        }
    }
}

impl FakeKernel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// As [`FakeKernel::new`], with a client kernel that takes an in-memory pipe, for the tests
    /// that run on the paused clock. [`FakeKernel::driver_pipe`] is the driver's end of it.
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            memory: true,
            ..Self::default()
        })
    }

    /// Replaces the devices on the bus; a device keeps its binding by bus ID.
    pub fn plug(&self, devices: Vec<UsbDevice>) {
        lock(&self.state).devices = devices;
    }

    /// Binds or unbinds a device behind Koppel's back, as a person with the usbip tool could.
    pub fn bind(&self, bus_id: &str, bound: bool) {
        let mut state = lock(&self.state);
        if bound {
            state.bound.insert(bus_id.to_owned());
        } else {
            state.bound.remove(bus_id);
        }
    }

    pub fn is_bound(&self, bus_id: &str) -> bool {
        lock(&self.state).bound.contains(bus_id)
    }

    pub fn set_share_failure(&self, failure: Option<&str>) {
        lock(&self.state).share_failure = failure.map(str::to_owned);
    }

    pub fn shared(&self) -> Vec<String> {
        lock(&self.state).shared.clone()
    }

    pub fn unshared(&self) -> Vec<String> {
        lock(&self.state).unshared.clone()
    }

    /// The next `times` listings fail with `message`.
    pub fn fail_listings(&self, message: &str, times: usize) {
        let mut state = lock(&self.state);
        state.list_failure = message.to_owned();
        state.list_failures = times;
    }

    /// How many times the host was asked for its devices.
    pub fn listings(&self) -> usize {
        lock(&self.state).listings
    }

    /// Reading the state of `bus_id` fails with `message`.
    pub fn fail_status(&self, bus_id: &str, message: &str) {
        lock(&self.state)
            .status_failures
            .push((bus_id.to_owned(), message.to_owned()));
    }

    pub fn listing_failed(&self) -> bool {
        lock(&self.state).list_failed
    }

    pub fn reset_before_export(&self) -> bool {
        lock(&self.state).reset_before_export
    }

    /// While the returned gate is shut, a reset blocks, so the server stalls an import after the
    /// client was let in.
    pub fn hold_resets(&self) -> Gate {
        let gate = Gate::default();
        lock(&self.gates).reset = Some(gate.clone());
        gate
    }

    /// While the returned gate is shut, an unshare blocks.
    pub fn hold_unshares(&self) -> Gate {
        let gate = Gate::default();
        lock(&self.gates).unshare = Some(gate.clone());
        gate
    }

    /// Waits up to `timeout` for a reset to start. False when none did.
    pub async fn reset_started(&self, timeout: Duration) -> bool {
        match tokio::time::timeout(timeout, self.reset_started.acquire()).await {
            Ok(Ok(permit)) => {
                permit.forget();
                true
            }
            _ => false,
        }
    }

    /// How many times the client kernel attached a device.
    pub fn attaches(&self) -> usize {
        let state = lock(&self.state);
        state.attached.len() + state.attached_memory.len()
    }

    /// The ports the client kernel detached, in order.
    pub fn detached_ports(&self) -> Vec<u32> {
        lock(&self.state).detached_ports.clone()
    }

    /// The socket of the `n`th attach (from 1), once there is one, as the USB driver holds it.
    pub async fn attached_socket(&self, n: usize) -> StdTcpStream {
        eventually("the attach", || self.attaches() >= n).await;
        lock(&self.state).attached[n - 1].try_clone().unwrap()
    }

    /// The driver's end of the `n`th in-memory attach (from 1), once there is one, for the test to
    /// play the USB driver on. Only for a kernel made with [`FakeKernel::in_memory`].
    pub async fn driver_pipe(&self, n: usize) -> tokio::io::DuplexStream {
        eventually("the attach", || self.attaches() >= n).await;
        lock(&self.state).attached_memory[n - 1]
            .take()
            .expect("the driver's end is taken once")
    }

    /// The `n`th attached socket as an async stream, for the test to play the USB driver on.
    pub async fn driver_stream(&self, n: usize) -> TcpStream {
        let socket = self.attached_socket(n).await;
        socket.set_nonblocking(true).unwrap();
        TcpStream::from_std(socket).unwrap()
    }
}

impl UsbHost for FakeKernel {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn capabilities(&self) -> koppel_usb::Capabilities {
        koppel_usb::Capabilities::KERNEL
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        let mut state = lock(&self.state);
        state.listings += 1;
        let listed = if state.list_failures > 0 {
            state.list_failures -= 1;
            state.list_failed = true;
            Err(UsbError::failed(state.list_failure.clone()))
        } else {
            Ok(state
                .devices
                .iter()
                .map(|d| UsbDevice {
                    driver: Some(if state.bound.contains(d.bus_id.as_str()) {
                        "usbip-host".into()
                    } else {
                        "ftdi_sio".into()
                    }),
                    ..d.clone()
                })
                .collect())
        };
        Box::pin(async move { listed })
    }

    fn is_shared(&self, device: &UsbDevice) -> bool {
        device.driver.as_deref() == Some("usbip-host")
    }

    fn sharing_advice(&self, _: &BusId, _: bool) -> String {
        "Koppel shares devices itself.".into()
    }

    fn share<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            if let Some(failure) = &state.share_failure {
                return Err(UsbError::failed(failure.clone()));
            }
            state.shared.push(bus_id.to_string());
            state.bound.insert(bus_id.to_string());
            Ok(())
        })
    }

    fn unshare<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async move {
            let gate = lock(&self.gates).unshare.clone();
            if let Some(gate) = gate {
                gate.wait().await;
            }
            let mut state = lock(&self.state);
            state.unshared.push(bus_id.to_string());
            state.bound.remove(bus_id.as_str());
            Ok(())
        })
    }

    fn status<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        if let Some((_, message)) = lock(&self.state)
            .status_failures
            .iter()
            .find(|(id, _)| id == bus_id.as_str())
        {
            let error = UsbError::failed(message.clone());
            return Box::pin(async move { Err(error) });
        }
        let status = if lock(&self.used).contains(bus_id) {
            ExportStatus::Used
        } else {
            ExportStatus::Available
        };
        Box::pin(async move { Ok(status) })
    }

    fn reset<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async move {
            self.reset_started.add_permits(1);
            let gate = lock(&self.gates).reset.clone();
            if let Some(gate) = gate {
                gate.wait().await;
            }
            lock(&self.state).reset = true;
            Ok(())
        })
    }

    fn export<'a>(
        &'a self,
        device: &'a UsbDevice,
    ) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        Box::pin(async move {
            {
                let mut state = lock(&self.state);
                state.reset_before_export = state.reset;
                state.reset = false;
            }
            lock(&self.used).insert(device.bus_id.clone());
            Ok(Box::new(EchoExport {
                used: self.used.clone(),
                bus_id: device.bus_id.clone(),
            }) as Box<dyn Export>)
        })
    }
}

/// Echoes what arrives, as usbip-host answers URBs, and frees the device when done.
struct EchoExport {
    used: Arc<Mutex<HashSet<BusId>>>,
    bus_id: BusId,
}

impl Drop for EchoExport {
    fn drop(&mut self) {
        lock(&self.used).remove(&self.bus_id);
    }
}

impl Export for EchoExport {
    fn relay<'a>(
        self: Box<Self>,
        network: &'a mut dyn DeviceStream,
        stop: CancellationToken,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let echo = async {
                let mut buffer = [0u8; 4096];
                loop {
                    let read = network.read(&mut buffer).await?;
                    if read == 0 {
                        return Ok::<_, std::io::Error>(());
                    }
                    network.write_all(&buffer[..read]).await?;
                }
            };
            tokio::select! {
                _ = echo => {}
                _ = stop.cancelled() => {}
            }
            drop(self);
        })
    }
}

impl UsbAttacher for FakeKernel {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn ports(&self) -> Vec<VhciPort> {
        Vec::new()
    }

    fn attach(&self, socket: &StdTcpStream, _: u32, _: UsbSpeed) -> Result<u32, UsbError> {
        let kept = socket
            .try_clone()
            .map_err(|e| UsbError::failed(e.to_string()))?;
        lock(&self.state).attached.push(kept);
        Ok(8)
    }

    fn detach(&self, port: u32) -> Result<(), UsbError> {
        let mut state = lock(&self.state);
        state.detached_ports.push(port);
        if let Some(socket) = state.attached.last() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        // Dropping the pipe's end closes it, as shutting the socket down does.
        if let Some(end) = state.attached_memory.last_mut() {
            end.take();
        }
        Ok(())
    }

    fn in_memory(&self) -> bool {
        self.memory
    }

    fn attach_in_memory(
        &self,
        end: tokio::io::DuplexStream,
        _: u32,
        _: UsbSpeed,
    ) -> Result<u32, UsbError> {
        lock(&self.state).attached_memory.push(Some(end));
        Ok(8)
    }
}

/// Plays usbip-win2's driver: on attach it connects to the given USB/IP server (Koppel's gateway),
/// imports the device and keeps the connection, as the kernel driver would.
#[derive(Default)]
pub struct FakeDriver {
    attaches: AtomicUsize,
    current: Mutex<Option<StdTcpStream>>,
    kept: Mutex<Vec<TcpStream>>,
    first: Mutex<Option<TcpStream>>,
    detached: Mutex<Vec<String>>,
    attach_gate: Mutex<Option<Gate>>,
    detach_gate: Mutex<Option<Gate>>,
}

impl FakeDriver {
    pub const DRIVER_PORT: u32 = 1;

    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn attaches(&self) -> usize {
        self.attaches.load(Ordering::SeqCst)
    }

    pub fn detached(&self) -> Vec<String> {
        lock(&self.detached).clone()
    }

    /// The first connection the driver made, once there is one, for the test to play USB on.
    pub async fn connection(&self) -> TcpStream {
        eventually("the driver's connection", || lock(&self.first).is_some()).await;
        lock(&self.first).take().unwrap()
    }

    /// Until the returned gate opens, an attach returns only after it imported, as usbip.exe
    /// does when it takes a while.
    pub fn hold_attaches(&self) -> Gate {
        let gate = Gate::default();
        *lock(&self.attach_gate) = Some(gate.clone());
        gate
    }

    pub fn hold_detaches(&self) -> Gate {
        let gate = Gate::default();
        *lock(&self.detach_gate) = Some(gate.clone());
        gate
    }

    /// The device goes away on the network side, as when the link drops.
    pub fn drop_connection(&self) {
        if let Some(socket) = lock(&self.current).as_ref() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

impl UsbIpDriver for FakeDriver {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn attach<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        bus_id: &'a BusId,
    ) -> BoxFuture<'a, Result<u32, UsbError>> {
        Box::pin(async move {
            self.attaches.fetch_add(1, Ordering::SeqCst);
            let fail = |e: std::io::Error| UsbError::failed(e.to_string());
            let mut stream = TcpStream::connect((host, port)).await.map_err(fail)?;
            stream
                .write_all(&protocol::import_request(bus_id))
                .await
                .map_err(fail)?;
            let reply = protocol::read_header(&mut stream).await.map_err(fail)?;
            if reply.status != protocol::STATUS_OK {
                return Err(UsbError::failed("Device not available."));
            }
            protocol::read_imported_device(&mut stream)
                .await
                .map_err(fail)?;
            let raw = stream.into_std().map_err(fail)?;
            *lock(&self.current) = Some(raw.try_clone().map_err(fail)?);
            let stream = TcpStream::from_std(raw).map_err(fail)?;
            {
                let mut first = lock(&self.first);
                if first.is_none() && self.attaches() == 1 {
                    *first = Some(stream);
                } else {
                    lock(&self.kept).push(stream);
                }
            }
            let gate = lock(&self.attach_gate).clone();
            if let Some(gate) = gate {
                gate.wait().await;
            }
            Ok(Self::DRIVER_PORT)
        })
    }

    fn detach<'a>(
        &'a self,
        _: &'a str,
        _: u16,
        bus_id: &'a BusId,
        driver_port: u32,
    ) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async move {
            lock(&self.detached).push(format!("{bus_id}@{driver_port}"));
            self.drop_connection();
            let gate = lock(&self.detach_gate).clone();
            if let Some(gate) = gate {
                gate.wait().await;
            }
            // usbip.exe returns once the device is gone, by which time the gateway session has
            // ended too.
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(())
        })
    }
}

/// The server side of a test: a USB server and its data plane over a host, with its own folder.
pub struct ServerSide {
    pub dir: TempDir,
    pub settings: ConfigFile<KoppelSettings>,
    pub server: UsbServer,
    pub plane: UsbDataPlane,
    pub activity: Arc<Log>,
}

impl ServerSide {
    /// A server whose `devices.json` already shares the dongle when `share_dongle` is set, as a
    /// person who shared it in the UI left it.
    pub async fn start(host: Arc<dyn UsbHost>, share_dongle: bool, share_on_startup: bool) -> Self {
        let devices = share_dongle
            .then_some(r#"{ "busIds": ["1-1.4"], "vidPids": { "1-1.4": "064f:2af9" } }"#);
        Self::open(server_dir(share_on_startup, devices), host).await
    }

    /// A server over a folder the test prepared.
    pub async fn open(dir: TempDir, host: Arc<dyn UsbHost>) -> Self {
        let paths = KoppelPaths::new(dir.path().join("config"), dir.path().join("keys"));
        let settings = open::settings(&paths);
        settings.start().await.unwrap();
        let activity = Arc::new(Log::default());
        let server = UsbServer::new(ServerDeps {
            host,
            shared: files::shared(&open::devices(&paths)),
            settings: files::settings(&settings),
            activity: activity.clone(),
            clock: Arc::new(SystemClock),
        });
        server.start().await.unwrap();
        let plane = UsbDataPlane::new(server.clone(), activity.clone(), files::settings(&settings));
        plane.start();
        Self {
            dir,
            settings,
            server,
            plane,
            activity,
        }
    }

    pub fn config_file(&self, name: &str) -> PathBuf {
        self.dir.path().join("config").join(name)
    }

    pub async fn set_role(&self, role: Role) {
        self.settings
            .update(|s| KoppelSettings { role, ..s.clone() })
            .await
            .unwrap();
    }

    /// Opens a stream to the data plane, as a client's link would.
    pub fn open_stream(&self, client: &str) -> memory::MemoryStream {
        let (client_end, server_end) = memory::pair();
        let plane = self.plane.clone();
        let client = RemoteClient {
            id: client.to_owned(),
            name: client.to_owned(),
        };
        tokio::spawn(async move { plane.serve(Box::new(server_end), client).await });
        client_end
    }

    pub async fn stop(&self) {
        self.plane.stop().await;
        self.server.stop().await;
        self.settings.stop();
    }
}

/// A server's folder with its settings (role server) and, when given, its `devices.json`.
pub fn server_dir(share_on_startup: bool, devices: Option<&str>) -> TempDir {
    let dir = TempDir::new("server");
    let config = dir.path().join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        format!(r#"{{ "role": "server", "shareOnStartup": {share_on_startup} }}"#),
    )
    .unwrap();
    if let Some(devices) = devices {
        fs::write(config.join("devices.json"), devices).unwrap();
    }
    dir
}

/// Joins clients to a server in memory: the device list follows the server's, as the remote hub
/// sends it, and each stream is served by the server's data plane.
pub struct MemoryLinks {
    server: UsbServer,
    plane: UsbDataPlane,
    down: Arc<AtomicBool>,
    made: AtomicUsize,
}

impl MemoryLinks {
    pub fn new(server: &ServerSide) -> Arc<Self> {
        Arc::new(Self {
            server: server.server.clone(),
            plane: server.plane.clone(),
            down: Arc::new(AtomicBool::new(false)),
            made: AtomicUsize::new(0),
        })
    }

    /// The server is gone: no stream opens any more.
    pub fn take_down(&self) {
        self.down.store(true, Ordering::SeqCst);
    }

    /// How many links were made.
    pub fn made(&self) -> usize {
        self.made.load(Ordering::SeqCst)
    }
}

impl Links for MemoryLinks {
    fn connect(&self, connection: &koppel_usb::Connection, events: LinkEvents) -> Arc<dyn Link> {
        self.made.fetch_add(1, Ordering::SeqCst);
        Arc::new(MemoryLink {
            server: self.server.clone(),
            plane: self.plane.clone(),
            down: self.down.clone(),
            events,
            client_id: connection.client_id.clone().unwrap_or_default(),
            address: connection.server_url.clone().unwrap_or_default(),
            listener: Mutex::new(None),
        })
    }
}

struct MemoryLink {
    server: UsbServer,
    plane: UsbDataPlane,
    down: Arc<AtomicBool>,
    events: LinkEvents,
    client_id: String,
    address: String,
    listener: Mutex<Option<Listener>>,
}

impl Link for MemoryLink {
    fn start(&self) {
        let events = self.events.clone();
        let hub = self.events.clone();
        let listener = self.server.on_changed(move |devices| {
            let hub = hub.clone();
            async move { hub.devices(devices.for_remote()).await }
        });
        *lock(&self.listener) = Some(listener);
        let server = self.server.clone();
        tokio::spawn(async move {
            events.status(LinkStatus::Connecting).await;
            events.status(LinkStatus::Connected).await;
            // A new connection gets the list at once.
            events.devices(server.current().for_remote()).await;
        });
    }

    fn address(&self) -> String {
        self.address.clone()
    }

    fn stop(&self) -> BoxFuture<'static, ()> {
        if let Some(listener) = lock(&self.listener).take() {
            self.server.remove_listener(listener);
        }
        Box::pin(async {})
    }
}

impl StreamOpener for MemoryLink {
    fn open(&self) -> BoxFuture<'_, Result<Box<dyn DeviceStream>, OpenError>> {
        Box::pin(async move {
            if self.down.load(Ordering::SeqCst) {
                return Err(OpenError::Failed(
                    "Could not reach the server. Check that it is running and on the network."
                        .into(),
                ));
            }
            let (client_end, server_end) = memory::pair();
            let plane = self.plane.clone();
            let client = RemoteClient {
                id: self.client_id.clone(),
                name: format!("client {}", self.client_id),
            };
            tokio::spawn(async move { plane.serve(Box::new(server_end), client).await });
            Ok(Box::new(client_end) as Box<dyn DeviceStream>)
        })
    }
}

/// The client side of a test: a USB client and its gateway, linked through [`MemoryLinks`].
pub struct ClientSide {
    pub dir: TempDir,
    pub settings: ConfigFile<KoppelSettings>,
    pub connection: ConfigFile<ClientConnection>,
    pub client: UsbClient,
    pub gateway: UsbIpGateway,
    pub links: Arc<MemoryLinks>,
    pub activity: Arc<Log>,
    credentials: AtomicUsize,
}

pub struct ClientSetup {
    pub role: Role,
    /// How `attached.json` is written; plain text unless a test needs a save to fail.
    pub attached_codec: Arc<dyn FileCodec>,
    pub attacher: Option<Arc<dyn UsbAttacher>>,
    pub driver: Option<Arc<dyn UsbIpDriver>>,
    pub gateway: GatewayOptions,
    pub options: ClientOptions,
}

impl ClientSetup {
    /// A client role machine with this attacher, no driver and no gateway.
    pub fn kernel(attacher: Arc<dyn UsbAttacher>) -> Self {
        Self {
            role: Role::Client,
            attached_codec: Arc::new(PlainText),
            attacher: Some(attacher),
            driver: None,
            gateway: GatewayOptions {
                port: 0,
                ..GatewayOptions::default()
            },
            options: ClientOptions::default(),
        }
    }

    /// With the gateway on a free port of its own.
    pub fn with_gateway(mut self) -> Self {
        self.gateway.port = free_port();
        self
    }

    /// With a USB/IP driver on this machine, which needs the gateway.
    pub fn driver(driver: Arc<dyn UsbIpDriver>) -> Self {
        let mut setup = Self::kernel(FakeKernel::new());
        setup.attacher = None;
        setup.driver = Some(driver);
        setup.with_gateway()
    }
}

/// A port nothing listens on right now.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl ClientSide {
    pub async fn start(links: Arc<MemoryLinks>, setup: ClientSetup) -> Self {
        Self::start_in(TempDir::new("client"), links, setup).await
    }

    /// A client over a folder the test may have put files in already.
    pub async fn start_in(dir: TempDir, links: Arc<MemoryLinks>, setup: ClientSetup) -> Self {
        let paths = KoppelPaths::new(dir.path().join("config"), dir.path().join("keys"));
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            paths.settings_file(),
            format!(r#"{{ "role": "{}" }}"#, setup.role.as_str()),
        )
        .unwrap();
        let settings = open::settings(&paths);
        settings.start().await.unwrap();
        let connection = ConfigFile::new(
            paths.connection_file(),
            ClientConnection::default(),
            Options::default(),
        );
        connection.start().await.unwrap();
        let activity = Arc::new(Log::default());
        let client = UsbClient::new(Deps {
            settings: files::settings(&settings),
            connection: files::connection(&connection),
            remembered: files::attached(&ConfigFile::new(
                paths.attached_file(),
                AttachedDevices::default(),
                Options {
                    codec: setup.attached_codec,
                    start_with_defaults_when_unusable: true,
                    ..Options::default()
                },
            )),
            platform: {
                let mut platform = Platform::unsupported("No driver or vhci-hcd here.");
                platform.attacher = setup.attacher;
                platform.driver = setup.driver;
                platform
            },
            links: links.clone(),
            activity: activity.clone(),
            clock: Arc::new(SystemClock),
            options: setup.options,
        });
        client.start().await.unwrap();
        // Every test connection comes from this process, so the gateway serves everyone, as
        // 0.1.x's tests had it.
        let gateway = UsbIpGateway::new(
            client.clone(),
            files::settings(&settings),
            setup.gateway,
            Some(Arc::new(AnyPeer)),
        );
        gateway.start().await;
        Self {
            dir,
            settings,
            connection,
            client,
            gateway,
            links,
            activity,
            credentials: AtomicUsize::new(0),
        }
    }

    pub fn config_file(&self, name: &str) -> PathBuf {
        self.dir.path().join("config").join(name)
    }

    /// Saves a connection with new credentials, as pairing with the server again does, and waits
    /// until the link lists the server's devices.
    pub async fn link(&self) {
        let n = self.credentials.fetch_add(1, Ordering::SeqCst) + 1;
        self.set_role(Role::Client).await;
        self.connection
            .update(|_| ClientConnection {
                server_url: Some("https://server.test:5240".into()),
                server_fingerprint: Some(
                    "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89"
                        .into(),
                ),
                client_id: Some(format!("client-{n}")),
                client_secret: Some(format!("secret-{n}").into()),
                next_server_fingerprint: None,
            })
            .await
            .unwrap();
        let client = self.client.clone();
        eventually("the link to list the server's devices", || {
            let current = client.current();
            current.link == LinkState::Connected && !current.devices.is_empty()
        })
        .await;
    }

    pub async fn set_role(&self, role: Role) {
        self.settings
            .update(|s| KoppelSettings { role, ..s.clone() })
            .await
            .unwrap();
    }

    /// Whether the first listed device shows as attached here.
    pub fn attached_here(&self) -> bool {
        self.client
            .current()
            .devices
            .first()
            .is_some_and(|d| d.attached_here)
    }

    pub fn fault(&self) -> Option<String> {
        self.client
            .current()
            .devices
            .first()
            .and_then(|d| d.fault.clone())
    }

    /// Attaches the dongle from the UI and waits until it shows as attached here.
    pub async fn attach(&self) {
        self.client.begin_attach("1-1.4").await.unwrap();
        eventually("the attach", || self.attached_here()).await;
    }

    pub async fn stop(&self) {
        self.gateway.stop().await;
        self.client.stop().await;
        self.settings.stop();
        self.connection.stop();
    }
}

/// Sends a fake URB as the USB driver would and expects the fake server kernel to echo it.
pub async fn echoes<S: AsyncReadExt + AsyncWriteExt + Unpin>(driver: &mut S) {
    let urb = b"CMD_SUBMIT fake urb";
    driver.write_all(urb).await.unwrap();
    let mut echoed = [0u8; 19];
    tokio::time::timeout(Duration::from_secs(10), driver.read_exact(&mut echoed))
        .await
        .expect("the echo arrives")
        .unwrap();
    assert_eq!(&echoed, urb);
}

/// Closes a stream the test opened, normally.
pub async fn close(stream: memory::MemoryStream) {
    Box::new(stream).close(CloseReason::Done).await;
}

/// Plain text that refuses to write once told to, so a save fails the way a full disk would.
#[derive(Default)]
pub struct FailingCodec(pub AtomicBool);

impl FileCodec for FailingCodec {
    fn encode(&self, json: &[u8]) -> Result<Vec<u8>, CodecError> {
        if self.0.load(Ordering::SeqCst) {
            return Err(CodecError("the disk is full".into()));
        }
        Ok(json.to_vec())
    }

    fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
        Ok(bytes.to_vec())
    }
}

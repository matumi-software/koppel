//! What the role tests share: a Koppel started in this process from a temporary data folder, on a
//! loopback port, with a fake kernel for USB, and an HTTPS client that signs in to its UI as a
//! browser does.

#![allow(dead_code)]

use std::collections::HashSet;
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream as StdTcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use koppel_config::{KoppelPaths, SecretBox};
use koppel_usb::{
    BoxFuture, BusId, Capabilities, DeviceStream, Export, ExportStatus, GatewayOptions, Platform,
    UsbAttacher, UsbDevice, UsbError, UsbHost, UsbInterface, UsbSpeed, VhciPort,
};
use koppel_web::{App, AppOptions, Host, NetworkOptions, UiReach, UsbOptions};
use reqwest::Method;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const PASSWORD: &str = "role-tests-password-1";

/// How long a test waits for something that happens in the background.
pub const WAIT: Duration = Duration::from_secs(60);

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A folder of its own under the system's temp folder, removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "koppel-roles-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The dongle the tests share: a CodeMeter stick on 1-1.4.
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
        driver: None,
        ..UsbDevice::new("1-1.4".parse().unwrap())
    }
}

/// Stands in for the Linux kernel on both ends. As a server it shares by binding to usbip-host and
/// echoes whatever arrives on an exported device, as usbip-host answers URBs. As a client it keeps
/// each attached socket for the test, which plays the USB driver.
pub struct FakeKernel {
    bound: Mutex<HashSet<String>>,
    used: Arc<AtomicBool>,
    attached: Mutex<Vec<StdTcpStream>>,
}

impl FakeKernel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            bound: Mutex::new(HashSet::new()),
            used: Arc::new(AtomicBool::new(false)),
            attached: Mutex::new(Vec::new()),
        })
    }

    pub fn attaches(&self) -> usize {
        lock(&self.attached).len()
    }

    /// The `n`th attached socket (from 1), once there is one, as the USB driver holds it.
    pub async fn driver_stream(&self, n: usize) -> TcpStream {
        eventually("the device to attach in the kernel", || async {
            (self.attaches() >= n).then_some(())
        })
        .await;
        let socket = lock(&self.attached)[n - 1].try_clone().unwrap();
        socket.set_nonblocking(true).unwrap();
        TcpStream::from_std(socket).unwrap()
    }
}

impl UsbHost for FakeKernel {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            reset: false,
            ..Capabilities::KERNEL
        }
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        let bound = lock(&self.bound).contains("1-1.4");
        Box::pin(async move {
            Ok(vec![UsbDevice {
                driver: Some(if bound { "usbip-host" } else { "ftdi_sio" }.into()),
                ..dongle()
            }])
        })
    }

    fn is_shared(&self, device: &UsbDevice) -> bool {
        device.driver.as_deref() == Some("usbip-host")
    }

    fn sharing_advice(&self, _: &BusId, _: bool) -> String {
        "Koppel shares devices itself.".into()
    }

    fn share<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        lock(&self.bound).insert(bus_id.to_string());
        Box::pin(async { Ok(()) })
    }

    fn unshare<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        lock(&self.bound).remove(bus_id.as_str());
        Box::pin(async { Ok(()) })
    }

    fn status<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        let status = if self.used.load(Ordering::SeqCst) {
            ExportStatus::Used
        } else {
            ExportStatus::Available
        };
        Box::pin(async move { Ok(status) })
    }

    fn reset<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async { Ok(()) })
    }

    fn export<'a>(&'a self, _: &'a UsbDevice) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        self.used.store(true, Ordering::SeqCst);
        let used = self.used.clone();
        Box::pin(async move { Ok(Box::new(EchoExport { used }) as Box<dyn Export>) })
    }
}

/// Echoes what arrives and frees the device when done.
struct EchoExport {
    used: Arc<AtomicBool>,
}

impl Drop for EchoExport {
    fn drop(&mut self) {
        self.used.store(false, Ordering::SeqCst);
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
        lock(&self.attached).push(kept);
        Ok(8)
    }

    fn detach(&self, _: u32) -> Result<(), UsbError> {
        if let Some(socket) = lock(&self.attached).last() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        Ok(())
    }
}

/// Writes bytes as the USB driver would and reads them back from the server's echo.
pub async fn echoes(driver: &mut TcpStream, text: &[u8]) {
    driver.write_all(text).await.unwrap();
    let mut back = vec![0u8; text.len()];
    tokio::time::timeout(WAIT, driver.read_exact(&mut back))
        .await
        .expect("the echo came back in time")
        .unwrap();
    assert_eq!(back, text);
}

/// Polls `check` until it gives something, for up to [`WAIT`].
pub async fn eventually<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let until = Instant::now() + WAIT;
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(Instant::now() < until, "{what} did not happen in time");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A Koppel running in this process.
pub struct Node {
    pub port: u16,
    dir: PathBuf,
    pub kernel: Arc<FakeKernel>,
    pub app: Arc<App>,
    stop: watch::Sender<bool>,
    serving: JoinHandle<()>,
    http: reqwest::Client,
    cookie: String,
}

/// A free UDP port on loopback.
pub fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A free TCP port on loopback.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Node {
    /// Starts a Koppel in `dir` with `role` (`server`, `client` or `both`), on `port` or a free one.
    pub async fn start(dir: &Path, role: &str, port: Option<u16>, kernel: Arc<FakeKernel>) -> Node {
        Self::start_with(dir, role, port, kernel, None).await
    }

    /// As [`Node::start`], with discovery on UDP `discovery.0` on loopback, a client sending its
    /// probes to `discovery.1`.
    pub async fn start_with(
        dir: &Path,
        role: &str,
        port: Option<u16>,
        kernel: Arc<FakeKernel>,
        discovery: Option<(u16, SocketAddr)>,
    ) -> Node {
        let config = dir.join("config");
        let keys = dir.join("keys");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        let settings = config.join("settings.json");
        if !settings.exists() {
            std::fs::write(
                &settings,
                json!({ "role": role, "shareOnStartup": true, "setupComplete": true }).to_string(),
            )
            .unwrap();
        }
        let paths = KoppelPaths::new(&config, &keys);
        let secrets = Arc::new(SecretBox::open(&keys).unwrap());
        let port = port.unwrap_or_else(free_port);
        let listener = koppel_web::bind(Some("127.0.0.1".parse().unwrap()), port).unwrap();
        let usb = UsbOptions {
            platform: {
                let mut platform = Platform::unsupported("No driver in the tests.");
                platform.host = kernel.clone();
                platform.attacher = Some(kernel.clone());
                platform
            },
            client: koppel_usb::ClientOptions {
                attach_timeout: Duration::from_secs(20),
                retry_interval: Duration::from_millis(500),
            },
            gateway: GatewayOptions {
                port: 0,
                ..GatewayOptions::default()
            },
        };
        let app = App::start(AppOptions {
            paths,
            secrets,
            host: Host {
                name: "bench".into(),
                version: "0.2.0".into(),
                started_at: time::OffsetDateTime::now_utc(),
                ui_port: port,
                network_port: port,
                tray: false,
                os: "test".into(),
            },
            reach: UiReach::resolve(Some("local"), false).unwrap(),
            setup_window: Duration::from_secs(600),
            auth_per_minute: 1000,
            network: NetworkOptions {
                address: Some("127.0.0.1".parse().unwrap()),
                discovery: discovery.is_some(),
                discovery_port: discovery.map_or(port, |d| d.0),
                probe_targets: discovery.map(|d| vec![d.1]),
                ..NetworkOptions::defaults(port)
            },
            usb,
        })
        .await
        .expect("Koppel starts");
        let acceptor = app.tls();
        let (stop, mut stopped) = watch::channel(false);
        let serving = tokio::spawn(koppel_web::serve(
            listener,
            acceptor,
            app.router(),
            async move {
                let _ = stopped.changed().await;
            },
        ));
        let mut node = Node {
            port,
            dir: dir.to_path_buf(),
            kernel,
            app,
            stop,
            serving,
            http: http_client(),
            cookie: String::new(),
        };
        node.sign_in().await;
        node
    }

    /// Stops it as Koppel stops: the ports first, then everything behind them.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(10), self.serving).await;
        self.app.stop().await;
    }

    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    /// The session cookie as a `Cookie` header's value, for a test that calls the routers itself.
    pub fn cookie(&self) -> &str {
        &self.cookie
    }

    async fn sign_in(&mut self) {
        let setup = self
            .http
            .post(format!("{}/api/v1/auth/setup", self.url()))
            .header("X-Koppel-UI", "1")
            .json(&json!({ "password": PASSWORD }))
            .send()
            .await
            .unwrap();
        let answer = if setup.status().as_u16() == 409 {
            self.http
                .post(format!("{}/api/v1/auth/login", self.url()))
                .header("X-Koppel-UI", "1")
                .json(&json!({ "password": PASSWORD }))
                .send()
                .await
                .unwrap()
        } else {
            setup
        };
        assert!(
            answer.status().is_success(),
            "signed in: {}",
            answer.status()
        );
        let cookie = answer
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("__Host-koppel-session="))
            .expect("a session cookie");
        self.cookie = cookie.split(';').next().unwrap().to_owned();
    }

    /// A UI call, signed in, and its status and JSON.
    pub async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.url()))
            .header("Cookie", &self.cookie)
            .header("X-Koppel-UI", "1");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let answer = request.send().await.unwrap();
        let status = answer.status().as_u16();
        let text = answer.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn get(&self, path: &str) -> Value {
        let (status, body) = self.call(Method::GET, path, None).await;
        assert_eq!(status, 200, "GET {path}: {body}");
        body
    }

    pub async fn post(&self, path: &str, body: Value) -> u16 {
        self.call(Method::POST, path, Some(body)).await.0
    }

    pub async fn put(&self, path: &str, body: Value) -> u16 {
        self.call(Method::PUT, path, Some(body)).await.0
    }

    pub fn data(&self) -> &Path {
        &self.dir
    }
}

/// An HTTPS client that takes any certificate: the tests check fingerprints through the API.
fn http_client() -> reqwest::Client {
    http_client_from(None)
}

/// As the tests' HTTPS client, with its connections made from the local address `from`, such as
/// another loopback address than 127.0.0.1, to be another caller.
pub fn http_client_from(from: Option<IpAddr>) -> reqwest::Client {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .no_proxy()
        .pool_max_idle_per_host(0)
        .local_address(from)
        .build()
        .unwrap()
}

#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Opens `server`'s pairing window, as a person at the server does, and waits until it shows as
/// open. Skakel 0.3.0 takes no pairing request before that.
pub async fn open_pairing(server: &Node) {
    let status = server
        .put("/api/v1/server/pairing/window", json!({ "minutes": 5 }))
        .await;
    assert_eq!(status, 202);
    eventually("the pairing window to open", || async {
        (server.get("/api/v1/server/pairing").await["window"]["open"] == json!(true)).then_some(())
    })
    .await;
}

/// Pairs `client` with `server` through the code, as a person would, and waits for the link.
/// Returns the client's ID on the server.
pub async fn pair(server: &Node, client: &Node) -> String {
    open_pairing(server).await;
    let fingerprint = server.get("/api/v1/server/identity").await["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    let status = client
        .post(
            "/api/v1/client/pairing",
            json!({ "serverUrl": server.url(), "serverFingerprint": fingerprint, "serverName": "bench" }),
        )
        .await;
    assert_eq!(status, 202);
    let code = eventually("the client to show a code", || async {
        let pairing = client.get("/api/v1/client/pairing").await;
        assert_ne!(pairing["state"], json!("failed"), "{pairing}");
        pairing["code"].as_str().map(str::to_owned)
    })
    .await;
    let request = eventually("the server to list the request", || async {
        server.get("/api/v1/server/pairing").await["requests"][0]["id"]
            .as_str()
            .map(str::to_owned)
    })
    .await;
    let approved = server
        .post(
            &format!("/api/v1/server/pairing/requests/{request}/approval"),
            json!({ "code": code }),
        )
        .await;
    assert_eq!(approved, 202);
    eventually("the client to be paired", || async {
        (client.get("/api/v1/client/pairing").await["state"] == json!("paired")).then_some(())
    })
    .await;
    let id = client.get("/api/v1/client/connection").await["clientId"]
        .as_str()
        .unwrap()
        .to_owned();
    linked(client).await;
    id
}

/// Waits for the client's link to be up, and gives its devices.
pub async fn linked(client: &Node) -> Value {
    eventually("the client to link", || async {
        let devices = client.get("/api/v1/client/devices").await;
        (devices["link"] == json!("connected")).then_some(devices)
    })
    .await
}

/// Waits for the client's link to be down with a reason, and gives it.
pub async fn down(client: &Node) -> String {
    eventually("the client's link to go down", || async {
        let devices = client.get("/api/v1/client/devices").await;
        (devices["link"] == json!("error")).then(|| {
            devices["linkMessage"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
    })
    .await
}

/// A socket address on loopback.
pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

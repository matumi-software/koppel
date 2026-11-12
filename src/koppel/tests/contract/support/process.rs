//! Starting the real `koppel` program for a test, on a free port, with a temporary config and keys
//! folder, and stopping it when the test is done.
//!
//! The program is the one cargo builds for this package (`CARGO_BIN_EXE_koppel`), so a test run
//! checks the code in the working tree. It is started as any user would start it, from environment
//! settings, and driven over HTTPS only.

use std::fs::File;
#[cfg(target_os = "linux")]
use std::net::{IpAddr, UdpSocket};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::Target;

/// The program under test.
const PROGRAM: &str = env!("CARGO_BIN_EXE_koppel");

/// How long a start gets before the test gives up.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a graceful stop gets before the program is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);

/// What an instance starts with, besides the folders and the port.
#[derive(Clone, Debug, Default)]
pub struct Options {
    settings: Vec<(String, String)>,
    network_port: bool,
    usb_device: bool,
}

impl Options {
    /// One more setting in the environment, such as `("KOPPEL_UI_ALLOW_FROM", "local")`.
    pub fn setting(mut self, name: &str, value: impl Into<String>) -> Options {
        self.settings.push((name.to_owned(), value.into()));
        self
    }

    /// Gives the instance one USB device it can share, in a folder that stands in for the kernel's
    /// sysfs (see [`Instance::spawn`]). Without it the instance has no USB/IP support, as on a
    /// machine without the `usbip-host` module.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn usb_device(mut self) -> Options {
        self.usb_device = true;
        self
    }

    /// Serves the network surface on a port of its own, apart from the UI's.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn network_port(mut self) -> Options {
        self.network_port = true;
        self
    }
}

/// The folder an instance keeps its config, its keys and its log in. Removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "koppel-contract-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary folder");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A free port: asks the system for one and lets go of it. Another process can take it before the
/// program binds it, so a start that loses that race tries again with another.
fn free_port() -> u16 {
    let listener = TcpListener::bind(("::", 0))
        .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))
        .expect("a free port");
    listener.local_addr().expect("an address").port()
}

/// A running `koppel`. Dropping it stops the program and removes its folders.
pub struct Instance {
    // The child first: it must be gone before its folder is removed.
    child: Option<Child>,
    dir: TempDir,
    options: Options,
    ui_port: u16,
    network_port: Option<u16>,
    pub target: Target,
}

/// What `koppel` did when it did not start.
#[derive(Debug)]
pub struct Started {
    /// The exit code, `None` when a signal ended it.
    pub code: Option<i32>,
    /// What it logged.
    pub log: String,
}

impl Instance {
    /// Starts a program and waits until it answers. Fails the test when it does not.
    pub async fn start(options: Options) -> Instance {
        match Instance::try_start(options).await {
            Ok(instance) => instance,
            Err(failed) => panic!(
                "koppel did not start (exit code {:?}). It logged:\n{}",
                failed.code, failed.log
            ),
        }
    }

    /// Starts a program and waits until it answers, or says how it stopped. A start that loses a race
    /// for its port is tried again with another.
    pub async fn try_start(options: Options) -> Result<Instance, Started> {
        let mut instance = Instance::unstarted(options);
        let mut failure = None;
        for _ in 0..5 {
            match instance.spawn().await {
                Ok(()) => return Ok(instance),
                Err(failed) if failed.log.contains("could not listen on port") => {
                    failure = Some(failed);
                    instance.choose_ports();
                }
                Err(failed) => return Err(failed),
            }
        }
        Err(failure.expect("at least one try"))
    }

    /// An instance with its folders and ports, not started. For a test that sets the folders up
    /// first, then calls [`Instance::spawn`].
    pub fn unstarted(options: Options) -> Instance {
        let mut instance = Instance {
            child: None,
            dir: TempDir::new(),
            ui_port: 0,
            network_port: None,
            target: Target::new("https://localhost"),
            options,
        };
        instance.choose_ports();
        instance
    }

    fn choose_ports(&mut self) {
        self.ui_port = free_port();
        self.network_port = self.options.network_port.then(free_port);
        self.target = Target::new(&format!("https://localhost:{}", self.ui_port));
    }

    /// The folder the instance keeps everything in, config and keys among it.
    pub fn root(&self) -> &Path {
        &self.dir.0
    }

    /// The UI port.
    pub fn port(&self) -> u16 {
        self.ui_port
    }

    /// The network port when it is apart from the UI's.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn network_port(&self) -> Option<u16> {
        self.network_port
    }

    /// The folder of `settings.json` and the other config files.
    pub fn config_dir(&self) -> PathBuf {
        self.dir.0.join("config")
    }

    /// The folder of the key ring and the server certificate.
    pub fn keys_dir(&self) -> PathBuf {
        self.dir.0.join("keys")
    }

    /// Everything the program logged since its last start.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.0.join("koppel.log")).unwrap_or_default()
    }

    /// Stops the program as a service manager would, and waits for it to end. Returns its exit code.
    pub fn stop(&mut self) -> Option<i32> {
        let mut child = self.child.take()?;
        terminate(&mut child);
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = child.kill();
                    return child.wait().ok().and_then(|status| status.code());
                }
            }
        }
    }

    /// Stops the program and starts it again on the same port and folders.
    pub async fn restart(&mut self) {
        self.stop();
        self.spawn()
            .await
            .unwrap_or_else(|f| panic!("koppel did not start again:\n{}", f.log));
    }

    /// Starts the program on the folders and ports this instance has.
    pub async fn spawn(&mut self) -> Result<(), Started> {
        let root = &self.dir.0;
        let log = File::create(root.join("koppel.log")).expect("a log file");
        let sys = root.join("sys");
        if self.options.usb_device {
            fake_sysfs(&sys);
        }
        let mut command = Command::new(PROGRAM);
        command
            .env("KOPPEL_CONFIG_DIR", root.join("config"))
            .env("KOPPEL_KEYS_DIR", root.join("keys"))
            .env("KOPPEL_UI_PORT", self.ui_port.to_string())
            // Never the machine's own /sys: a test must not touch the USB devices of whoever runs it.
            .env("KOPPEL_USB_SYS_ROOT", &sys)
            .env("KOPPEL_RATE_AUTH_PER_MINUTE", "1000")
            // This machine only, so the first password needs no setup token (Linux asks for one when
            // the UI is open beyond this machine). A test of the token sets `lan` itself.
            .env("KOPPEL_UI_ALLOW_FROM", "local")
            // The USB/IP gateway listens on a port of its own, which two instances cannot share.
            .env("KOPPEL_GATEWAY_PORT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().expect("a second handle")))
            .stderr(Stdio::from(log));
        if let Some(port) = self.network_port {
            command.env("KOPPEL_NETWORK_PORT", port.to_string());
        }
        // On Windows an instance listens on this machine only, with discovery off, so no firewall
        // prompt comes up.
        if cfg!(windows) {
            command
                .env("KOPPEL_LISTEN_ADDRESS", "127.0.0.1")
                .env("KOPPEL_DISCOVERY_ENABLED", "false");
        }
        for (name, value) in &self.options.settings {
            command.env(name, value);
        }
        self.child = Some(command.spawn().expect("the koppel program starts"));

        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let child = self.child.as_mut().expect("a child");
            if let Ok(Some(status)) = child.try_wait() {
                self.child = None;
                return Err(Started {
                    code: status.code(),
                    log: self.log(),
                });
            }
            if listening(self.ui_port) {
                break;
            }
            if Instant::now() > deadline {
                let log = self.log();
                self.stop();
                panic!(
                    "koppel did not listen on port {} in time:\n{log}",
                    self.ui_port
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn listening(port: u16) -> bool {
    ("localhost", port)
        .to_socket_addrs()
        .map(|addresses| {
            addresses
                .into_iter()
                .any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(1)).is_ok())
        })
        .unwrap_or(false)
}

/// SIGTERM on Linux, as systemd and Docker stop it; elsewhere the program is killed.
#[cfg(unix)]
fn terminate(child: &mut Child) {
    let sent = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .is_ok_and(|status| status.success());
    if !sent {
        let _ = child.kill();
    }
}

#[cfg(not(unix))]
fn terminate(child: &mut Child) {
    let _ = child.kill();
}

/// This machine's own address on its network, the one a connection from here comes from. `None`
/// without a route.
#[cfg(target_os = "linux")]
pub fn own_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // No packet goes out: connecting a datagram socket only picks the route.
    socket.connect("192.0.2.1:9").ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_unspecified() && !address.is_loopback()).then_some(address)
}

/// A folder that looks enough like sysfs for Koppel to share one device: the `usbip-host` driver
/// with the files a share writes to (plain files here, so a write always works), and device `1-1`.
fn fake_sysfs(sys: &Path) {
    let usb = sys.join("bus").join("usb");
    let driver = usb.join("drivers").join("usbip-host");
    let device = usb.join("devices").join("1-1");
    for folder in [&driver, &device] {
        std::fs::create_dir_all(folder).expect("a folder in the fake sysfs");
    }
    for file in [
        driver.join("match_busid"),
        driver.join("bind"),
        driver.join("unbind"),
        usb.join("drivers_probe"),
    ] {
        std::fs::write(file, "").expect("a file in the fake sysfs");
    }
    // Bound to usbip-host already, which is what a shared device looks like.
    #[cfg(unix)]
    std::os::unix::fs::symlink(Path::new("../../drivers/usbip-host"), device.join("driver"))
        .expect("the driver link");
    for (name, value) in [
        ("idVendor", "1234"),
        ("idProduct", "5678"),
        ("bcdDevice", "0100"),
        ("busnum", "1"),
        ("devnum", "2"),
        ("speed", "480"),
        ("bDeviceClass", "00"),
        ("bNumConfigurations", "1"),
        ("bConfigurationValue", "1"),
        ("manufacturer", "Koppel tests"),
        ("product", "Contract device"),
    ] {
        std::fs::write(
            device.join(name),
            format!(
                "{value}
"
            ),
        )
        .expect("an attribute");
    }
}

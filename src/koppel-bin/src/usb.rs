//! What drives USB on this machine, from the settings 0.1.x read: the Linux kernel's usbip-host and
//! vhci-hcd through sysfs (`Usb:SysRoot`, `/sys` unless set), another USB/IP server to share from
//! instead (`Usb:Upstream`, such as usbipd-win on a Windows host), usbip-win2's `usbip.exe` on
//! Windows (`Usb:UsbipPath`, else the copy the installer bundles, else a standalone install), the
//! attach deadline (`Usb:AttachTimeoutSeconds`) and the gateway (`Gateway:Port`, `Gateway:Address`,
//! `Gateway:MaxSessions`).

use std::sync::Arc;
use std::time::Duration;

use koppel_usb::{
    ClientOptions, GatewayOptions, SystemClock, Unsupported, UpstreamAddress, UsbAttacher, UsbHost,
    UsbIpDriver, UsbIpUpstreamHost,
};
use koppel_web::UsbOptions;

use crate::config::Config;

/// What a platform Koppel cannot drive says.
const UNSUPPORTED: &str = "Sharing and attaching USB devices needs Linux, or a USB/IP server or driver on the host. See docs/windows.md.";

/// The USB side as the settings say.
pub fn options(config: &Config) -> Result<UsbOptions, String> {
    let unsupported = Arc::new(Unsupported::new(UNSUPPORTED));
    let sys_root = config.get("Usb:SysRoot").unwrap_or("/sys").to_owned();

    let host: Arc<dyn UsbHost> = match config.get("Usb:Upstream") {
        Some(upstream) => {
            let address =
                UpstreamAddress::parse(upstream).map_err(|e| format!("Usb:Upstream: {e}"))?;
            Arc::new(UsbIpUpstreamHost::new(
                address.host,
                address.port,
                Arc::new(SystemClock),
            ))
        }
        None => native_host(&sys_root).unwrap_or_else(|| unsupported.clone()),
    };
    let attacher: Arc<dyn UsbAttacher> =
        native_attacher(&sys_root).unwrap_or_else(|| unsupported.clone());
    let driver: Arc<dyn UsbIpDriver> = native_driver(config).unwrap_or(unsupported);

    let attach_seconds = config.count("Usb:AttachTimeoutSeconds", 20)?;
    let gateway = GatewayOptions {
        port: match config.get("Gateway:Port") {
            Some("0") => 0,
            _ => config.port("Gateway:Port", GatewayOptions::default_port())?,
        },
        address: config
            .get("Gateway:Address")
            .unwrap_or("127.0.0.1")
            .trim()
            .to_owned(),
        max_sessions: config.count(
            "Gateway:MaxSessions",
            GatewayOptions::DEFAULT_MAX_SESSIONS as u32,
        )? as usize,
    };
    Ok(UsbOptions {
        host,
        attacher,
        driver,
        client: ClientOptions {
            attach_timeout: Duration::from_secs(u64::from(attach_seconds)),
            ..ClientOptions::default()
        },
        gateway,
    })
}

#[cfg(target_os = "linux")]
fn native_host(sys_root: &str) -> Option<Arc<dyn UsbHost>> {
    Some(Arc::new(koppel_usb::linux::SysfsUsbHost::new(
        sys_root, "/dev",
    )))
}

#[cfg(not(target_os = "linux"))]
fn native_host(_sys_root: &str) -> Option<Arc<dyn UsbHost>> {
    None
}

#[cfg(target_os = "linux")]
fn native_attacher(sys_root: &str) -> Option<Arc<dyn UsbAttacher>> {
    Some(Arc::new(koppel_usb::linux::SysfsVhci::new(sys_root)))
}

#[cfg(not(target_os = "linux"))]
fn native_attacher(_sys_root: &str) -> Option<Arc<dyn UsbAttacher>> {
    None
}

/// usbip-win2's `usbip.exe`: `Usb:UsbipPath`, else the copy beside Koppel, else a standalone
/// install under Program Files.
#[cfg(windows)]
fn native_driver(config: &Config) -> Option<Arc<dyn UsbIpDriver>> {
    let path = match config.get("Usb:UsbipPath") {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let bundled = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("usbip").join("usbip.exe")));
            match bundled {
                Some(bundled) if bundled.is_file() => bundled,
                _ => std::path::PathBuf::from(
                    std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into()),
                )
                .join("USBip")
                .join("usbip.exe"),
            }
        }
    };
    Some(Arc::new(koppel_usb::windows::UsbipWin2Driver::new(path)))
}

#[cfg(not(windows))]
fn native_driver(_config: &Config) -> Option<Arc<dyn UsbIpDriver>> {
    None
}

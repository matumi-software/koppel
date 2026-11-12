//! What drives USB on this machine, from the settings: the Linux kernel's usbip-host and
//! vhci-hcd through sysfs (`KOPPEL_USB_SYS_ROOT`, `/sys` unless set), another USB/IP server to share from
//! instead (`KOPPEL_USB_UPSTREAM`, such as usbipd-win on a Windows host), usbip-win2's `usbip.exe` on
//! Windows (`KOPPEL_USB_USBIP_PATH`, else the copy the installer bundles, else a standalone install), the
//! attach deadline (`KOPPEL_USB_ATTACH_TIMEOUT_SECONDS`) and the gateway (`KOPPEL_GATEWAY_PORT`,
//! `KOPPEL_GATEWAY_ADDRESS`, `KOPPEL_GATEWAY_MAX_SESSIONS`).

use std::sync::Arc;
use std::time::Duration;

use koppel_usb::{
    ClientOptions, GatewayOptions, Platform, SystemClock, UpstreamAddress, UsbAttacher, UsbHost,
    UsbIpDriver, UsbIpUpstreamHost,
};
use koppel_web::UsbOptions;

use crate::config::Settings;

/// What a platform Koppel cannot drive says.
const UNSUPPORTED: &str = "Sharing and attaching USB devices needs Linux, or a USB/IP server or driver on the host. See docs/windows.md.";

/// The USB side as the settings say.
pub fn options(settings: &Settings) -> Result<UsbOptions, String> {
    let sys_root = settings.usb_sys_root().to_owned();

    let host: Arc<dyn UsbHost> = match settings.usb_upstream() {
        Some(upstream) => {
            let address = UpstreamAddress::parse(upstream)
                .map_err(|e| format!("KOPPEL_USB_UPSTREAM: {e}"))?;
            Arc::new(UsbIpUpstreamHost::new(
                address.host,
                address.port,
                Arc::new(SystemClock),
            ))
        }
        None => native_host(&sys_root).unwrap_or_else(|| Platform::unsupported(UNSUPPORTED).host),
    };
    let platform = Platform::new(
        host,
        native_attacher(&sys_root),
        native_driver(settings),
        UNSUPPORTED,
    );

    let gateway = GatewayOptions {
        port: settings
            .gateway_port()
            .unwrap_or_else(GatewayOptions::default_port),
        address: settings.gateway_address().to_owned(),
        max_sessions: settings
            .gateway_max_sessions()
            .map_or(GatewayOptions::DEFAULT_MAX_SESSIONS, |n| n as usize),
    };
    Ok(UsbOptions {
        platform,
        client: ClientOptions {
            attach_timeout: Duration::from_secs(u64::from(settings.usb_attach_timeout_seconds())),
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

/// usbip-win2's `usbip.exe`: `KOPPEL_USB_USBIP_PATH`, else the copy beside Koppel, else a standalone
/// install under Program Files.
#[cfg(windows)]
fn native_driver(settings: &Settings) -> Option<Arc<dyn UsbIpDriver>> {
    let path = match settings.usb_usbip_path() {
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
fn native_driver(_settings: &Settings) -> Option<Arc<dyn UsbIpDriver>> {
    None
}

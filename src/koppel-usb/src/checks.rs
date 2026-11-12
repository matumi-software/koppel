//! The USB lines of the system checks the UI shows: whether this machine can share and attach.

use serde::Serialize;

use crate::{HostKind, Platform};

/// One line of the system checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemCheck {
    pub id: String,
    pub ok: bool,
    pub title: String,
    pub detail: String,
}

impl SystemCheck {
    pub fn new(id: &str, ok: bool, title: &str, detail: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            ok,
            title: title.to_owned(),
            detail: detail.into(),
        }
    }
}

/// On Linux the host is usbip-host. On native Windows Koppel cannot share by itself: a server
/// there is usbipd-win reached through `Usb:Upstream`, which the upstream host reports on.
pub fn share_check(platform: &Platform) -> SystemCheck {
    let (host, native_windows) = (&platform.host, platform.native_windows);
    let kind = host.capabilities().kind;
    if native_windows && kind == HostKind::Unsupported {
        return SystemCheck::new(
            "share",
            false,
            "Sharing needs usbipd-win.",
            "A Windows server shares through usbipd-win. Install it and set Usb:Upstream to its address. See docs/windows.md.",
        );
    }
    if let Some(problem) = host.unavailable() {
        return SystemCheck::new("share", false, "Sharing is not available.", problem);
    }
    if native_windows && kind == HostKind::Upstream {
        return SystemCheck::new(
            "share",
            true,
            "usbipd-win is reachable.",
            "This machine shares the devices of the USB/IP server in Usb:Upstream.",
        );
    }
    SystemCheck::new(
        "share",
        true,
        "usbip-host is loaded.",
        "This machine can share devices.",
    )
}

/// On Linux the kernel's vhci-hcd attaches. On native Windows it is usbip-win2's driver.
pub fn attach_check(platform: &Platform) -> SystemCheck {
    if platform.native_windows {
        return match platform.driver_unavailable() {
            Some(problem) => SystemCheck::new(
                "attach",
                false,
                "The usbip-win2 driver is not installed.",
                problem,
            ),
            None => SystemCheck::new(
                "attach",
                true,
                "The usbip-win2 driver is installed.",
                "This machine can attach devices.",
            ),
        };
    }
    match platform.attach_unavailable() {
        Some(problem) => SystemCheck::new("attach", false, "Attaching is not available.", problem),
        None => SystemCheck::new(
            "attach",
            true,
            "vhci-hcd is loaded.",
            "This machine can attach devices.",
        ),
    }
}

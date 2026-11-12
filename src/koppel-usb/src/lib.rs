//! Koppel's USB layer: USB/IP as the Linux kernel documents it, the drivers that speak it on
//! each platform, and the server and client roles that carry it to the other machine.
//!
//! - [`protocol`]: the USB/IP device list and import messages, byte for byte as the Linux
//!   kernel's usbip-host and vhci-hcd, usbip-win2 and usbipd-win expect them.
//! - [`UsbHost`], [`UsbAttacher`] and [`UsbIpDriver`]: what a platform can do. The Linux kernel
//!   (`linux`, through sysfs), a plain USB/IP server such as usbipd-win ([`UsbIpUpstreamHost`]),
//!   and usbip-win2's `usbip.exe` on Windows (`windows`).
//! - [`DeviceStream`] and [`StreamOpener`]: the byte stream to the other machine for one device.
//!   The transport (Skakel) implements them; this crate never names it.
//! - [`UsbServer`] and [`UsbDataPlane`]: the server role. What is shared, the device list, and
//!   serving a client's stream: a device list or an import followed by the device's traffic.
//! - [`UsbClient`] and [`UsbIpGateway`]: the client role. Attaching through vhci-hcd or through
//!   usbip-win2 and the gateway, attaching again after a drop, and `attached.json`.

pub mod activity;
pub mod bus_id;
mod checks;
mod client;
mod data_plane;
mod device;
mod error;
mod gateway;
mod host;
pub mod peer_check;
pub mod protocol;
mod relay;
mod server;
pub mod stream;
mod tasks;
mod upstream;
mod views;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use time::OffsetDateTime;

pub use activity::{
    ActivityOutcome, ActivityReason, ActivitySink, EntityKind, EntityRef, NoActivity,
};
pub use bus_id::BusId;
pub use checks::{SystemCheck, attach_check, share_check};
pub use client::{
    BeginAttachError, ClientOptions, Deps, ImportMismatch, Link, LinkEvents, LinkStatus, Links,
    UsbClient, check_import,
};
pub use data_plane::{RemoteClient, UsbDataPlane};
pub use device::{ExportStatus, Hub, PortStatus, UsbDevice, UsbInterface, UsbSpeed, VhciPort};
pub use error::{Module, UsbError};
pub use gateway::{GatewayOptions, UsbIpGateway};
pub use host::{Capabilities, Export, HostKind, Platform, UsbAttacher, UsbHost, UsbIpDriver};
pub use relay::{loopback_pair, relay};
pub use server::{ServerDeps, SetSharedError, UsbServer, shared_expects};
pub use stream::{BoxFuture, CloseReason, DeviceStream, OpenError, StreamOpener};
pub use tasks::Listener;
pub use upstream::{UpstreamAddress, UsbIpUpstreamHost};
pub use views::{
    ClientDeviceView, ClientDevices, DeviceState, DeviceView, LinkState, RemoteDevices,
    ServerDevices, interface_class_name, speed_label,
};

/// Where the time comes from, so a test can move it by hand.
pub trait Clock: Send + Sync {
    /// The wall clock, for what people read: when a device was attached.
    fn now(&self) -> OffsetDateTime;

    /// A monotonic reading, for what is measured: when to look again, when to try again. A step of
    /// the wall clock never stalls or hurries it.
    fn monotonic(&self) -> tokio::time::Instant {
        tokio::time::Instant::now()
    }
}

/// The system's clock, in UTC.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

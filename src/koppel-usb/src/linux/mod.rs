//! The Linux kernel's USB/IP drivers, driven through sysfs the way the `usbip` tools drive them:
//! `usbip-host` shares devices on a server, `vhci-hcd` attaches them on a client. Both need root
//! and their kernel module.

mod sysfs_host;
mod usbfs;
mod vhci;

pub use sysfs_host::SysfsUsbHost;
pub use vhci::SysfsVhci;

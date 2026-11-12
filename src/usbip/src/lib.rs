//! USB/IP as the Linux kernel documents it (`Documentation/usb/usbip_protocol.rst`), with no
//! knowledge of Koppel: the device model, the bus ID, the operation messages that list and import
//! a device, and the relay that carries the device's traffic after an import.
//!
//! - [`protocol`]: the device list and import messages, byte for byte as the Linux kernel's
//!   usbip-host and vhci-hcd, usbip-win2 and usbipd-win expect them.
//! - [`BusId`]: a bus ID that can only be made from text that is one.
//! - [`UsbDevice`] and the types around it: a device as the host sees it, and the kernel's export
//!   and virtual port states.
//! - [`relay()`]: copies both directions of a connection, one write and one flush per read.
//!
//! The properties of the messages are checked with proptest in this crate; a failing case is saved
//! in `proptest-regressions/`.

pub mod bus_id;
mod device;
pub mod protocol;
mod relay;

pub use bus_id::{BusId, InvalidBusId};
pub use device::{ExportStatus, Hub, PortStatus, UsbDevice, UsbInterface, UsbSpeed, VhciPort};
pub use relay::relay;

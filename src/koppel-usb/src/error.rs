use std::io;

use crate::BusId;
use crate::bus_id::InvalidBusId;

/// The kernel module a USB operation needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    /// `usbip-host`, which shares devices.
    UsbipHost,
    /// `vhci-hcd`, which attaches them.
    VhciHcd,
}

impl Module {
    pub fn name(self) -> &'static str {
        match self {
            Self::UsbipHost => "usbip-host",
            Self::VhciHcd => "vhci-hcd",
        }
    }
}

/// A USB operation that did not work. `Display` is the sentence the UI shows, as 0.1.x's
/// `UsbHostException` message was; the source of an I/O failure stays for the log.
#[derive(Debug, thiserror::Error)]
pub enum UsbError {
    #[error(
        "The {} kernel module is not loaded. Run 'modprobe {}' on the host (add it to /etc/modules to keep it).",
        .0.name(), .0.name()
    )]
    ModuleMissing(Module),
    #[error("That is not a USB bus ID.")]
    InvalidBusId,
    #[error("Device {0} is not available to export. It may be in use by another client.")]
    Busy(BusId),
    #[error("{0} is already being used through this server.")]
    Claimed(BusId),
    #[error(
        "A different device is plugged into this port now, so it is not shared. Share it again to use it."
    )]
    WrongDevice,
    #[error("All virtual USB ports are in use. Detach a device first.")]
    NoFreePort,
    /// A file, a socket or a command failed. `op` says what was being done.
    #[error("{op}: {source}")]
    Io {
        op: String,
        #[source]
        source: io::Error,
    },
    #[error("{0}")]
    Timeout(String),
    #[error("The operation was cancelled.")]
    Cancelled,
    /// The host or driver cannot be used here; the text says why, for the UI.
    #[error("{0}")]
    Unavailable(String),
    /// Anything else the kernel, a tool or the other server said no to, in words for the UI.
    #[error("{0}")]
    Failed(String),
}

impl UsbError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    pub fn io(op: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            op: op.into(),
            source,
        }
    }
}

impl From<InvalidBusId> for UsbError {
    fn from(_: InvalidBusId) -> Self {
        Self::InvalidBusId
    }
}

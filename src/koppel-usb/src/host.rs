use std::net::TcpStream;

use tokio_util::sync::CancellationToken;

use crate::{BoxFuture, DeviceStream, ExportStatus, UsbDevice, UsbError, UsbSpeed, VhciPort};

/// Which kind of host this is, for the system checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// This machine's kernel shares the devices (usbip-host on Linux).
    Kernel,
    /// A plain USB/IP server next to Koppel shares them, such as usbipd-win.
    Upstream,
    /// Nothing here can share devices.
    Unsupported,
}

/// The server side of USB/IP on this machine: list devices and hand them to the network. What a
/// host can do differs (the Linux kernel shares devices itself; usbipd-win on another machine
/// shares them by its own command), so the trait says what each host can do and callers check
/// before they ask.
pub trait UsbHost: Send + Sync {
    /// `None` when this machine can share USB devices; otherwise why not, for the UI.
    fn unavailable(&self) -> Option<String>;

    /// True when [`UsbHost::share`] works: Koppel itself decides which devices are shared.
    fn can_share(&self) -> bool;

    /// True when [`UsbHost::unshare`] works.
    fn can_unshare(&self) -> bool;

    /// True when [`UsbHost::reset`] does something.
    fn can_reset(&self) -> bool;

    fn kind(&self) -> HostKind {
        HostKind::Kernel
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>>;

    /// Whether a listed device is ready to export. The Linux kernel says so by binding it to
    /// usbip-host.
    fn is_shared(&self, device: &UsbDevice) -> bool;

    /// What to tell the user when this host cannot share or unshare a device itself, such as the
    /// command to run on the machine that does.
    fn sharing_advice(&self, bus_id: &str, share: bool) -> String;

    /// Takes the device from its driver and gives it to usbip-host, so it can be exported. Only
    /// where [`UsbHost::can_share`].
    fn share<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>>;

    /// Gives the device back to its normal driver. Only where [`UsbHost::can_unshare`].
    fn unshare<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>>;

    fn status<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<ExportStatus, UsbError>>;

    /// Resets a shared device on its USB port, so a client gets it the way a fresh plug-in would.
    /// A driver on this machine may have changed the device before it was shared: Linux's
    /// Logitech receiver driver switches the receiver to a mode only that driver reads, and the
    /// mouse then stays silent on the client. Only where [`UsbHost::can_reset`].
    fn reset<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>>;

    /// Exports the device. When this returns, the host is ready (the kernel holds its socket, or
    /// the upstream server accepted the import) and no USB traffic has flowed yet, which is when
    /// the caller answers the client's import request. Then [`Export::relay`] carries the traffic.
    /// An error means the host cannot export the device, and nothing is left held.
    fn export<'a>(
        &'a self,
        device: &'a UsbDevice,
    ) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>>;
}

/// A device the host has exported, waiting for its traffic. Dropping it without relaying gives
/// the device back.
pub trait Export: Send {
    /// Carries the device's USB traffic between `network` and the device until either side
    /// closes or `stop` is cancelled. The host frees the device when it returns.
    fn relay<'a>(
        self: Box<Self>,
        network: &'a mut dyn DeviceStream,
        stop: CancellationToken,
    ) -> BoxFuture<'a, ()>;
}

/// The client side of USB/IP on this machine: attach remote devices as local ones (vhci-hcd).
pub trait UsbAttacher: Send + Sync {
    /// `None` when this machine can attach USB devices; otherwise why not, for the UI.
    fn unavailable(&self) -> Option<String>;

    fn ports(&self) -> Vec<VhciPort>;

    /// Hands a socket that has already completed OP_REQ_IMPORT to the virtual host controller.
    /// Returns the port used. Keep `socket` open until the device is detached.
    fn attach(&self, socket: &TcpStream, dev_id: u32, speed: UsbSpeed) -> Result<u32, UsbError>;

    fn detach(&self, port: u32) -> Result<(), UsbError>;
}

/// A USB/IP driver that makes its own connection to a USB/IP server, such as usbip-win2 on
/// Windows. Linux takes a socket from Koppel instead ([`UsbAttacher`]). Koppel points this kind
/// of driver at its own gateway on 127.0.0.1, which carries the session to the server.
pub trait UsbIpDriver: Send + Sync {
    /// `None` when the driver is installed; otherwise why not, for the UI.
    fn unavailable(&self) -> Option<String>;

    /// Attaches `bus_id` from the USB/IP server at host:port. Returns the driver's port.
    fn attach<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        bus_id: &'a str,
    ) -> BoxFuture<'a, Result<u32, UsbError>>;

    /// Detaches the device on `driver_port` and stops the driver from attaching it again.
    fn detach<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        bus_id: &'a str,
        driver_port: u32,
    ) -> BoxFuture<'a, Result<(), UsbError>>;
}

/// Stands in where Koppel cannot drive USB.
#[derive(Debug, Clone)]
pub struct Unsupported {
    reason: String,
}

impl Unsupported {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn refuse<T: Send + 'static>(&self) -> BoxFuture<'static, Result<T, UsbError>> {
        let error = UsbError::new(self.reason.clone());
        Box::pin(async move { Err(error) })
    }
}

impl UsbHost for Unsupported {
    fn unavailable(&self) -> Option<String> {
        Some(self.reason.clone())
    }

    fn can_share(&self) -> bool {
        false
    }

    fn can_unshare(&self) -> bool {
        false
    }

    fn can_reset(&self) -> bool {
        false
    }

    fn kind(&self) -> HostKind {
        HostKind::Unsupported
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn is_shared(&self, _: &UsbDevice) -> bool {
        false
    }

    fn sharing_advice(&self, _: &str, _: bool) -> String {
        self.reason.clone()
    }

    fn share<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        self.refuse()
    }

    fn unshare<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        self.refuse()
    }

    fn status<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        Box::pin(async { Ok(ExportStatus::NotExported) })
    }

    fn reset<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async { Ok(()) })
    }

    fn export<'a>(&'a self, _: &'a UsbDevice) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        self.refuse()
    }
}

impl UsbAttacher for Unsupported {
    fn unavailable(&self) -> Option<String> {
        Some(self.reason.clone())
    }

    fn ports(&self) -> Vec<VhciPort> {
        Vec::new()
    }

    fn attach(&self, _: &TcpStream, _: u32, _: UsbSpeed) -> Result<u32, UsbError> {
        Err(UsbError::new(self.reason.clone()))
    }

    fn detach(&self, _: u32) -> Result<(), UsbError> {
        Err(UsbError::new(self.reason.clone()))
    }
}

impl UsbIpDriver for Unsupported {
    fn unavailable(&self) -> Option<String> {
        Some(self.reason.clone())
    }

    fn attach<'a>(
        &'a self,
        _: &'a str,
        _: u16,
        _: &'a str,
    ) -> BoxFuture<'a, Result<u32, UsbError>> {
        self.refuse()
    }

    fn detach<'a>(
        &'a self,
        _: &'a str,
        _: u16,
        _: &'a str,
        _: u32,
    ) -> BoxFuture<'a, Result<(), UsbError>> {
        self.refuse()
    }
}

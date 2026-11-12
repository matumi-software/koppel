use std::net::TcpStream;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::relay::KernelEnd;
use crate::{
    BoxFuture, BusId, DeviceStream, ExportStatus, UsbDevice, UsbError, UsbSpeed, VhciPort,
};

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

/// What a host can do, as a value callers read once and branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub kind: HostKind,
    /// [`UsbHost::share`] works: Koppel itself decides which devices are shared.
    pub share: bool,
    /// [`UsbHost::unshare`] works.
    pub unshare: bool,
    /// [`UsbHost::reset`] does something.
    pub reset: bool,
}

impl Capabilities {
    /// This machine's kernel: Koppel binds, unbinds and resets devices itself.
    pub const KERNEL: Self = Self {
        kind: HostKind::Kernel,
        share: true,
        unshare: true,
        reset: true,
    };

    /// A plain USB/IP server shares by its own tools, so Koppel only lists and exports.
    pub const UPSTREAM: Self = Self {
        kind: HostKind::Upstream,
        share: false,
        unshare: false,
        reset: false,
    };

    /// Nothing here can share devices.
    pub const NONE: Self = Self {
        kind: HostKind::Unsupported,
        share: false,
        unshare: false,
        reset: false,
    };
}

/// The server side of USB/IP on this machine: list devices and hand them to the network. What a
/// host can do differs (the Linux kernel shares devices itself; usbipd-win on another machine
/// shares them by its own command), so [`UsbHost::capabilities`] says what each host can do and
/// callers check before they ask.
pub trait UsbHost: Send + Sync {
    /// `None` when this machine can share USB devices; otherwise why not, for the UI.
    fn unavailable(&self) -> Option<String>;

    fn capabilities(&self) -> Capabilities;

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>>;

    /// Whether a listed device is ready to export. The Linux kernel says so by binding it to
    /// usbip-host.
    fn is_shared(&self, device: &UsbDevice) -> bool;

    /// What to tell the user when this host cannot share or unshare a device itself, such as the
    /// command to run on the machine that does.
    fn sharing_advice(&self, bus_id: &BusId, share: bool) -> String;

    /// Takes the device from its driver and gives it to usbip-host, so it can be exported. Only
    /// where [`Capabilities::share`].
    fn share<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>>;

    /// Gives the device back to its normal driver. Only where [`Capabilities::unshare`].
    fn unshare<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>>;

    fn status<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<ExportStatus, UsbError>>;

    /// Resets a shared device on its USB port, so a client gets it the way a fresh plug-in would.
    /// A driver on this machine may have changed the device before it was shared: Linux's
    /// Logitech receiver driver switches the receiver to a mode only that driver reads, and the
    /// mouse then stays silent on the client. Only where [`Capabilities::reset`].
    fn reset<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>>;

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

    /// For a test attacher that takes one end of a pipe in memory instead of a socket, so the
    /// attach involves no real I/O. Then [`UsbAttacher::attach_in_memory`] is called in place of
    /// [`UsbAttacher::attach`].
    #[cfg(feature = "test-util")]
    fn in_memory(&self) -> bool {
        false
    }

    /// As [`UsbAttacher::attach`], with the end of the pipe, which the attacher keeps until the
    /// device is detached.
    #[cfg(feature = "test-util")]
    fn attach_in_memory(
        &self,
        _end: tokio::io::DuplexStream,
        _dev_id: u32,
        _speed: UsbSpeed,
    ) -> Result<u32, UsbError> {
        Err(UsbError::failed("This attacher takes a socket."))
    }
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
        bus_id: &'a BusId,
    ) -> BoxFuture<'a, Result<u32, UsbError>>;

    /// Detaches the device on `driver_port` and stops the driver from attaching it again.
    fn detach<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        bus_id: &'a BusId,
        driver_port: u32,
    ) -> BoxFuture<'a, Result<(), UsbError>>;
}

/// What drives USB on this machine: the host that shares, and what attaches, as the kernel
/// does or as a driver such as usbip-win2 does. A part this machine lacks is `None`, and the
/// platform says why with one reason.
#[derive(Clone)]
pub struct Platform {
    pub host: Arc<dyn UsbHost>,
    /// Attaches through this kernel's virtual host controller.
    pub attacher: Option<Arc<dyn UsbAttacher>>,
    /// Attaches through a USB/IP driver on this machine.
    pub driver: Option<Arc<dyn UsbIpDriver>>,
    /// This is Windows itself rather than Linux or a container, which changes what the system
    /// checks look at.
    pub native_windows: bool,
    reason: String,
}

impl Platform {
    /// `reason` is what a missing part says.
    pub fn new(
        host: Arc<dyn UsbHost>,
        attacher: Option<Arc<dyn UsbAttacher>>,
        driver: Option<Arc<dyn UsbIpDriver>>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            host,
            attacher,
            driver,
            native_windows: cfg!(windows),
            reason: reason.into(),
        }
    }

    /// A machine Koppel cannot drive: it shares and attaches nothing, and says `reason`.
    pub fn unsupported(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::new(Arc::new(NoHost(reason.clone())), None, None, reason)
    }

    /// For a test that runs as if on another operating system.
    #[must_use]
    pub fn on_windows(mut self, native_windows: bool) -> Self {
        self.native_windows = native_windows;
        self
    }

    /// `None` when this kernel can attach; otherwise why not.
    pub fn attach_unavailable(&self) -> Option<String> {
        match &self.attacher {
            Some(attacher) => attacher.unavailable(),
            None => Some(self.reason.clone()),
        }
    }

    /// `None` when the driver is installed; otherwise why not.
    pub fn driver_unavailable(&self) -> Option<String> {
        match &self.driver {
            Some(driver) => driver.unavailable(),
            None => Some(self.reason.clone()),
        }
    }

    fn missing(&self) -> UsbError {
        UsbError::Unavailable(self.reason.clone())
    }

    pub(crate) fn attacher(&self) -> Result<&Arc<dyn UsbAttacher>, UsbError> {
        self.attacher.as_ref().ok_or_else(|| self.missing())
    }

    pub(crate) fn driver(&self) -> Result<&Arc<dyn UsbIpDriver>, UsbError> {
        self.driver.as_ref().ok_or_else(|| self.missing())
    }

    /// Whether the attacher takes a pipe in memory (a test double only).
    pub(crate) fn attaches_in_memory(&self) -> bool {
        #[cfg(feature = "test-util")]
        {
            self.attacher.as_ref().is_some_and(|a| a.in_memory())
        }
        #[cfg(not(feature = "test-util"))]
        {
            false
        }
    }

    /// Hands the kernel's end to the virtual host controller. The write to sysfs blocks, so it runs
    /// off the async workers; the controller takes its own reference to the socket, which the caller
    /// keeps open until the device is detached.
    pub(crate) async fn attach_kernel(
        &self,
        end: &KernelEnd,
        dev_id: u32,
        speed: UsbSpeed,
    ) -> Result<u32, UsbError> {
        let attacher = self.attacher()?.clone();
        // Without the test-util feature a socket is the only kind of end there is.
        #[allow(clippy::infallible_destructuring_match)]
        let socket = match end {
            KernelEnd::Socket(socket) => socket,
            #[cfg(feature = "test-util")]
            KernelEnd::Memory(_) => {
                let pipe = end
                    .take_memory()
                    .ok_or_else(|| UsbError::failed("The pipe was handed over already."))?;
                return attacher.attach_in_memory(pipe, dev_id, speed);
            }
        };
        let socket = socket
            .try_clone()
            .map_err(|e| UsbError::failed(format!("The socket could not be handed over: {e}")))?;
        tokio::task::spawn_blocking(move || attacher.attach(&socket, dev_id, speed))
            .await
            .map_err(|e| UsbError::failed(format!("The attach did not finish: {e}")))?
    }

    /// Frees a virtual port, off the async workers.
    pub(crate) async fn detach_kernel(&self, port: u32) -> Result<(), UsbError> {
        let attacher = self.attacher()?.clone();
        tokio::task::spawn_blocking(move || attacher.detach(port))
            .await
            .map_err(|e| UsbError::failed(format!("The detach did not finish: {e}")))?
    }
}

/// The host of a machine Koppel cannot drive.
struct NoHost(String);

impl NoHost {
    fn refuse<T: Send + 'static>(&self) -> BoxFuture<'static, Result<T, UsbError>> {
        let error = UsbError::Unavailable(self.0.clone());
        Box::pin(async move { Err(error) })
    }
}

impl UsbHost for NoHost {
    fn unavailable(&self) -> Option<String> {
        Some(self.0.clone())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::NONE
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn is_shared(&self, _: &UsbDevice) -> bool {
        false
    }

    fn sharing_advice(&self, _: &BusId, _: bool) -> String {
        self.0.clone()
    }

    fn share<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        self.refuse()
    }

    fn unshare<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        self.refuse()
    }

    fn status<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        Box::pin(async { Ok(ExportStatus::NotExported) })
    }

    fn reset<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async { Ok(()) })
    }

    fn export<'a>(&'a self, _: &'a UsbDevice) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        self.refuse()
    }
}

use std::fs;
use std::io;
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::host::UsbAttacher;
use crate::tasks::lock;
use crate::{BusId, Hub, Module, PortStatus, UsbError, UsbSpeed, VhciPort};

/// The Linux kernel's USB/IP client side (`vhci-hcd`), driven through sysfs, the way
/// `usbip attach` does it. Attached devices appear on the host kernel. Needs root and the
/// `vhci-hcd` module.
#[derive(Debug)]
pub struct SysfsVhci {
    sys: PathBuf,
    attach_lock: Mutex<()>,
}

impl Default for SysfsVhci {
    fn default() -> Self {
        Self::new("/sys")
    }
}

impl SysfsVhci {
    /// `sys` is normally `/sys`; tests point it at a fake tree.
    pub fn new(sys: impl Into<PathBuf>) -> Self {
        Self {
            sys: sys.into(),
            attach_lock: Mutex::new(()),
        }
    }

    /// The controller's attribute folder. Older kernels use the platform device; a 2026 change
    /// moved attach, detach and status under the driver, so both are tried.
    fn controller(&self) -> Option<PathBuf> {
        [
            self.sys.join("devices").join("platform").join("vhci_hcd.0"),
            self.sys
                .join("bus")
                .join("platform")
                .join("drivers")
                .join("vhci_hcd"),
        ]
        .into_iter()
        .find(|dir| dir.join("attach").is_file())
    }

    fn require_controller(&self) -> Result<PathBuf, UsbError> {
        self.controller()
            .ok_or(UsbError::ModuleMissing(Module::VhciHcd))
    }
}

impl UsbAttacher for SysfsVhci {
    fn unavailable(&self) -> Option<String> {
        match self.controller() {
            Some(_) => None,
            None => Some(UsbError::ModuleMissing(Module::VhciHcd).to_string()),
        }
    }

    /// Parses the status files: `hub port sta spd dev sockfd local_busid` per line.
    fn ports(&self) -> Vec<VhciPort> {
        let Some(controller) = self.controller() else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = fs::read_dir(&controller)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("status"))
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();
        files.sort();

        files
            .iter()
            .filter_map(|file| fs::read_to_string(file).ok())
            .flat_map(|text| parse_status(&text))
            .collect()
    }

    fn attach(&self, socket: &TcpStream, dev_id: u32, speed: UsbSpeed) -> Result<u32, UsbError> {
        let controller = self.require_controller()?;
        // SuperSpeed devices need a port on the SuperSpeed hub; everything else uses the
        // high-speed hub.
        let hub = Hub::for_speed(speed);
        let fd = socket.as_raw_fd();

        // Picking a free port and writing the attach line is one step: two attaches at once
        // would otherwise pick the same port. If the kernel still refuses (someone outside Koppel
        // took the port in between), the next free port gets one try.
        let _attaching = lock(&self.attach_lock);
        let mut tried = Vec::new();
        let mut last = None;
        for _ in 0..2 {
            let port = self
                .ports()
                .into_iter()
                .find(|p| p.is_free() && p.hub == hub && !tried.contains(&p.port))
                .ok_or(UsbError::NoFreePort)?;
            let line = format!("{} {fd} {dev_id} {}", port.port, speed.wire());
            match fs::write(controller.join("attach"), line) {
                Ok(()) => return Ok(port.port),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    return Err(UsbError::io("The kernel refused to attach the device", e));
                }
                Err(e) => {
                    tried.push(port.port);
                    last = Some(e);
                }
            }
        }
        Err(UsbError::io(
            "The kernel refused to attach the device",
            last.unwrap_or_else(|| io::Error::other("no attempt was made")),
        ))
    }

    fn detach(&self, port: u32) -> Result<(), UsbError> {
        let controller = self.require_controller()?;
        fs::write(controller.join("detach"), port.to_string())
            .map_err(|e| UsbError::io(format!("The kernel refused to detach port {port}"), e))
    }
}

/// Parses one status file: a header line, then `hub port sta spd dev sockfd local_busid` per
/// port. A line that does not read is skipped.
pub(crate) fn parse_status(text: &str) -> Vec<VhciPort> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 7 {
                return None;
            }
            Some(VhciPort {
                hub: Hub::parse(parts[0])?,
                port: parts[1].parse().ok()?,
                status: PortStatus::from_kernel(parts[2].parse().ok()?),
                dev_id: u32::from_str_radix(parts[4], 16).ok()?,
                // A free port prints `0-0`, which is no bus ID.
                local_bus_id: parts[6].parse::<BusId>().ok().filter(|_| parts[6] != "0-0"),
            })
        })
        .collect()
}

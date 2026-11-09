use std::fs;
use std::io;
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use super::usbfs;
use crate::host::{Export, UsbHost};
use crate::{
    BoxFuture, DeviceStream, ExportStatus, UsbDevice, UsbError, UsbInterface, UsbSpeed, bus_id,
    loopback_pair, relay,
};

/// The Linux kernel's USB/IP server side (`usbip-host`), driven through sysfs, the way the
/// `usbip bind` tool and `usbipd` do it. Needs root and the `usbip-host` module.
#[derive(Debug, Clone)]
pub struct SysfsUsbHost {
    tree: Tree,
}

/// Where sysfs and the usbfs device nodes are. The work on them is small and local, but it is
/// still file I/O, so it runs off the async threads.
#[derive(Debug, Clone)]
struct Tree {
    sys: PathBuf,
    dev: PathBuf,
}

impl Default for SysfsUsbHost {
    fn default() -> Self {
        Self::new("/sys", "/dev")
    }
}

impl SysfsUsbHost {
    /// `sys` is normally `/sys` and `dev` normally `/dev`; tests point them at a fake tree.
    pub fn new(sys: impl Into<PathBuf>, dev: impl Into<PathBuf>) -> Self {
        Self {
            tree: Tree {
                sys: sys.into(),
                dev: dev.into(),
            },
        }
    }

    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Tree) -> Result<T, UsbError> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, UsbError>> {
        let tree = self.tree.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || work(&tree))
                .await
                .map_err(|e| UsbError::new(format!("Reading sysfs failed: {e}")))?
        })
    }
}

impl UsbHost for SysfsUsbHost {
    fn unavailable(&self) -> Option<String> {
        self.tree.unavailable()
    }

    fn can_share(&self) -> bool {
        true
    }

    fn can_unshare(&self) -> bool {
        true
    }

    fn can_reset(&self) -> bool {
        true
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        self.run(Tree::list_devices)
    }

    fn is_shared(&self, device: &UsbDevice) -> bool {
        device.driver.as_deref() == Some("usbip-host")
    }

    fn sharing_advice(&self, _: &str, _: bool) -> String {
        self.unavailable()
            .unwrap_or_else(|| "Koppel shares devices on this machine itself.".to_owned())
    }

    fn share<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        let bus_id = bus_id.to_owned();
        self.run(move |tree| tree.share(&bus_id))
    }

    fn unshare<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        let bus_id = bus_id.to_owned();
        self.run(move |tree| tree.unshare(&bus_id))
    }

    fn status<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        let bus_id = bus_id.to_owned();
        self.run(move |tree| tree.status(&bus_id))
    }

    fn reset<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        let bus_id = bus_id.to_owned();
        self.run(move |tree| tree.reset(&bus_id))
    }

    /// Hands the kernel one end of a loopback socket pair, which from then on carries the
    /// device's USB traffic. The relay carries the other end through the stream.
    fn export<'a>(
        &'a self,
        device: &'a UsbDevice,
    ) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        Box::pin(async move {
            let bus_id = bus_id::require(&device.bus_id)?.to_owned();
            if self.status(&bus_id).await? != ExportStatus::Available {
                return Err(UsbError::new(format!(
                    "Device {bus_id} is not available to export. It may be in use by another client."
                )));
            }
            let (kernel_end, relay_end) = loopback_pair()
                .await
                .map_err(|e| UsbError::new(format!("Could not export {bus_id}: {e}")))?;
            let file = self.tree.devices().join(&bus_id).join("usbip_sockfd");
            let fd = kernel_end.as_raw_fd().to_string();
            tokio::task::spawn_blocking(move || write(&file, &fd))
                .await
                .map_err(|e| UsbError::new(format!("Writing sysfs failed: {e}")))??;
            Ok(Box::new(KernelExport {
                kernel_end,
                relay_end,
            }) as Box<dyn Export>)
        })
    }
}

/// A device the kernel took a socket for.
struct KernelExport {
    kernel_end: TcpStream,
    relay_end: tokio::net::TcpStream,
}

impl Export for KernelExport {
    fn relay<'a>(
        self: Box<Self>,
        network: &'a mut dyn DeviceStream,
        stop: CancellationToken,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let KernelExport {
                kernel_end,
                relay_end,
            } = *self;
            // Runs until the client detaches or the link drops; the kernel then frees the device.
            relay(network, relay_end, &stop).await;
            drop(kernel_end);
        })
    }
}

impl Tree {
    fn devices(&self) -> PathBuf {
        self.sys.join("bus").join("usb").join("devices")
    }

    fn usbip_host(&self) -> PathBuf {
        self.sys
            .join("bus")
            .join("usb")
            .join("drivers")
            .join("usbip-host")
    }

    fn unavailable(&self) -> Option<String> {
        if self.usbip_host().is_dir() {
            None
        } else {
            Some("The usbip-host kernel module is not loaded. Run 'modprobe usbip-host' on the host (add it to /etc/modules to keep it).".to_owned())
        }
    }

    fn require_module(&self) -> Result<(), UsbError> {
        match self.unavailable() {
            Some(reason) => Err(UsbError::new(reason)),
            None => Ok(()),
        }
    }

    fn list_devices(&self) -> Result<Vec<UsbDevice>, UsbError> {
        let devices = self.devices();
        if !devices.is_dir() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&devices)
            .map_err(|e| UsbError::new(format!("Could not read {}: {e}", devices.display())))?;
        let mut listed = Vec::new();
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !entry.path().is_dir() || !bus_id::is_valid(&name) || self.is_imported(&name) {
                continue;
            }
            if let Some(device) = self.read_device(&name)
                && !device.is_hub()
            {
                listed.push(device);
            }
        }
        listed.sort_by(|a, b| a.bus_id.cmp(&b.bus_id));
        Ok(listed)
    }

    fn share(&self, bus_id: &str) -> Result<(), UsbError> {
        self.require_module()?;
        let bus_id = bus_id::require(bus_id)?;
        let device = self.devices().join(bus_id);
        let usbip_host = self.usbip_host();
        write(&usbip_host.join("match_busid"), &format!("add {bus_id}"))?;
        let bound = (|| {
            if let Some(driver) = driver_of(&device)
                && driver != "usbip-host"
            {
                write(&device.join("driver").join("unbind"), bus_id)?;
            }
            if driver_of(&device).as_deref() != Some("usbip-host") {
                write(&usbip_host.join("bind"), bus_id)?;
            }
            Ok(())
        })();
        if bound.is_err() {
            // A share that did not take must not leave the bus ID on the match list, or the
            // device binds by itself later. The first failure is the one to report.
            let _ = write(&usbip_host.join("match_busid"), &format!("del {bus_id}"));
        }
        bound
    }

    fn unshare(&self, bus_id: &str) -> Result<(), UsbError> {
        self.require_module()?;
        let bus_id = bus_id::require(bus_id)?;
        let device = self.devices().join(bus_id);
        let usbip_host = self.usbip_host();
        if driver_of(&device).as_deref() == Some("usbip-host") {
            write(&usbip_host.join("unbind"), bus_id)?;
        }
        write(&usbip_host.join("match_busid"), &format!("del {bus_id}"))?;
        // Let the kernel find the normal driver again.
        write(
            &self.sys.join("bus").join("usb").join("drivers_probe"),
            bus_id,
        )
    }

    fn status(&self, bus_id: &str) -> Result<ExportStatus, UsbError> {
        let bus_id = bus_id::require(bus_id)?;
        Ok(read_attribute(&self.devices().join(bus_id), "usbip_status")
            .and_then(|s| s.parse::<i64>().ok())
            .map_or(ExportStatus::NotExported, ExportStatus::from_kernel))
    }

    fn reset(&self, bus_id: &str) -> Result<(), UsbError> {
        let bus_id = bus_id::require(bus_id)?;
        let device = self.devices().join(bus_id);
        let node = self
            .dev
            .join("bus")
            .join("usb")
            .join(format!("{:03}", read_u32(&device, "busnum")))
            .join(format!("{:03}", read_u32(&device, "devnum")));
        // A real node is a character device, which is not a regular file.
        if !node.exists() || node.is_dir() {
            return Err(UsbError::new(format!(
                "Cannot reset {bus_id}: there is no device node {}.",
                node.display()
            )));
        }
        usbfs::reset(&node)
    }

    /// Devices attached from another server sit under vhci-hcd. They are on loan to this machine,
    /// so they are never offered for sharing here.
    fn is_imported(&self, bus_id: &str) -> bool {
        fs::read_link(self.devices().join(bus_id))
            .is_ok_and(|target| target.to_string_lossy().contains("vhci_hcd"))
    }

    fn read_device(&self, bus_id: &str) -> Option<UsbDevice> {
        let path = self.devices().join(bus_id);
        let vendor_id = read_hex16(&path, "idVendor")?;
        let product_id = read_hex16(&path, "idProduct")?;

        let prefix = format!("{bus_id}:");
        let mut interface_dirs: Vec<PathBuf> = fs::read_dir(&path)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        interface_dirs.sort();
        let interfaces = interface_dirs
            .iter()
            .map(|i| {
                UsbInterface::new(
                    read_hex8(i, "bInterfaceClass"),
                    read_hex8(i, "bInterfaceSubClass"),
                    read_hex8(i, "bInterfaceProtocol"),
                )
            })
            .collect();

        Some(UsbDevice {
            bus_id: bus_id.to_owned(),
            path: path.to_string_lossy().into_owned(),
            bus_num: read_u32(&path, "busnum"),
            dev_num: read_u32(&path, "devnum"),
            speed: parse_speed(read_attribute(&path, "speed").as_deref()),
            vendor_id,
            product_id,
            bcd_device: read_hex16(&path, "bcdDevice").unwrap_or(0),
            class: read_hex8(&path, "bDeviceClass"),
            sub_class: read_hex8(&path, "bDeviceSubClass"),
            protocol: read_hex8(&path, "bDeviceProtocol"),
            configuration_value: read_u32(&path, "bConfigurationValue") as u8,
            num_configurations: read_u32(&path, "bNumConfigurations") as u8,
            interfaces,
            manufacturer: read_attribute(&path, "manufacturer"),
            product: read_attribute(&path, "product"),
            serial: read_attribute(&path, "serial"),
            driver: driver_of(&path),
        })
    }
}

/// Speeds as sysfs prints them, in Mbit/s.
fn parse_speed(speed: Option<&str>) -> UsbSpeed {
    match speed {
        Some("1.5") => UsbSpeed::Low,
        Some("12") => UsbSpeed::Full,
        Some("480") => UsbSpeed::High,
        Some("53.3-480") => UsbSpeed::Wireless,
        Some("5000") => UsbSpeed::Super,
        Some("10000" | "20000") => UsbSpeed::SuperPlus,
        _ => UsbSpeed::Unknown,
    }
}

/// A real sysfs driver entry is a symlink to `/sys/bus/usb/drivers/<name>`; a fake tree may use
/// a folder with the name in a `name` file.
fn driver_of(device: &Path) -> Option<String> {
    let link = device.join("driver");
    if !link.is_dir() {
        return None;
    }
    match fs::read_link(&link) {
        Ok(target) => target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        Err(_) => read_attribute(&link, "name"),
    }
}

/// The attribute's text, trimmed. Missing or unreadable counts as absent.
fn read_attribute(directory: &Path, name: &str) -> Option<String> {
    fs::read_to_string(directory.join(name))
        .ok()
        .map(|text| text.trim().to_owned())
}

fn read_u32(directory: &Path, name: &str) -> u32 {
    read_attribute(directory, name)
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
}

fn read_hex16(directory: &Path, name: &str) -> Option<u16> {
    u16::from_str_radix(&read_attribute(directory, name)?, 16).ok()
}

fn read_hex8(directory: &Path, name: &str) -> u8 {
    read_attribute(directory, name)
        .and_then(|text| u8::from_str_radix(&text, 16).ok())
        .unwrap_or(0)
}

fn write(file: &Path, value: &str) -> Result<(), UsbError> {
    fs::write(file, value).map_err(|e: io::Error| {
        UsbError::new(format!(
            "The kernel refused '{value}' on {}: {e}",
            file.display()
        ))
    })
}

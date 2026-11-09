/// Device speed, numbered as the kernel's `enum usb_device_speed` and the USB/IP wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum UsbSpeed {
    #[default]
    Unknown = 0,
    Low = 1,
    Full = 2,
    High = 3,
    Wireless = 4,
    Super = 5,
    SuperPlus = 6,
}

impl UsbSpeed {
    /// The speed a wire value or a kernel number stands for. One this crate does not know is
    /// `Unknown`.
    pub fn from_wire(value: u32) -> Self {
        match value {
            1 => Self::Low,
            2 => Self::Full,
            3 => Self::High,
            4 => Self::Wireless,
            5 => Self::Super,
            6 => Self::SuperPlus,
            _ => Self::Unknown,
        }
    }

    pub fn wire(self) -> u32 {
        self as u32
    }
}

/// One interface of a device: its class, subclass and protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UsbInterface {
    pub class: u8,
    pub sub_class: u8,
    pub protocol: u8,
}

impl UsbInterface {
    pub fn new(class: u8, sub_class: u8, protocol: u8) -> Self {
        Self {
            class,
            sub_class,
            protocol,
        }
    }
}

/// A USB device as the host sees it, with what USB/IP needs to describe it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbDevice {
    /// The kernel's bus ID, such as `1-1.4`. Stable for a port, not for a device.
    pub bus_id: String,
    pub path: String,
    pub bus_num: u32,
    pub dev_num: u32,
    pub speed: UsbSpeed,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub class: u8,
    pub sub_class: u8,
    pub protocol: u8,
    pub configuration_value: u8,
    pub num_configurations: u8,
    pub interfaces: Vec<UsbInterface>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// The driver bound to it, such as `ftdi_sio` or `usbip-host`; `None` when there is none.
    pub driver: Option<String>,
}

impl UsbDevice {
    /// USB/IP's device ID: bus number in the high 16 bits, device number in the low.
    pub fn dev_id(&self) -> u32 {
        (self.bus_num << 16) | self.dev_num
    }

    /// `0403:6001`.
    pub fn vid_pid(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }

    /// Hubs are never shared: sharing one would take every device behind it.
    pub fn is_hub(&self) -> bool {
        self.class == 0x09
    }
}

/// Whether a shared device is free, in use by a client, or broken (the kernel's `usbip_status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportStatus {
    NotExported = 0,
    Available = 1,
    Used = 2,
    Error = 3,
}

impl ExportStatus {
    /// The status a kernel number stands for; anything else counts as not exported.
    pub fn from_kernel(value: i64) -> Self {
        match value {
            1 => Self::Available,
            2 => Self::Used,
            3 => Self::Error,
            _ => Self::NotExported,
        }
    }
}

/// A virtual host controller port on the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VhciPort {
    pub port: u32,
    /// `hs` or `ss`.
    pub hub: String,
    pub status: u32,
    pub dev_id: u32,
    pub local_bus_id: Option<String>,
}

impl VhciPort {
    /// `VDEV_ST_NULL`: nothing attached.
    pub fn is_free(&self) -> bool {
        self.status == 4
    }
}

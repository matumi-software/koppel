use crate::BusId;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    /// The kernel's bus ID, such as `1-1.4`. Stable for a port, not for a device.
    pub bus_id: BusId,
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
    /// A device on `bus_id` with every other field empty, for a host to fill in.
    pub fn new(bus_id: BusId) -> Self {
        Self {
            bus_id,
            path: String::new(),
            bus_num: 0,
            dev_num: 0,
            speed: UsbSpeed::Unknown,
            vendor_id: 0,
            product_id: 0,
            bcd_device: 0,
            class: 0,
            sub_class: 0,
            protocol: 0,
            configuration_value: 0,
            num_configurations: 0,
            interfaces: Vec::new(),
            manufacturer: None,
            product: None,
            serial: None,
            driver: None,
        }
    }

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

/// Which of vhci-hcd's two root hubs a port is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hub {
    /// `hs`: the high-speed hub, for everything below SuperSpeed.
    HighSpeed,
    /// `ss`: the SuperSpeed hub.
    SuperSpeed,
}

impl Hub {
    /// The hub a device of this speed needs.
    pub fn for_speed(speed: UsbSpeed) -> Self {
        if speed >= UsbSpeed::Super {
            Self::SuperSpeed
        } else {
            Self::HighSpeed
        }
    }

    /// The name vhci-hcd's status file uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HighSpeed => "hs",
            Self::SuperSpeed => "ss",
        }
    }

    /// `hs` or `ss`; anything else is no hub of ours.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "hs" => Some(Self::HighSpeed),
            "ss" => Some(Self::SuperSpeed),
            _ => None,
        }
    }
}

/// A virtual port's state, numbered as the kernel's `VDEV_ST_*` (the number is what the status
/// file prints).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortStatus {
    /// `VDEV_ST_NULL`: nothing attached.
    Null,
    /// `VDEV_ST_NOTASSIGNED`: claimed, not yet connected.
    NotAssigned,
    /// `VDEV_ST_USED`: a device is attached.
    Used,
    /// `VDEV_ST_ERROR`.
    Error,
    /// A number this version does not know.
    Other(u32),
}

impl PortStatus {
    pub fn from_kernel(value: u32) -> Self {
        match value {
            4 => Self::Null,
            5 => Self::NotAssigned,
            6 => Self::Used,
            7 => Self::Error,
            other => Self::Other(other),
        }
    }
}

/// A virtual host controller port on the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VhciPort {
    pub port: u32,
    pub hub: Hub,
    pub status: PortStatus,
    pub dev_id: u32,
    pub local_bus_id: Option<BusId>,
}

impl VhciPort {
    /// Nothing is attached on this port.
    pub fn is_free(&self) -> bool {
        self.status == PortStatus::Null
    }
}

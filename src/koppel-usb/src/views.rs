//! What the UI and the remote clients see, with 0.1.x's JSON names.

use koppel_config::json::date_time_offset;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::UsbSpeed;

/// A device as the UI sees it. `state` uses the design system's state words: `local`, `shared`,
/// `attached`, or `offline` (shared but unplugged).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceView {
    pub bus_id: String,
    pub vid_pid: String,
    pub name: String,
    pub manufacturer: Option<String>,
    pub speed: String,
    pub state: String,
    pub attached_to: Option<String>,
    #[serde(with = "date_time_offset::option")]
    pub attached_since: Option<OffsetDateTime>,
    /// Why the last change to this device failed, for the UI.
    pub fault: Option<String>,
    pub serial: Option<String>,
    pub driver: Option<String>,
    pub interfaces: Option<Vec<String>>,
}

/// What the server's UI shows: its devices, or why it cannot share any.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDevices {
    pub devices: Vec<DeviceView>,
    pub unavailable: Option<String>,
}

/// What a remote client sees of a server's devices: only the shared ones, without who holds them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteDevices {
    pub devices: Vec<DeviceView>,
}

impl ServerDevices {
    /// What a remote client may see: the shared devices, with what identifies the device and what
    /// its UI shows. Who holds a device, since when, the serial number, the driver and fault text
    /// stay on this machine. Another client learns only that the device is taken.
    pub fn for_remote(&self) -> RemoteDevices {
        RemoteDevices {
            devices: self
                .devices
                .iter()
                .filter(|d| d.state == "shared" || d.state == "attached")
                .map(|d| DeviceView {
                    bus_id: d.bus_id.clone(),
                    vid_pid: d.vid_pid.clone(),
                    name: d.name.clone(),
                    manufacturer: d.manufacturer.clone(),
                    speed: d.speed.clone(),
                    state: d.state.clone(),
                    interfaces: d.interfaces.clone(),
                    ..DeviceView::default()
                })
                .collect(),
        }
    }
}

/// A remote device as the client's UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDeviceView {
    pub device: DeviceView,
    pub attached_here: bool,
    /// The port this machine's kernel or driver attached it on; `None` for another gateway attach.
    pub port: Option<u32>,
    #[serde(with = "date_time_offset::option")]
    pub attached_since: Option<OffsetDateTime>,
    pub fault: Option<String>,
    /// Where a gateway attach came from, such as `172.17.0.1`.
    pub attached_via: Option<String>,
}

/// What the client's UI shows: the link to the server and the devices it shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDevices {
    /// `unconfigured`, `connecting`, `connected` or `error`.
    pub link: String,
    pub link_message: Option<String>,
    pub devices: Vec<ClientDeviceView>,
    pub unavailable: Option<String>,
    /// True while the plain USB/IP gateway listens for a driver Koppel does not drive itself
    /// (Koppel in a container on Windows), so the UI can show the command that attaches through it.
    pub gateway: bool,
    /// Why the gateway is not listening although it should be, such as a port that is taken.
    pub gateway_fault: Option<String>,
}

/// USB interface classes as people know them.
pub fn interface_class_name(code: u8) -> String {
    match code {
        0x01 => "Audio",
        0x02 => "Communications",
        0x03 => "HID (keyboard, mouse, game pad)",
        0x05 => "Physical",
        0x06 => "Image",
        0x07 => "Printer",
        0x08 => "Mass storage",
        0x09 => "Hub",
        0x0A => "CDC data",
        0x0B => "Smart card",
        0x0D => "Content security",
        0x0E => "Video",
        0x0F => "Health",
        0x10 => "Audio/video",
        0xDC => "Diagnostic",
        0xE0 => "Wireless",
        0xEF => "Miscellaneous",
        0xFE => "Application specific",
        0xFF => "Vendor specific",
        _ => return format!("Class 0x{code:02x}"),
    }
    .to_owned()
}

pub fn speed_label(speed: UsbSpeed) -> &'static str {
    match speed {
        UsbSpeed::Low => "USB 1.0 (1.5 Mbit/s)",
        UsbSpeed::Full => "USB 1.1 (12 Mbit/s)",
        UsbSpeed::High => "USB 2.0 (480 Mbit/s)",
        UsbSpeed::Super => "USB 3.0 (5 Gbit/s)",
        UsbSpeed::SuperPlus => "USB 3.1+ (10 Gbit/s)",
        UsbSpeed::Unknown | UsbSpeed::Wireless => "Unknown speed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_view_hides_who_holds_a_device_and_the_serial() {
        let server = ServerDevices {
            devices: vec![
                DeviceView {
                    bus_id: "1-1.4".into(),
                    vid_pid: "064f:2af9".into(),
                    name: "CodeMeter-Stick".into(),
                    state: "attached".into(),
                    attached_to: Some("client-1".into()),
                    attached_since: Some(OffsetDateTime::UNIX_EPOCH),
                    fault: Some("detail".into()),
                    serial: Some("SERIAL-123".into()),
                    driver: Some("usbip-host".into()),
                    interfaces: Some(vec!["Vendor specific".into()]),
                    ..DeviceView::default()
                },
                DeviceView {
                    bus_id: "2-1".into(),
                    state: "local".into(),
                    ..DeviceView::default()
                },
            ],
            unavailable: None,
        };

        let remote = server.for_remote();

        assert_eq!(remote.devices.len(), 1);
        let device = &remote.devices[0];
        assert_eq!(device.state, "attached");
        assert_eq!(device.attached_to, None);
        assert_eq!(device.attached_since, None);
        assert_eq!(device.serial, None);
        assert_eq!(device.driver, None);
        assert_eq!(device.fault, None);
        assert_eq!(device.interfaces, Some(vec!["Vendor specific".to_owned()]));
    }

    #[test]
    fn views_use_the_json_names_of_0_1() {
        let json = serde_json::to_value(ClientDevices {
            link: "connected".into(),
            link_message: None,
            devices: vec![ClientDeviceView {
                device: DeviceView::default(),
                attached_here: true,
                port: Some(8),
                attached_since: None,
                fault: None,
                attached_via: None,
            }],
            unavailable: None,
            gateway: false,
            gateway_fault: None,
        })
        .unwrap();

        assert_eq!(json["devices"][0]["attachedHere"], true);
        assert_eq!(json["devices"][0]["device"]["busId"], "");
        assert!(json["devices"][0]["device"]["attachedSince"].is_null());
        assert!(json["gatewayFault"].is_null());
        assert!(json.get("linkMessage").is_some());
    }

    #[test]
    fn a_device_list_from_a_server_reads_back() {
        let remote: RemoteDevices = serde_json::from_str(
            r#"{"devices":[{"busId":"1-1.4","vidPid":"064f:2af9","name":"Stick","manufacturer":null,"speed":"USB 1.1 (12 Mbit/s)","state":"shared","attachedTo":null,"attachedSince":null,"fault":null,"serial":null,"driver":null,"interfaces":["Vendor specific"]}]}"#,
        )
        .unwrap();

        assert_eq!(remote.devices[0].bus_id, "1-1.4");
        assert_eq!(remote.devices[0].state, "shared");
    }
}

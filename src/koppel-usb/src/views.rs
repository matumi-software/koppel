//! What the UI and the remote clients see, with 0.1.x's JSON names.

use serde::de::IgnoredAny;
use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;

use crate::{BusId, UsbSpeed};

/// Where a device stands, in the design system's state words: `local`, `shared`, `attached`, or
/// `offline` (shared but unplugged). A word this version does not know reads as `local`, so a
/// newer server's list still loads and the device is simply not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceState {
    Shared,
    Attached,
    Offline,
    #[default]
    #[serde(other)]
    Local,
}

impl DeviceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Shared => "shared",
            Self::Attached => "attached",
            Self::Offline => "offline",
        }
    }

    /// The state a word stands for; an unknown word is `Local`.
    pub fn parse(word: &str) -> Self {
        match word {
            "shared" => Self::Shared,
            "attached" => Self::Attached,
            "offline" => Self::Offline,
            _ => Self::Local,
        }
    }
}

/// Where the link to the server stands: `unconfigured`, `connecting`, `connected` or `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    #[default]
    Unconfigured,
    Connecting,
    Connected,
    Error,
}

impl LinkState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Error => "error",
        }
    }
}

/// A device as the UI sees it. `state` uses the design system's state words: `local`, `shared`,
/// `attached`, or `offline` (shared but unplugged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub bus_id: BusId,
    #[serde(default)]
    pub vid_pid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub speed: String,
    #[serde(default)]
    pub state: DeviceState,
    #[serde(default)]
    pub attached_to: Option<String>,
    #[serde(with = "time::serde::rfc3339::option", default)]
    pub attached_since: Option<OffsetDateTime>,
    /// Why the last change to this device failed, for the UI.
    #[serde(default)]
    pub fault: Option<String>,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default)]
    pub interfaces: Option<Vec<String>>,
}

impl DeviceView {
    /// A row for `bus_id` with nothing else filled in.
    pub fn new(bus_id: BusId) -> Self {
        Self {
            bus_id,
            vid_pid: String::new(),
            name: String::new(),
            manufacturer: None,
            speed: String::new(),
            state: DeviceState::Local,
            attached_to: None,
            attached_since: None,
            fault: None,
            serial: None,
            driver: None,
            interfaces: None,
        }
    }
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
    /// A row whose bus ID is not one is dropped while reading, not the whole list.
    #[serde(deserialize_with = "lenient_devices")]
    pub devices: Vec<DeviceView>,
}

fn lenient_devices<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<DeviceView>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Row {
        Device(Box<DeviceView>),
        Unreadable(IgnoredAny),
    }
    let rows = Vec::<Row>::deserialize(deserializer)?;
    let total = rows.len();
    let devices: Vec<DeviceView> = rows
        .into_iter()
        .filter_map(|row| match row {
            Row::Device(device) => Some(*device),
            Row::Unreadable(_) => None,
        })
        .collect();
    if devices.len() != total {
        tracing::warn!(
            "Ignored {} devices from the server with a bus ID that is not valid",
            total - devices.len()
        );
    }
    Ok(devices)
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
                .filter(|d| matches!(d.state, DeviceState::Shared | DeviceState::Attached))
                .map(|d| DeviceView {
                    vid_pid: d.vid_pid.clone(),
                    name: d.name.clone(),
                    manufacturer: d.manufacturer.clone(),
                    speed: d.speed.clone(),
                    state: d.state,
                    interfaces: d.interfaces.clone(),
                    ..DeviceView::new(d.bus_id.clone())
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
    #[serde(with = "time::serde::rfc3339::option")]
    pub attached_since: Option<OffsetDateTime>,
    pub fault: Option<String>,
    /// Where a gateway attach came from, such as `172.17.0.1`.
    pub attached_via: Option<String>,
}

/// What the client's UI shows: the link to the server and the devices it shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDevices {
    pub link: LinkState,
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
                    vid_pid: "064f:2af9".into(),
                    name: "CodeMeter-Stick".into(),
                    state: DeviceState::Attached,
                    attached_to: Some("client-1".into()),
                    attached_since: Some(OffsetDateTime::UNIX_EPOCH),
                    fault: Some("detail".into()),
                    serial: Some("SERIAL-123".into()),
                    driver: Some("usbip-host".into()),
                    interfaces: Some(vec!["Vendor specific".into()]),
                    ..DeviceView::new("1-1.4".parse().unwrap())
                },
                DeviceView::new("2-1".parse().unwrap()),
            ],
            unavailable: None,
        };

        let remote = server.for_remote();

        assert_eq!(remote.devices.len(), 1);
        let device = &remote.devices[0];
        assert_eq!(device.state, DeviceState::Attached);
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
            link: LinkState::Connected,
            link_message: None,
            devices: vec![ClientDeviceView {
                device: DeviceView::new("1-1.4".parse().unwrap()),
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
        assert_eq!(json["devices"][0]["device"]["busId"], "1-1.4");
        assert_eq!(json["devices"][0]["device"]["state"], "local");
        assert_eq!(json["link"], "connected");
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
        assert_eq!(remote.devices[0].state, DeviceState::Shared);
    }
}

#[cfg(test)]
mod word_tests {
    use super::*;

    #[test]
    fn the_state_words_are_the_ones_the_ui_knows() {
        for (state, word) in [
            (DeviceState::Local, "local"),
            (DeviceState::Shared, "shared"),
            (DeviceState::Attached, "attached"),
            (DeviceState::Offline, "offline"),
        ] {
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{word}\"")
            );
            assert_eq!(state.as_str(), word);
            assert_eq!(DeviceState::parse(word), state);
            assert_eq!(
                serde_json::from_str::<DeviceState>(&format!("\"{word}\"")).unwrap(),
                state
            );
        }
        for (state, word) in [
            (LinkState::Unconfigured, "unconfigured"),
            (LinkState::Connecting, "connecting"),
            (LinkState::Connected, "connected"),
            (LinkState::Error, "error"),
        ] {
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{word}\"")
            );
            assert_eq!(state.as_str(), word);
        }
    }

    #[test]
    fn a_state_word_from_a_newer_server_reads_as_local() {
        assert_eq!(
            serde_json::from_str::<DeviceState>("\"quarantined\"").unwrap(),
            DeviceState::Local
        );
    }

    #[test]
    fn a_row_with_a_bad_bus_id_is_dropped_and_the_rest_of_the_list_stays() {
        let remote: RemoteDevices = serde_json::from_str(
            r#"{"devices":[{"busId":"1-1 --stop","name":"bad"},{"busId":"2-1","name":"ok","state":"shared"}]}"#,
        )
        .unwrap();

        assert_eq!(remote.devices.len(), 1);
        assert_eq!(remote.devices[0].name, "ok");
    }
}

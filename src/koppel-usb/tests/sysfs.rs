//! Drives the sysfs adapters against a fake `/sys` tree. The real kernel is not involved. Linux
//! only: the code is, and the tree has folders with `:` in their names, as sysfs does.
#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use koppel_usb::linux::{SysfsUsbHost, SysfsVhci};
use koppel_usb::stream::memory;
use koppel_usb::{UsbAttacher, UsbHost, UsbInterface, UsbSpeed, loopback_pair};
use tokio_util::sync::CancellationToken;

struct Sys {
    dir: TempDir,
}

impl Sys {
    fn new() -> Self {
        Self {
            dir: TempDir::new("sysfs"),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn devices(&self) -> PathBuf {
        self.root().join("bus").join("usb").join("devices")
    }

    fn usbip_host(&self) -> PathBuf {
        let dir = self
            .root()
            .join("bus")
            .join("usb")
            .join("drivers")
            .join("usbip-host");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn host(&self) -> SysfsUsbHost {
        SysfsUsbHost::new(self.root(), self.root().join("dev"))
    }

    fn add_device(
        &self,
        bus_id: &str,
        vid: &str,
        pid: &str,
        class: &str,
        speed: &str,
        driver: &str,
        product: Option<&str>,
    ) {
        let dir = self.devices().join(bus_id);
        fs::create_dir_all(&dir).unwrap();
        let write =
            |name: &str, value: &str| fs::write(dir.join(name), format!("{value}\n")).unwrap();
        write("idVendor", vid);
        write("idProduct", pid);
        write("bcdDevice", "0600");
        write("bDeviceClass", class);
        write("bDeviceSubClass", "00");
        write("bDeviceProtocol", "00");
        write("bConfigurationValue", "1");
        write("bNumConfigurations", "1");
        write("busnum", "1");
        write("devnum", "5");
        write("speed", speed);
        if let Some(product) = product {
            write("product", product);
        }
        // A real driver entry is a symlink; the fake tree records the driver name in a file instead.
        let driver_dir = dir.join("driver");
        fs::create_dir_all(&driver_dir).unwrap();
        fs::write(driver_dir.join("name"), driver).unwrap();
        let interface = dir.join(format!("{bus_id}:1.0"));
        fs::create_dir_all(&interface).unwrap();
        for name in [
            "bInterfaceClass",
            "bInterfaceSubClass",
            "bInterfaceProtocol",
        ] {
            fs::write(interface.join(name), "ff").unwrap();
        }
    }
}

#[tokio::test]
async fn lists_devices_and_skips_hubs_root_hubs_and_interfaces() {
    let sys = Sys::new();
    sys.add_device(
        "1-1.4",
        "0403",
        "6001",
        "00",
        "12",
        "ftdi_sio",
        Some("FT232R USB UART"),
    );
    sys.add_device("1-1", "05e3", "0610", "09", "480", "hub", None);
    fs::create_dir_all(sys.devices().join("usb1")).unwrap();

    let devices = sys.host().list_devices().await.unwrap();

    assert_eq!(devices.len(), 1);
    let device = &devices[0];
    assert_eq!(device.bus_id, "1-1.4");
    assert_eq!(device.vid_pid(), "0403:6001");
    assert_eq!(device.speed, UsbSpeed::Full);
    assert_eq!(device.product.as_deref(), Some("FT232R USB UART"));
    assert_eq!(device.driver.as_deref(), Some("ftdi_sio"));
    assert_eq!(device.interfaces, vec![UsbInterface::new(0xff, 0xff, 0xff)]);
}

#[tokio::test]
async fn a_hub_is_not_listed_but_the_devices_behind_it_are() {
    let sys = Sys::new();
    // The hub's own interface folder (3-1:1.0) sits in the tree too, as it does on a real machine.
    sys.add_device("3-1", "05e3", "0610", "09", "480", "hub", None);
    sys.add_device(
        "3-1.1",
        "046d",
        "0825",
        "ef",
        "480",
        "uvcvideo",
        Some("Webcam C270"),
    );
    sys.add_device(
        "3-1.2",
        "0781",
        "5567",
        "00",
        "480",
        "usb-storage",
        Some("Cruzer Blade"),
    );
    sys.add_device("3-2", "05e3", "0610", "09", "480", "hub", None);

    let listed: Vec<String> = sys
        .host()
        .list_devices()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.bus_id)
        .collect();

    assert_eq!(listed, vec!["3-1.1", "3-1.2"]);
}

#[tokio::test]
async fn devices_attached_from_another_server_are_not_listed() {
    let sys = Sys::new();
    let real = sys
        .root()
        .join("devices")
        .join("platform")
        .join("vhci_hcd.0")
        .join("usb1")
        .join("1-1");
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(sys.devices()).unwrap();
    symlink(&real, sys.devices().join("1-1")).unwrap();
    fs::write(real.join("idVendor"), "046d").unwrap();
    fs::write(real.join("idProduct"), "c548").unwrap();
    fs::write(real.join("bDeviceClass"), "00").unwrap();
    sys.add_device("3-2", "0403", "6001", "00", "12", "ftdi_sio", None);

    let listed: Vec<String> = sys
        .host()
        .list_devices()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.bus_id)
        .collect();

    assert_eq!(listed, vec!["3-2"]);
}

#[tokio::test]
async fn a_driver_entry_that_is_a_symlink_names_the_driver() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "ignored", None);
    let driver = sys.devices().join("1-1.4").join("driver");
    fs::remove_dir_all(&driver).unwrap();
    let target = sys.usbip_host();
    symlink(&target, &driver).unwrap();

    let devices = sys.host().list_devices().await.unwrap();

    assert_eq!(devices[0].driver.as_deref(), Some("usbip-host"));
}

#[tokio::test]
async fn share_claims_the_bus_id_unbinds_the_old_driver_and_binds_usbip_host() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "ftdi_sio", None);
    let usbip_host = sys.usbip_host();

    sys.host().share("1-1.4").await.unwrap();

    assert_eq!(
        fs::read_to_string(usbip_host.join("match_busid")).unwrap(),
        "add 1-1.4"
    );
    assert_eq!(
        fs::read_to_string(sys.devices().join("1-1.4").join("driver").join("unbind")).unwrap(),
        "1-1.4"
    );
    assert_eq!(
        fs::read_to_string(usbip_host.join("bind")).unwrap(),
        "1-1.4"
    );
}

#[tokio::test]
async fn a_share_whose_bind_fails_takes_the_bus_id_off_the_match_list_again() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "usbip-host", None);
    let usbip_host = sys.usbip_host();
    fs::write(sys.devices().join("1-1.4").join("usbip_status"), "0").unwrap();
    // The driver is not usbip-host yet, so the share has to bind, and the kernel refuses that.
    fs::create_dir_all(usbip_host.join("bind")).unwrap();
    fs::write(
        sys.devices().join("1-1.4").join("driver").join("name"),
        "ftdi_sio",
    )
    .unwrap();

    let share = sys.host().share("1-1.4").await;

    assert!(share.unwrap_err().message().contains("bind"));
    assert_eq!(
        fs::read_to_string(usbip_host.join("match_busid")).unwrap(),
        "del 1-1.4"
    );
}

#[tokio::test]
async fn unshare_unbinds_takes_the_bus_id_off_and_lets_the_kernel_find_the_driver() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "usbip-host", None);
    let usbip_host = sys.usbip_host();

    sys.host().unshare("1-1.4").await.unwrap();

    assert_eq!(
        fs::read_to_string(usbip_host.join("unbind")).unwrap(),
        "1-1.4"
    );
    assert_eq!(
        fs::read_to_string(usbip_host.join("match_busid")).unwrap(),
        "del 1-1.4"
    );
    assert_eq!(
        fs::read_to_string(sys.root().join("bus").join("usb").join("drivers_probe")).unwrap(),
        "1-1.4"
    );
}

#[tokio::test]
async fn share_without_the_module_explains_what_to_load() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "ftdi_sio", None);

    let share = sys.host().share("1-1.4").await;

    assert!(share.unwrap_err().message().contains("modprobe usbip-host"));
}

#[tokio::test]
async fn bus_ids_that_could_escape_sysfs_are_refused() {
    let sys = Sys::new();
    sys.usbip_host();

    assert!(sys.host().share("../../etc").await.is_err());
    assert!(sys.host().share("1-1\n").await.is_err());
}

#[tokio::test]
async fn reset_goes_through_the_usbfs_node_for_the_device_bus_and_number() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "046d", "c52b", "00", "12", "usbip-host", None);

    let reset = sys.host().reset("1-1.4").await;

    let expected = sys
        .root()
        .join("dev")
        .join("bus")
        .join("usb")
        .join("001")
        .join("005");
    assert!(
        reset
            .unwrap_err()
            .message()
            .contains(&expected.display().to_string())
    );
}

#[tokio::test]
async fn reset_reports_a_node_the_kernel_will_not_reset() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "046d", "c52b", "00", "12", "usbip-host", None);
    let node = sys.root().join("dev").join("bus").join("usb").join("001");
    fs::create_dir_all(&node).unwrap();
    fs::write(node.join("005"), "").unwrap();

    let reset = sys.host().reset("1-1.4").await;

    assert!(reset.unwrap_err().message().contains("refused to reset"));
}

#[tokio::test]
async fn export_hands_the_kernel_a_socket_and_relays_the_network_stream_through_the_other_end() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "usbip-host", None);
    fs::write(sys.devices().join("1-1.4").join("usbip_status"), "1").unwrap();
    let host = sys.host();
    let device = host.list_devices().await.unwrap().remove(0);
    let (_client, mut network) = memory::pair();

    let export = host.export(&device).await.unwrap();
    let stop = CancellationToken::new();
    let stopping = stop.clone();
    let relaying = tokio::spawn(async move { export.relay(&mut network, stopping).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), relaying)
        .await
        .unwrap()
        .unwrap();

    let fd = fs::read_to_string(sys.devices().join("1-1.4").join("usbip_sockfd")).unwrap();
    assert!(
        !fd.is_empty() && fd.chars().all(|c| c.is_ascii_digit()),
        "{fd:?}"
    );
}

#[tokio::test]
async fn export_refuses_a_device_that_is_in_use() {
    let sys = Sys::new();
    sys.add_device("1-1.4", "0403", "6001", "00", "12", "usbip-host", None);
    fs::write(sys.devices().join("1-1.4").join("usbip_status"), "2").unwrap();
    let host = sys.host();
    let device = host.list_devices().await.unwrap().remove(0);

    let export = host.export(&device).await;

    assert!(export.err().unwrap().message().contains("not available"));
}

#[tokio::test]
async fn vhci_picks_a_free_port_on_the_right_hub_and_writes_the_attach_line() {
    let sys = Sys::new();
    let vhci = sys
        .root()
        .join("devices")
        .join("platform")
        .join("vhci_hcd.0");
    fs::create_dir_all(&vhci).unwrap();
    fs::write(vhci.join("attach"), "").unwrap();
    fs::write(
        vhci.join("status"),
        [
            "hub port sta spd dev      sockfd local_busid",
            "hs  0000 006 002 00010005 000003 1-1",
            "hs  0001 004 000 00000000 000000 0-0",
            "ss  0008 004 000 00000000 000000 0-0",
        ]
        .join("\n"),
    )
    .unwrap();
    let attacher = SysfsVhci::new(sys.root());
    let (socket, _relay) = loopback_pair().await.unwrap();

    assert_eq!(attacher.ports().len(), 3);
    assert_eq!(attacher.ports()[0].local_bus_id.as_deref(), Some("1-1"));
    let port = attacher.attach(&socket, 65541, UsbSpeed::Full).unwrap();

    assert_eq!(port, 1);
    let line = fs::read_to_string(vhci.join("attach")).unwrap();
    let parts: Vec<&str> = line.split(' ').collect();
    assert_eq!(parts.len(), 4, "{line}");
    assert_eq!(parts[0], "1");
    assert!(parts[1].chars().all(|c| c.is_ascii_digit()));
    assert_eq!(&parts[2..], ["65541", "2"]);
}

#[tokio::test]
async fn vhci_puts_a_superspeed_device_on_the_superspeed_hub_and_detaches_by_port() {
    let sys = Sys::new();
    // The newer kernels keep the files under the driver.
    let vhci = sys
        .root()
        .join("bus")
        .join("platform")
        .join("drivers")
        .join("vhci_hcd");
    fs::create_dir_all(&vhci).unwrap();
    fs::write(vhci.join("attach"), "").unwrap();
    fs::write(
        vhci.join("status"),
        "hub port sta spd dev      sockfd local_busid\nhs  0001 004 000 00000000 000000 0-0\nss  0008 004 000 00000000 000000 0-0\n",
    )
    .unwrap();
    let attacher = SysfsVhci::new(sys.root());
    let (socket, _relay) = loopback_pair().await.unwrap();

    let port = attacher.attach(&socket, 1, UsbSpeed::Super).unwrap();
    attacher.detach(port).unwrap();

    assert_eq!(port, 8);
    assert_eq!(fs::read_to_string(vhci.join("detach")).unwrap(), "8");
}

#[test]
fn vhci_without_the_module_explains_what_to_load() {
    let sys = Sys::new();

    assert!(
        SysfsVhci::new(sys.root())
            .unavailable()
            .unwrap()
            .contains("modprobe vhci-hcd")
    );
}

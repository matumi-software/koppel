//! The kernel's own text, as the adapters read it. The fixtures are in the layout the kernel's
//! sources print (`vhci_sysfs.c` for the status files, the USB core's attribute files for a
//! device); they were written from those sources, not captured from a running kernel, so a
//! kernel that changes a layout is caught by comparing the two, as vhci.rs notes for the 2026
//! move of attach and detach under the driver. Linux only, like the code under test.
#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::path::Path;

use common::TempDir;
use koppel_usb::linux::{SysfsUsbHost, SysfsVhci};
use koppel_usb::{Hub, PortStatus, UsbAttacher, UsbHost, UsbSpeed};

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn the_vhci_status_files_of_a_kernel_read_port_by_port() {
    let sys = TempDir::new("golden-vhci");
    let controller = sys.path().join("devices/platform/vhci_hcd.0");
    fs::create_dir_all(&controller).unwrap();
    fs::write(controller.join("attach"), "").unwrap();
    fs::write(controller.join("status"), fixture("vhci/status")).unwrap();
    fs::write(controller.join("status.1"), fixture("vhci/status.1")).unwrap();

    let ports = SysfsVhci::new(sys.path()).ports();

    assert_eq!(ports.len(), 7);
    let first = &ports[0];
    assert_eq!(
        (first.port, first.hub, first.status),
        (0, Hub::HighSpeed, PortStatus::Null)
    );
    assert!(first.is_free());
    assert_eq!(first.local_bus_id, None);
    let used = &ports[1];
    assert_eq!(used.status, PortStatus::Used);
    assert_eq!(used.dev_id, 0x0001_0005);
    assert_eq!(used.local_bus_id.as_ref().map(|b| b.as_str()), Some("1-1"));
    let super_speed = ports.iter().find(|p| p.port == 9).unwrap();
    assert_eq!(super_speed.hub, Hub::SuperSpeed);
    assert_eq!(super_speed.dev_id, 0x0002_0003);
    // The second controller's file follows the first's.
    assert_eq!(ports[5].port, 16);
    assert_eq!(ports[6].port, 24);
}

#[test]
fn the_vhci_controller_is_found_under_the_driver_after_the_2026_kernel_change() {
    let sys = TempDir::new("golden-vhci-driver");
    let controller = sys.path().join("bus/platform/drivers/vhci_hcd");
    fs::create_dir_all(&controller).unwrap();
    fs::write(controller.join("attach"), "").unwrap();
    fs::write(controller.join("status"), fixture("vhci/status")).unwrap();

    assert!(SysfsVhci::new(sys.path()).unavailable().is_none());
    assert_eq!(SysfsVhci::new(sys.path()).ports().len(), 5);
}

/// Writes the sections of a manifest below `device`.
fn materialise(manifest: &str, device: &Path) {
    let mut sections = manifest.lines().filter(|l| !l.starts_with('#')).peekable();
    while let Some(line) = sections.next() {
        let Some(path) = line.strip_prefix("== ") else {
            continue;
        };
        let mut body = String::new();
        while let Some(next) = sections.peek() {
            if next.starts_with("== ") {
                break;
            }
            body.push_str(next);
            body.push('\n');
            sections.next();
        }
        let target = device.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, body).unwrap();
    }
}

#[tokio::test]
async fn a_device_as_the_kernel_prints_it_reads_into_every_field() {
    let sys = TempDir::new("golden-sysfs");
    let device = sys.path().join("bus/usb/devices/1-1.4");
    materialise(&fixture("sysfs/ftdi.txt"), &device);
    fs::create_dir_all(sys.path().join("bus/usb/drivers/usbip-host")).unwrap();

    let listed = SysfsUsbHost::new(sys.path(), sys.path().join("dev"))
        .list_devices()
        .await
        .unwrap();

    assert_eq!(listed.len(), 1);
    let d = &listed[0];
    assert_eq!(d.bus_id, "1-1.4");
    assert_eq!(d.vid_pid(), "0403:6001");
    assert_eq!(d.bcd_device, 0x0600);
    assert_eq!((d.bus_num, d.dev_num), (1, 5));
    assert_eq!(d.speed, UsbSpeed::Full);
    assert_eq!(d.configuration_value, 1);
    assert_eq!(d.num_configurations, 1);
    assert_eq!(d.manufacturer.as_deref(), Some("FTDI"));
    assert_eq!(d.product.as_deref(), Some("FT232R USB UART"));
    assert_eq!(d.serial.as_deref(), Some("A50285BI"));
    assert_eq!(d.driver.as_deref(), Some("ftdi_sio"));
    assert_eq!(d.interfaces.len(), 1);
    assert_eq!(d.interfaces[0].class, 0xff);
}

//! What the server decides to share: the saved choice, the device on the port, what the host
//! took, and a `devices.json` that cannot be read.

mod common;

use std::fs;
use std::sync::Arc;

use common::*;
use koppel_usb::{SetSharedError, UsbDevice};

fn other_device() -> UsbDevice {
    UsbDevice {
        vendor_id: 0x046d,
        product_id: 0xc548,
        product: Some("Other device".into()),
        ..dongle()
    }
}

#[tokio::test]
async fn a_different_device_on_a_shared_port_is_unbound_and_not_exportable() {
    let kernel = FakeKernel::new();
    kernel.plug(vec![other_device()]);
    let side = ServerSide::start(kernel.clone(), true, true).await;

    let devices = side.server.current();

    assert!(kernel.unshared().contains(&"1-1.4".to_owned()));
    let device = &devices.devices[0];
    assert_eq!(device.state, "local");
    assert!(
        device
            .fault
            .as_deref()
            .unwrap()
            .contains("different device")
    );
    assert!(side.server.exportable().await.unwrap().is_empty());
    side.stop().await;
}

#[tokio::test]
async fn a_device_bound_on_the_port_but_not_the_shared_one_is_not_exportable_even_before_it_is_unbound()
 {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;

    kernel.plug(vec![other_device()]);
    kernel.bind("1-1.4", true);

    assert!(side.server.exportable().await.unwrap().is_empty());
    side.stop().await;
}

#[tokio::test]
async fn a_bad_bus_id_is_refused_and_not_saved() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), false, true).await;

    for bus_id in [
        "1-1\n",
        "abc",
        "\u{6f1}-1",
        "1-1.4.5.6.7.8.9.10.11.12.13.14.15.16.17.18.19.20",
    ] {
        let refused = side.server.set_shared(bus_id, true).await;
        assert!(
            matches!(refused, Err(SetSharedError::InvalidBusId)),
            "{bus_id:?}"
        );
    }

    assert!(kernel.shared().is_empty());
    let saved = fs::read_to_string(side.config_file("devices.json")).unwrap();
    assert!(!saved.contains("abc") && !saved.contains("1-1\\n"));
    side.stop().await;
}

#[tokio::test]
async fn a_gone_device_shows_its_remembered_vid_pid_as_unplugged() {
    let kernel = FakeKernel::new();
    kernel.bind("1-1.4", false);
    let dir = server_dir(
        true,
        Some(
            r#"{ "busIds": ["1-1.4", "3-2"], "vidPids": { "1-1.4": "064f:2af9", "3-2": "046d:c548" } }"#,
        ),
    );
    let side = ServerSide::open(dir, kernel).await;

    let devices = side.server.current();

    let gone = devices.devices.iter().find(|d| d.bus_id == "3-2").unwrap();
    assert_eq!(gone.state, "offline");
    assert_eq!(gone.vid_pid, "046d:c548");
    side.stop().await;
}

#[tokio::test]
async fn a_remembered_device_that_moved_to_another_saved_port_is_not_also_listed_as_unplugged() {
    let kernel = FakeKernel::new();
    kernel.bind("1-1.4", false);
    // The dongle was saved at 3-1 first, then moved to 1-1.4 and saved there.
    let dir = server_dir(
        true,
        Some(
            r#"{ "busIds": ["3-1", "1-1.4"], "vidPids": { "3-1": "064f:2af9", "1-1.4": "064f:2af9" } }"#,
        ),
    );
    let side = ServerSide::open(dir, kernel).await;

    let listed: Vec<String> = side
        .server
        .current()
        .devices
        .iter()
        .map(|d| d.bus_id.clone())
        .collect();

    assert_eq!(listed, vec!["1-1.4"]);
    side.stop().await;
}

#[tokio::test]
async fn a_share_the_kernel_refused_is_not_saved_and_shows_as_a_fault() {
    let kernel = FakeKernel::new();
    kernel.bind("1-1.4", false);
    kernel.set_share_failure(Some("The kernel refused it."));
    let side = ServerSide::start(kernel.clone(), false, true).await;

    side.server.set_shared("1-1.4", true).await.unwrap();

    assert!(
        !fs::read_to_string(side.config_file("devices.json"))
            .unwrap()
            .contains("1-1.4")
    );
    assert_eq!(
        side.server.current().devices[0].fault.as_deref(),
        Some("The kernel refused it.")
    );

    // Once the kernel agrees, the share is saved.
    kernel.set_share_failure(None);
    side.server.set_shared("1-1.4", true).await.unwrap();
    assert!(
        fs::read_to_string(side.config_file("devices.json"))
            .unwrap()
            .contains("1-1.4")
    );
    assert!(kernel.is_bound("1-1.4"));
    side.stop().await;
}

#[tokio::test]
async fn start_up_unbinds_devices_bound_to_usbip_host_that_were_never_shared() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), false, true).await;

    assert!(!kernel.is_bound("1-1.4"));
    assert_eq!(side.server.current().devices[0].state, "local");
    side.stop().await;
}

#[tokio::test]
async fn start_up_unbinds_saved_shares_when_share_on_startup_is_off() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, false).await;

    assert!(!kernel.is_bound("1-1.4"));
    assert_eq!(side.server.current().devices[0].state, "local");
    assert!(
        fs::read_to_string(side.config_file("devices.json"))
            .unwrap()
            .contains("1-1.4"),
        "the choice is kept for when the user shares it again"
    );
    side.stop().await;
}

#[tokio::test]
async fn the_poll_survives_a_host_that_fails_and_picks_up_changes_afterwards() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;
    let plugged = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = plugged.clone();
    side.server.on_changed(move |devices| {
        if devices.devices.iter().any(|d| d.bus_id == "2-1") {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        async {}
    });

    kernel.fail_listings("sysfs said no", 1);
    eventually("a failed listing", || kernel.listing_failed()).await;
    kernel.plug(vec![
        dongle(),
        UsbDevice {
            bus_id: "2-1".into(),
            ..dongle()
        },
    ]);

    eventually("the plugged device to be listed", || {
        plugged.load(std::sync::atomic::Ordering::SeqCst)
    })
    .await;
    side.stop().await;
}

#[tokio::test]
async fn a_host_error_at_start_up_does_not_stop_the_server_and_the_saved_share_is_applied_by_the_poll()
 {
    let kernel = FakeKernel::new();
    kernel.bind("1-1.4", false);
    kernel.fail_listings("sysfs is busy", 1);

    // It must come up although the first look at the kernel failed.
    let side = ServerSide::start(kernel.clone(), true, true).await;

    eventually("the saved share", || {
        side.server
            .current()
            .devices
            .iter()
            .any(|d| d.state == "shared")
    })
    .await;
    assert!(kernel.is_bound("1-1.4"));
    side.stop().await;
}

#[tokio::test]
async fn start_up_keeps_what_was_shared_and_saved() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;

    assert!(kernel.is_bound("1-1.4"));
    assert!(kernel.unshared().is_empty());
    assert_eq!(side.server.current().devices[0].state, "shared");
    side.stop().await;
}

#[tokio::test]
async fn plugging_and_unplugging_reach_the_activity_log() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;

    kernel.plug(vec![]);
    eventually("the unplug", || {
        side.activity
            .events()
            .iter()
            .any(|(_, _, summary)| summary == "CodeMeter-Stick was unplugged from bus 1-1.4")
    })
    .await;
    side.stop().await;
}

const CORRUPT: &str = "{ not json";

#[tokio::test]
async fn a_corrupt_devices_file_does_not_stop_the_start_and_says_why() {
    let kernel = FakeKernel::new();
    let side = ServerSide::open(server_dir(true, Some(CORRUPT)), kernel).await;

    let problem = side
        .server
        .shared_problem()
        .expect("the problem is reported");

    assert!(problem.contains("not valid JSON"), "{problem}");
    assert_eq!(
        fs::read_to_string(side.config_file("devices.json")).unwrap(),
        CORRUPT,
        "the file is left as it was until the user changes something"
    );
    side.stop().await;
}

#[tokio::test]
async fn a_healthy_server_has_no_devices_file_problem() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;

    assert_eq!(side.server.shared_problem(), None);
    side.stop().await;
}

#[tokio::test]
async fn sharing_a_device_over_a_corrupt_devices_file_moves_it_aside_and_writes_a_fresh_one() {
    let kernel = FakeKernel::new();
    let side = ServerSide::open(server_dir(true, Some(CORRUPT)), kernel).await;

    side.server.set_shared("1-1.4", true).await.unwrap();

    assert!(
        fs::read_to_string(side.config_file("devices.json"))
            .unwrap()
            .contains("1-1.4")
    );
    let aside: Vec<_> = fs::read_dir(side.config_file(""))
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("devices.json.unreadable-")
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read_to_string(aside[0].path()).unwrap(), CORRUPT);
    assert_eq!(side.server.shared_problem(), None);
    side.stop().await;
}

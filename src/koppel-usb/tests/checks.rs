//! The USB lines of the system checks.

mod common;

use std::sync::Arc;

use common::*;
use koppel_usb::{
    BoxFuture, Unsupported, UsbError, UsbIpDriver, UsbIpUpstreamHost, attach_check, share_check,
};

const MISSING: &str =
    "The Windows USB driver is not installed (usbip.exe is missing). Reinstall Koppel to add it.";

struct WorkingDriver;

impl UsbIpDriver for WorkingDriver {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn attach<'a>(
        &'a self,
        _: &'a str,
        _: u16,
        _: &'a koppel_usb::BusId,
    ) -> BoxFuture<'a, Result<u32, UsbError>> {
        Box::pin(async { Ok(0) })
    }

    fn detach<'a>(
        &'a self,
        _: &'a str,
        _: u16,
        _: &'a koppel_usb::BusId,
        _: u32,
    ) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn attach_on_native_windows_passes_when_the_driver_is_installed() {
    let check = attach_check(&Unsupported::new(MISSING), &WorkingDriver, true);

    assert!(check.ok);
    assert!(check.title.contains("usbip-win2"));
}

#[test]
fn attach_on_native_windows_fails_without_the_driver_and_says_to_reinstall() {
    let check = attach_check(&*FakeKernel::new(), &Unsupported::new(MISSING), true);

    assert!(!check.ok);
    assert!(check.detail.contains("Reinstall Koppel"));
}

#[test]
fn attach_off_windows_still_reads_the_attacher() {
    assert!(!attach_check(&Unsupported::new(MISSING), &WorkingDriver, false).ok);
    assert_eq!(
        attach_check(&*FakeKernel::new(), &Unsupported::new(MISSING), false).title,
        "vhci-hcd is loaded."
    );
}

#[test]
fn share_on_native_windows_without_an_upstream_names_usbipd_win_and_the_setting() {
    let check = share_check(&Unsupported::new(MISSING), true);

    assert!(!check.ok);
    assert!(check.detail.contains("usbipd-win") && check.detail.contains("Usb:Upstream"));
}

#[test]
fn share_off_windows_keeps_the_host_message() {
    assert_eq!(
        share_check(&Unsupported::new(MISSING), false).detail,
        MISSING
    );
    assert_eq!(
        share_check(&*FakeKernel::new(), false).title,
        "usbip-host is loaded."
    );
}

#[tokio::test]
async fn share_on_native_windows_through_a_reachable_upstream_says_so() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let _ = koppel_usb::protocol::read_header(&mut tcp).await;
            let _ = tokio::io::AsyncWriteExt::write_all(
                &mut tcp,
                &koppel_usb::protocol::dev_list_reply(&[]),
            )
            .await;
        }
    });
    let host = UsbIpUpstreamHost::new("127.0.0.1", port, Arc::new(koppel_usb::SystemClock));
    koppel_usb::UsbHost::list_devices(&host).await.unwrap();

    let check = share_check(&host, true);

    assert!(check.ok);
    assert_eq!(check.title, "usbipd-win is reachable.");
}

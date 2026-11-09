//! The client on Windows, where a driver on the machine makes its own connection to Koppel's
//! gateway: what the gateway serves, and what happens when the person changes their mind while
//! the driver is working. A fake driver plays usbip-win2. Renewing the link before its token
//! ends is the transport's, and tested with it.

mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use koppel_config::files::Role;
use koppel_usb::protocol;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn bench(max_sessions: Option<usize>) -> (ServerSide, Arc<FakeDriver>, ClientSide) {
    let server = ServerSide::start(FakeKernel::new(), true, true).await;
    let driver = FakeDriver::new();
    let mut setup = ClientSetup::driver(driver.clone());
    if let Some(max) = max_sessions {
        setup.gateway.max_sessions = max;
    }
    let client = ClientSide::start(MemoryLinks::new(&server), setup).await;
    (server, driver, client)
}

#[tokio::test(flavor = "multi_thread")]
async fn detaching_while_the_driver_is_still_attaching_releases_what_it_makes() {
    let (server, driver, client) = bench(None).await;
    let gate = driver.hold_attaches();
    client.link().await;
    client.client.begin_attach("1-1.4").await.unwrap();
    let _connection = driver.connection().await;

    // usbip.exe has not returned yet, so Koppel does not know the driver's port.
    client.client.detach("1-1.4").await;
    assert!(driver.detached().is_empty());
    gate.open();

    eventually("the driver's detach", || driver.detached().len() == 1).await;
    assert_eq!(driver.detached(), vec!["1-1.4@1"]);
    assert!(!client.attached_here());
    assert!(
        !fs::read_to_string(client.config_file("attached.json"))
            .unwrap()
            .contains("1-1.4")
    );

    // Nothing is left for a stop to release, so the driver hears of this device once.
    client.client.stop().await;
    assert_eq!(driver.detached(), vec!["1-1.4@1"]);
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_nobody_asked_the_driver_for_is_refused() {
    let (server, driver, client) = bench(None).await;
    client.link().await;
    let port = client.client.gateway_port().expect("the gateway listens");

    // Another user's "usbip attach", or the driver trying again by itself: Koppel did not ask.
    let mut visitor = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    visitor
        .write_all(&protocol::import_request("1-1.4"))
        .await
        .unwrap();
    assert_ne!(
        protocol::read_header(&mut visitor).await.unwrap().status,
        protocol::STATUS_OK
    );

    let mut list = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    list.write_all(&protocol::dev_list_request()).await.unwrap();
    assert_ne!(
        protocol::read_header(&mut list).await.unwrap().status,
        protocol::STATUS_OK
    );

    assert!(!client.attached_here());
    assert_eq!(driver.attaches(), 0);
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remembered_device_is_attached_again_after_the_link_drops_because_koppel_asks_for_it() {
    let (server, driver, client) = bench(None).await;
    client.link().await;
    client.client.begin_attach("1-1.4").await.unwrap();
    eventually("the first attach", || {
        driver.attaches() == 1 && client.attached_here()
    })
    .await;

    // The connection goes away; the driver would try again by itself and is refused, Koppel asks anew.
    driver.drop_connection();

    eventually("the second attach", || {
        driver.attaches() == 2 && client.attached_here()
    })
    .await;
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_role_change_returns_at_once_while_the_driver_is_still_detaching() {
    let (server, driver, client) = bench(None).await;
    client.link().await;
    client.client.begin_attach("1-1.4").await.unwrap();
    eventually("the attach", || client.attached_here()).await;
    let gate = driver.hold_detaches();

    // usbip.exe may take 30 seconds; the settings update, and the confirmation the UI waits for, must not.
    tokio::time::timeout(Duration::from_secs(10), client.set_role(Role::Server))
        .await
        .expect("the settings change returns");
    eventually("the driver's detach", || driver.detached().len() == 1).await;

    gate.open();
    eventually("the device to go", || !client.attached_here()).await;
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gateway_serves_at_most_the_configured_number_of_sessions() {
    let (server, _, client) = bench(Some(2)).await;
    client.link().await;
    let port = client.client.gateway_port().expect("the gateway listens");

    // Two connections that say nothing hold both places.
    let mut first = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let _second = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    eventually_async("a third connection to be closed at once", || async {
        let mut third = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut byte = [0u8; 1];
        matches!(
            tokio::time::timeout(Duration::from_secs(2), third.read(&mut byte)).await,
            Ok(Ok(0) | Err(_))
        )
    })
    .await;

    let mut byte = [0u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(200), first.read(&mut byte))
            .await
            .is_err(),
        "the first connection is still open"
    );
    client.stop().await;
    server.stop().await;
}

//! A server and a client joined in memory, with a fake kernel on each side. Covers everything
//! except the kernel and the transport: the device list, the USB/IP import, the deadline and the
//! relay on both ends. The transport's own part (tokens, pinning, TLS) is Skakel's and is tested
//! there.

mod common;

use std::fs;

use common::*;
use koppel_usb::protocol;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

async fn bench() -> (
    std::sync::Arc<FakeKernel>,
    ServerSide,
    std::sync::Arc<FakeKernel>,
    ClientSide,
) {
    let server_kernel = FakeKernel::new();
    let server = ServerSide::start(server_kernel.clone(), true, true).await;
    let client_kernel = FakeKernel::new();
    let client = ClientSide::start(
        MemoryLinks::new(&server),
        ClientSetup::kernel(client_kernel.clone()).with_gateway(),
    )
    .await;
    (server_kernel, server, client_kernel, client)
}

#[tokio::test(flavor = "multi_thread")]
async fn client_attaches_a_shared_device_and_usb_traffic_round_trips() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;
    assert_eq!(
        client.client.current().devices[0].device.vid_pid,
        "064f:2af9"
    );

    client.attach().await;

    // The test plays the client's USB driver: bytes go out through the stream and the fake
    // server kernel echoes them.
    let mut driver = client_kernel.driver_stream(1).await;
    echoes(&mut driver).await;
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn client_attaches_a_device_again_after_its_link_drops() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;
    client.attach().await;
    let first = client_kernel.attached_socket(1).await;

    // The link to the server drops, as when the server restarts.
    first.shutdown(std::net::Shutdown::Both).unwrap();

    let mut again = client_kernel.driver_stream(2).await;
    echoes(&mut again).await;
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn client_remembers_what_it_attached_until_it_is_detached() {
    let (_, server, _, client) = bench().await;
    client.link().await;
    client.attach().await;
    assert!(
        fs::read_to_string(client.config_file("attached.json"))
            .unwrap()
            .contains("1-1.4")
    );

    client.client.detach("1-1.4").await;

    assert!(
        !fs::read_to_string(client.config_file("attached.json"))
            .unwrap()
            .contains("1-1.4")
    );
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_resets_the_device_before_a_client_gets_it() {
    let (server_kernel, server, _, client) = bench().await;
    client.link().await;

    client.attach().await;

    // Drivers on the server may have changed the device before it was shared; a reset undoes that.
    assert!(server_kernel.reset_before_export());
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_lets_a_usbip_driver_on_the_client_machine_list_and_attach() {
    let (_, server, _, client) = bench().await;
    client.link().await;
    let port = client.client.gateway_port().expect("the gateway listens");
    assert!(client.client.current().gateway);

    // The test plays usbip-win2: "usbip list -r 127.0.0.1", then "usbip attach -r 127.0.0.1 -b 1-1.4".
    {
        let mut list = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        list.write_all(&protocol::dev_list_request()).await.unwrap();
        let header = protocol::read_header(&mut list).await.unwrap();
        assert_eq!(header.code, protocol::OP_REP_DEVLIST);
        assert_eq!(header.status, protocol::STATUS_OK);
        let listed = protocol::read_dev_list(&mut list, 10, 4096).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].bus_id, "1-1.4");
        assert_eq!(listed[0].interfaces, dongle().interfaces);
    }

    let mut usb = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    usb.write_all(&protocol::import_request("1-1.4"))
        .await
        .unwrap();
    let reply = protocol::read_header(&mut usb).await.unwrap();
    assert_eq!(reply.status, protocol::STATUS_OK);
    let device = protocol::read_imported_device(&mut usb).await.unwrap();
    assert_eq!(device.vid_pid(), "064f:2af9");
    assert_eq!(device.dev_id(), dongle().dev_id());

    eventually("the gateway attach to show", || {
        client.client.current().devices[0].attached_via.is_some()
    })
    .await;
    let entry = client.client.current().devices[0].clone();
    assert!(entry.attached_here);
    assert_eq!(entry.port, None);
    echoes(&mut usb).await;
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn with_a_driver_attach_and_detach_drive_it_through_the_gateway() {
    let server = ServerSide::start(FakeKernel::new(), true, true).await;
    let driver = FakeDriver::new();
    let client = ClientSide::start(
        MemoryLinks::new(&server),
        ClientSetup::driver(driver.clone()),
    )
    .await;
    client.link().await;
    assert!(
        !client.client.current().gateway,
        "Koppel drives the driver itself, so the UI needs no usbip command"
    );

    client.client.begin_attach("1-1.4").await.unwrap();
    eventually("the driver's port", || {
        client.client.current().devices[0].port.is_some()
    })
    .await;
    let entry = client.client.current().devices[0].clone();
    assert!(entry.attached_here);
    assert_eq!(entry.port, Some(FakeDriver::DRIVER_PORT));
    assert_eq!(entry.attached_via, None);
    let mut connection = driver.connection().await;
    echoes(&mut connection).await;

    client.client.detach("1-1.4").await;
    eventually("the detach", || !client.attached_here()).await;
    assert_eq!(driver.detached(), vec!["1-1.4@1"]);
    client.stop().await;
    server.stop().await;
}

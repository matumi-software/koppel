//! What the client does when the server, the link or its own settings misbehave: faults a person
//! can read, the deadline, no restart for an unchanged setting, detaching on a change, a gateway
//! that cannot bind, and stopping.
//!
//! Most of these run on tokio's paused clock with nothing real under them: the server is in memory
//! (`MemoryLinks`) and the client kernel takes a pipe in memory (`FakeKernel::in_memory`), so what
//! `eventually` and the deadlines wait for is virtual time and a test takes milliseconds. A paused
//! clock jumps ahead whenever the runtime waits for a real socket, which is why the few tests that
//! need one (a kernel attach over loopback, the gateway, the USB/IP driver) keep real time and
//! `flavor = "multi_thread"`.

mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use koppel_config::files::{KoppelSettings, Role};
use koppel_usb::{BeginAttachError, ClientOptions, GatewayOptions, OpenError};
use tokio::time::Instant;

/// A server and a client whose kernel takes a pipe in memory. No socket, so it runs on the paused clock.
async fn bench() -> (Arc<FakeKernel>, ServerSide, Arc<FakeKernel>, ClientSide) {
    let server_kernel = FakeKernel::new();
    let server = ServerSide::start(server_kernel.clone(), true, true).await;
    let client_kernel = FakeKernel::in_memory();
    let mut setup = ClientSetup::kernel(client_kernel.clone());
    setup.options = ClientOptions {
        attach_timeout: Duration::from_secs(3),
        ..ClientOptions::default()
    };
    let client = ClientSide::start(MemoryLinks::new(&server), setup).await;
    (server_kernel, server, client_kernel, client)
}

/// As [`bench`], with a kernel that takes a real loopback socket and the gateway on a port of its own.
async fn bench_real() -> (Arc<FakeKernel>, ServerSide, Arc<FakeKernel>, ClientSide) {
    let server_kernel = FakeKernel::new();
    let server = ServerSide::start(server_kernel.clone(), true, true).await;
    let client_kernel = FakeKernel::new();
    let mut setup = ClientSetup::kernel(client_kernel.clone()).with_gateway();
    setup.options = ClientOptions {
        attach_timeout: Duration::from_secs(3),
        ..ClientOptions::default()
    };
    let client = ClientSide::start(MemoryLinks::new(&server), setup).await;
    (server_kernel, server, client_kernel, client)
}

#[tokio::test(start_paused = true)]
async fn attach_to_a_server_that_went_away_records_a_fault() {
    let (_, server, _, client) = bench().await;
    client.link().await;

    // The server is switched off.
    client.links.take_down();
    client.client.begin_attach("1-1.4").await.unwrap();

    eventually("the fault", || client.fault().is_some()).await;
    let fault = client.fault().unwrap();
    assert!(
        fault.contains("Could not reach the server") || fault.contains("did not answer within"),
        "{fault}"
    );
    assert!(!client.attached_here());
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn attach_to_a_server_that_stalls_gives_up_at_the_deadline() {
    let (server_kernel, server, _, client) = bench().await;
    client.link().await;
    let gate = server_kernel.hold_resets();

    // The server lets the client in and then says nothing.
    client.client.begin_attach("1-1.4").await.unwrap();

    eventually("the fault", || client.fault().is_some()).await;
    assert!(
        client
            .fault()
            .unwrap()
            .contains("did not answer within 3 seconds")
    );
    gate.open();
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn attaches_and_the_device_list_share_one_link_until_the_connection_changes() {
    let (_, server, _, client) = bench().await;
    client.link().await;
    let before = client.client.link().unwrap();
    let made = client.links.made();

    client.attach().await;
    assert!(
        Arc::ptr_eq(&client.client.link().unwrap(), &before),
        "the attach opened its stream on the link"
    );
    assert_eq!(client.links.made(), made);

    // New credentials are a new connection, so they get their own link.
    client.link().await;
    assert!(!Arc::ptr_eq(&client.client.link().unwrap(), &before));
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn changing_the_connection_detaches_what_was_attached() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;
    client.attach().await;
    assert!(client_kernel.detached_ports().is_empty());

    client.link().await;

    eventually("the detach", || !client_kernel.detached_ports().is_empty()).await;
    assert!(client_kernel.detached_ports().contains(&8));
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_setting_that_did_not_change_the_link_leaves_it_and_its_devices_alone() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;
    client.attach().await;
    let applied = client.client.changes_applied();

    client
        .settings
        .update(|s| KoppelSettings {
            share_on_startup: false,
            ..s.clone()
        })
        .await
        .unwrap();

    // The update returns after every listener has run, so an applied change would be counted by now.
    assert_eq!(
        client.client.changes_applied(),
        applied,
        "the link keeps running when neither the role nor the connection changed"
    );
    assert!(client_kernel.detached_ports().is_empty());
    assert!(client.attached_here());
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn detach_and_attach_again_leaves_the_new_attachment_in_place() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;
    client.attach().await;

    // The old attach's relay ends while the next one is being set up. It may only clean up its
    // own entry, or it would remove the new attachment and detach the new port (the fake kernel
    // hands out port 8 every time).
    let cycles = 3;
    for cycle in 1..=cycles {
        client.client.detach("1-1.4").await;
        let expected = cycle + 1;
        eventually_async("the attach again", || async {
            let _ = client.client.begin_attach("1-1.4").await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            client_kernel.attaches() == expected
        })
        .await;
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(client_kernel.detached_ports().len(), cycles);
    assert!(client.attached_here());
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stopping_the_client_does_not_wait_for_its_relays_and_keeps_the_device_remembered() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;
    client.attach().await;

    let started = Instant::now();
    client.client.stop().await;

    // A relay nobody stopped made the stop wait out its five seconds.
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(client_kernel.detached_ports().contains(&8));
    assert!(
        fs::read_to_string(client.config_file("attached.json"))
            .unwrap()
            .contains("1-1.4")
    );
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_detach_that_cannot_be_saved_is_a_fault_on_the_device_and_the_device_is_detached() {
    let server = ServerSide::start(FakeKernel::new(), true, true).await;
    let client_kernel = FakeKernel::in_memory();
    let codec = Arc::new(FailingCodec::default());
    let mut setup = ClientSetup::kernel(client_kernel.clone());
    setup.attached_codec = codec.clone();
    let client = ClientSide::start(MemoryLinks::new(&server), setup).await;
    client.link().await;
    client.attach().await;
    // 0.1.x's test put a folder where the file belongs. koppel-config moves an unusable
    // attached.json aside and writes a fresh one, folder or not, so the save fails here instead.
    codec.0.store(true, std::sync::atomic::Ordering::SeqCst);

    client.client.detach("1-1.4").await;

    assert!(client_kernel.detached_ports().contains(&8));
    assert!(!client.attached_here());
    assert!(client.fault().unwrap().contains("could not save"));
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn attaching_a_device_that_is_attached_or_on_its_way_does_nothing() {
    let (_, server, client_kernel, client) = bench().await;
    client.link().await;

    // Twice at once, while the first is still in flight; then again once it is attached.
    let (first, second) = tokio::join!(
        client.client.begin_attach("1-1.4"),
        client.client.begin_attach("1-1.4")
    );
    first.unwrap();
    second.unwrap();
    eventually("the attach", || client.attached_here()).await;
    client.client.begin_attach("1-1.4").await.unwrap();

    // The request has been answered, so an attach it started would be counted by now.
    assert_eq!(client.client.attach_starts(), 1);
    assert_eq!(client_kernel.attaches(), 1);
    assert_eq!(client.fault(), None);
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_bus_id_that_is_not_one_is_refused() {
    let (_, server, _, client) = bench().await;

    let refused = client.client.begin_attach("1-1.4;calc").await;

    assert!(matches!(refused, Err(BeginAttachError::InvalidBusId)));
    client.stop().await;
    server.stop().await;
}

#[test]
fn a_refused_stream_is_described_for_a_person() {
    assert_eq!(
        OpenError::Refused("the client is not registered".into()).describe(),
        "The server refused the USB connection: the client is not registered"
    );
    assert_eq!(
        OpenError::Failed("Could not reach the server.".into()).describe(),
        "Could not reach the server."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gateway_port_that_is_taken_does_not_stop_the_client_and_shows_in_the_client_list() {
    let server = ServerSide::start(FakeKernel::new(), true, true).await;
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let mut setup = ClientSetup::kernel(FakeKernel::new());
    setup.role = Role::Server;
    setup.gateway = GatewayOptions {
        port,
        ..GatewayOptions::default()
    };
    let client = ClientSide::start(MemoryLinks::new(&server), setup).await;

    // Only a client wants the gateway.
    client.set_role(Role::Client).await;

    eventually("the gateway fault", || {
        client.client.current().gateway_fault.is_some()
    })
    .await;
    let current = client.client.current();
    assert!(current.gateway_fault.unwrap().contains(&port.to_string()));
    assert!(!current.gateway);

    // Once the port is free the gateway binds on its own.
    drop(blocker);
    eventually("the gateway to listen", || client.client.current().gateway).await;

    // A server has no gateway.
    client.set_role(Role::Server).await;
    eventually("the gateway to stop", || !client.client.current().gateway).await;
    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_client_detaches_what_the_driver_holds() {
    let server = ServerSide::start(FakeKernel::new(), true, true).await;
    let driver = FakeDriver::new();
    let client = ClientSide::start(
        MemoryLinks::new(&server),
        ClientSetup::driver(driver.clone()),
    )
    .await;
    client.link().await;
    client.client.begin_attach("1-1.4").await.unwrap();
    eventually("the driver's port", || {
        client.client.current().devices[0].port.is_some()
    })
    .await;

    client.client.stop().await;

    assert_eq!(driver.detached(), vec!["1-1.4@1"]);
    assert!(
        fs::read_to_string(client.config_file("attached.json"))
            .unwrap()
            .contains("1-1.4"),
        "stopping does not forget the device"
    );
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_corrupt_attached_file_does_not_stop_the_start_and_says_why() {
    let server = ServerSide::start(FakeKernel::new(), true, true).await;
    let dir = TempDir::new("client");
    fs::create_dir_all(dir.path().join("config")).unwrap();
    fs::write(
        dir.path().join("config").join("attached.json"),
        "{ not json",
    )
    .unwrap();

    let client = ClientSide::start_in(
        dir,
        MemoryLinks::new(&server),
        ClientSetup::kernel(FakeKernel::new()),
    )
    .await;

    let problem = client
        .client
        .remembered_problem()
        .expect("the problem is reported");
    assert!(problem.contains("not valid JSON"), "{problem}");
    assert_eq!(
        fs::read_to_string(client.config_file("attached.json")).unwrap(),
        "{ not json"
    );
    client.stop().await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn linking_to_the_server_reaches_the_activity_log() {
    let (_, server, _, client) = bench().await;

    client.link().await;

    let events = client.activity.events();
    assert!(
        events
            .iter()
            .any(|(_, _, summary)| summary == "Linked to server.test")
    );
    client.stop().await;
    server.stop().await;
}

/// The same as the paused-clock test of this name, over a real loopback socket for the kernel's
/// end and the gateway listening: what an attach really does on a machine.
#[tokio::test(flavor = "multi_thread")]
async fn changing_the_connection_detaches_what_was_attached_over_a_real_socket() {
    let (_, server, client_kernel, client) = bench_real().await;
    client.link().await;
    client.attach().await;
    assert!(client_kernel.detached_ports().is_empty());

    client.link().await;

    eventually("the detach", || !client_kernel.detached_ports().is_empty()).await;
    assert!(client_kernel.detached_ports().contains(&8));
    client.stop().await;
    server.stop().await;
}

/// Detach and attach again, three times over real sockets: the old attach's relay must not remove
/// the new attachment, whichever way the real ends of the sockets close first.
#[tokio::test(flavor = "multi_thread")]
async fn detach_and_attach_again_over_real_sockets_leaves_the_new_attachment_in_place() {
    let (_, server, client_kernel, client) = bench_real().await;
    client.link().await;
    client.attach().await;

    let cycles = 3;
    for cycle in 1..=cycles {
        client.client.detach("1-1.4").await;
        let expected = cycle + 1;
        eventually_async("the attach again", || async {
            let _ = client.client.begin_attach("1-1.4").await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            client_kernel.attaches() == expected
        })
        .await;
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(client_kernel.detached_ports().len(), cycles);
    assert!(client.attached_here());
    client.stop().await;
    server.stop().await;
}

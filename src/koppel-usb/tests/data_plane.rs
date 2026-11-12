//! The server's data plane: what a role change does to live sessions and shares, and why a slow
//! listener or a second import cannot get in the way. Who may open a stream at all (the token,
//! the port, the role check before the stream opens) is the transport's, and tested with it.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use koppel_config::files::Role;
use koppel_usb::stream::memory::MemoryStream;
use koppel_usb::{CloseReason, ServerDevices, protocol};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Imports the device on a stream, as a client would, and expects it to be granted.
async fn import(stream: &mut MemoryStream, bus_id: &str) {
    stream
        .write_all(&protocol::import_request(&bus_id.parse().unwrap()))
        .await
        .unwrap();
    let header = protocol::read_header(stream).await.unwrap();
    assert_eq!(header.status, protocol::STATUS_OK);
    protocol::read_imported_device(stream).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_the_server_role_unshares_what_was_shared_and_coming_back_shares_again() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;
    assert!(kernel.is_bound("1-1.4"));

    side.set_role(Role::Client).await;
    eventually("the unshare", || !kernel.is_bound("1-1.4")).await;

    side.set_role(Role::Server).await;
    eventually("the share", || kernel.is_bound("1-1.4")).await;
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_the_server_role_ends_live_sessions() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;
    let mut session = side.open_stream("bench-2");
    let closed = session.peer_close();
    import(&mut session, "1-1.4").await;

    side.set_role(Role::Client).await;

    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        let mut buffer = [0u8; 16];
        while session.read(&mut buffer).await.unwrap_or(0) > 0 {}
    })
    .await;
    assert!(ended.is_ok(), "the session ends");
    assert_eq!(closed.closed().await, CloseReason::Stopping);
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listener_of_the_device_list_that_never_returns_does_not_stall_sharing_or_imports() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;
    let stuck = Gate::default();
    let held = stuck.clone();
    side.server.on_changed(move |_| {
        let held = held.clone();
        async move { held.wait().await }
    });

    let within = Duration::from_secs(10);
    tokio::time::timeout(within, side.server.set_shared("1-1.4", false))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(within, side.server.set_shared("1-1.4", true))
        .await
        .unwrap()
        .unwrap();
    let mut session = side.open_stream("bench-3");
    tokio::time::timeout(within, import(&mut session, "1-1.4"))
        .await
        .unwrap();

    echoes(&mut session).await;
    stuck.open();
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshots_are_delivered_in_order_and_the_ones_a_newer_one_overtook_are_dropped() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;
    let first = Gate::default();
    let release = Gate::default();
    let delivered: Arc<Mutex<Vec<Arc<ServerDevices>>>> = Arc::default();
    {
        let (first, release, delivered) = (first.clone(), release.clone(), delivered.clone());
        side.server.on_changed(move |devices| {
            lock(&delivered).push(devices);
            first.open();
            let release = release.clone();
            async move { release.wait().await }
        });
    }
    side.server.set_shared("1-1.4", false).await.unwrap();
    first.wait().await;

    // The listener is busy with the first snapshot, so these queue up behind it.
    for shared in [true, false, true] {
        side.server.set_shared("1-1.4", shared).await.unwrap();
    }

    let last = side.server.current();
    release.open();
    eventually("the newest snapshot", || {
        let delivered = lock(&delivered);
        delivered.len() >= 2 && Arc::ptr_eq(delivered.last().unwrap(), &last)
    })
    .await;
    assert!(
        lock(&delivered).len() <= 3,
        "the snapshots between the first and the newest were overtaken and dropped"
    );
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_imports_of_one_device_do_not_reset_it_at_the_same_time() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;
    let gate = kernel.hold_resets();
    let mut first = side.open_stream("bench-4");
    let mut second = side.open_stream("bench-4");

    first
        .write_all(&protocol::import_request(&"1-1.4".parse().unwrap()))
        .await
        .unwrap();
    assert!(kernel.reset_started(Duration::from_secs(10)).await);

    // The first import is inside its reset. The second must wait for it; without the lock it would
    // reset too. A second reset is the only thing that can happen here, so the wait is bounded by
    // how long to look for it.
    second
        .write_all(&protocol::import_request(&"1-1.4".parse().unwrap()))
        .await
        .unwrap();
    assert!(
        !kernel.reset_started(Duration::from_secs(1)).await,
        "the second import waits for the first one's reset"
    );
    gate.open();

    assert_eq!(
        protocol::read_header(&mut first).await.unwrap().status,
        protocol::STATUS_OK
    );
    assert_ne!(
        protocol::read_header(&mut second).await.unwrap().status,
        protocol::STATUS_OK,
        "the device is in use by then"
    );
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_the_server_role_is_confirmed_before_the_devices_are_given_back() {
    let kernel = FakeKernel::new();
    let side = ServerSide::start(kernel.clone(), true, true).await;
    let gate = kernel.hold_unshares();

    // The unshare is blocked, and the settings change still returns.
    tokio::time::timeout(Duration::from_secs(10), side.set_role(Role::Client))
        .await
        .expect("the settings change returns");
    assert!(kernel.is_bound("1-1.4"), "the teardown has not finished");
    gate.open();

    eventually("the unshare", || !kernel.is_bound("1-1.4")).await;
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_list_request_gets_the_exportable_devices() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;
    let mut session = side.open_stream("bench-5");

    session
        .write_all(&protocol::dev_list_request())
        .await
        .unwrap();
    let header = protocol::read_header(&mut session).await.unwrap();
    let devices = protocol::read_dev_list(&mut session, 10, 4096)
        .await
        .unwrap();

    assert_eq!(header.code, protocol::OP_REP_DEVLIST);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].bus_id, "1-1.4");
    let closed = session.peer_close();
    assert_eq!(closed.closed().await, CloseReason::Done);
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_of_a_bus_id_that_is_not_one_is_refused() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;
    let mut session = side.open_stream("bench-6");

    let mut request = protocol::header(protocol::OP_REQ_IMPORT, 0).to_vec();
    request.extend_from_slice(b"../1-1");
    request.resize(protocol::HEADER_SIZE + protocol::BUS_ID_SIZE, 0);
    session.write_all(&request).await.unwrap();

    assert_eq!(
        protocol::read_header(&mut session).await.unwrap().status,
        protocol::STATUS_ERROR
    );
    side.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lock_for_a_bus_id_is_gone_once_the_export_has_begun() {
    let side = ServerSide::start(FakeKernel::new(), true, true).await;
    let mut session = side.open_stream("bench-lock");

    import(&mut session, "1-1.4").await;

    assert_eq!(side.plane.bus_locks(), 0, "the map does not keep the lock");
    drop(session);
    side.stop().await;
}

//! The upstream host against a stand-in USB/IP server (usbipd-win's part): cached lists, one
//! deadline, capped input, a relayed export, and a client attaching through it end to end.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use koppel_usb::stream::memory;
use koppel_usb::{
    Clock, CloseReason, DeviceStream, ExportStatus, UsbDevice, UsbHost, UsbInterface,
    UsbIpUpstreamHost, protocol,
};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_millis(300);

type Serve = Arc<dyn Fn(TcpStream) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Plays usbipd-win: each connection is handed to `serve`.
struct FakeUsbIpServer {
    port: u16,
    stop: CancellationToken,
}

impl FakeUsbIpServer {
    async fn start<F, Fut>(serve: F) -> Self
    where
        F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let serve: Serve = Arc::new(move |tcp| Box::pin(serve(tcp)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    _ = stopping.cancelled() => return,
                };
                let Ok((tcp, _)) = accepted else { return };
                let serve = serve.clone();
                let stopping = stopping.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = serve(tcp) => {}
                        _ = stopping.cancelled() => {}
                    }
                });
            }
        });
        Self { port, stop }
    }
}

impl Drop for FakeUsbIpServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// A clock the test moves by hand. Timers still run on the real clock.
struct ManualClock(Mutex<OffsetDateTime>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(OffsetDateTime::UNIX_EPOCH)))
    }

    fn advance(&self, by: Duration) {
        *lock(&self.0) += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> OffsetDateTime {
        *lock(&self.0)
    }
}

fn device(bus_id: &str) -> UsbDevice {
    UsbDevice {
        bus_id: bus_id.into(),
        path: r"USB\VID_064F&PID_2AF9\0".into(),
        bus_num: 2,
        dev_num: 3,
        speed: koppel_usb::UsbSpeed::Full,
        vendor_id: 0x064f,
        product_id: 0x2af9,
        bcd_device: 0x0100,
        configuration_value: 1,
        num_configurations: 1,
        interfaces: vec![UsbInterface::new(0xff, 0, 0)],
        ..UsbDevice::default()
    }
}

async fn answer_list(mut tcp: TcpStream, devices: Vec<UsbDevice>) {
    if protocol::read_header(&mut tcp).await.is_ok() {
        let _ = tcp.write_all(&protocol::dev_list_reply(&devices)).await;
    }
}

/// Answers a device list with `device`, and an import of it with `reply` and then an echo.
async fn list_or_import(
    mut tcp: TcpStream,
    device: UsbDevice,
    grant: bool,
    imports: Arc<AtomicUsize>,
    hang: bool,
) {
    let Ok(request) = protocol::read_header(&mut tcp).await else {
        return;
    };
    if request.code == protocol::OP_REQ_DEVLIST {
        let _ = tcp
            .write_all(&protocol::dev_list_reply(std::slice::from_ref(&device)))
            .await;
        return;
    }
    imports.fetch_add(1, Ordering::SeqCst);
    if protocol::read_bus_id(&mut tcp).await.is_err() {
        return;
    }
    let reply = protocol::import_reply(grant.then_some(&device));
    if tcp.write_all(&reply).await.is_err() || !grant {
        return;
    }
    if hang {
        std::future::pending::<()>().await;
    }
    let mut buffer = [0u8; 64];
    while let Ok(read) = tcp.read(&mut buffer).await {
        if read == 0 || tcp.write_all(&buffer[..read]).await.is_err() {
            return;
        }
    }
}

#[tokio::test]
async fn lists_what_the_server_exports_and_leaves_hubs_out() {
    let server = FakeUsbIpServer::start(|tcp| {
        answer_list(
            tcp,
            vec![
                device("2-3"),
                UsbDevice {
                    class: 0x09,
                    ..device("2-4")
                },
            ],
        )
    })
    .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);

    let devices = host.list_devices().await.unwrap();

    assert_eq!(
        devices
            .iter()
            .map(|d| d.bus_id.as_str())
            .collect::<Vec<_>>(),
        vec!["2-3"]
    );
    assert_eq!(host.unavailable(), None);
    assert!(!host.can_share());
    assert!(!host.can_unshare());
}

#[tokio::test]
async fn a_server_that_never_answers_ends_the_refresh_at_its_deadline() {
    let server = FakeUsbIpServer::start(|tcp| async move {
        // Holds the connection open and says nothing.
        let _tcp = tcp;
        std::future::pending::<()>().await
    })
    .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);

    let started = Instant::now();
    let devices = host.list_devices().await.unwrap();

    assert!(devices.is_empty());
    assert!(
        host.unavailable()
            .unwrap()
            .contains("did not answer in time")
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn getters_return_the_cached_list_while_a_refresh_hangs() {
    let answers = Arc::new(AtomicUsize::new(0));
    let counted = answers.clone();
    let server = FakeUsbIpServer::start(move |tcp| {
        let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
        async move {
            if first {
                answer_list(tcp, vec![device("2-3")]).await;
            } else {
                let _tcp = tcp;
                std::future::pending::<()>().await;
            }
        }
    })
    .await;
    let clock = ManualClock::new();
    let host = UsbIpUpstreamHost::with_deadline(
        "127.0.0.1",
        server.port,
        clock.clone(),
        Duration::from_secs(30),
    );
    assert_eq!(host.list_devices().await.unwrap().len(), 1);

    // The list is stale now, so the next call starts a refresh that never finishes. Nobody waits for it.
    clock.advance(Duration::from_secs(60));
    let started = Instant::now();
    let mut calls = Vec::new();
    for _ in 0..20 {
        calls.push(host.list_devices().await.unwrap());
    }

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(calls.iter().all(|list| list.len() == 1));
    assert_eq!(host.unavailable(), None);
}

#[tokio::test]
async fn a_list_with_more_devices_than_the_cap_is_refused() {
    let server = FakeUsbIpServer::start(|mut tcp| async move {
        if protocol::read_header(&mut tcp).await.is_err() {
            return;
        }
        let mut message = protocol::header(protocol::OP_REP_DEVLIST, protocol::STATUS_OK).to_vec();
        message.extend_from_slice(&1_000_000u32.to_be_bytes());
        let _ = tcp.write_all(&message).await;
        std::future::pending::<()>().await;
    })
    .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);

    let devices = host.list_devices().await.unwrap();

    assert!(devices.is_empty());
    assert!(host.unavailable().unwrap().contains("limit"));
}

#[tokio::test]
async fn bus_ids_that_are_not_bus_ids_are_dropped() {
    let server =
        FakeUsbIpServer::start(|tcp| answer_list(tcp, vec![device("2-3"), device("../../etc")]))
            .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);

    let listed: Vec<String> = host
        .list_devices()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.bus_id)
        .collect();

    assert_eq!(listed, vec!["2-3"]);
}

#[tokio::test]
async fn export_relays_the_network_stream_straight_to_the_server_and_reports_it_in_use() {
    let imports = Arc::new(AtomicUsize::new(0));
    let counted = imports.clone();
    let server = FakeUsbIpServer::start(move |tcp| {
        list_or_import(tcp, device("2-3"), true, counted.clone(), false)
    })
    .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);
    host.list_devices().await.unwrap();
    let (mut client, mut network) = memory::pair();

    let export = host.export(&device("2-3")).await.unwrap();
    let relaying = tokio::spawn(async move {
        export.relay(&mut network, CancellationToken::new()).await;
        Box::new(network).close(CloseReason::Done).await;
    });
    client.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    client.read_exact(&mut echoed).await.unwrap();

    assert_eq!(&echoed, b"ping");
    assert_eq!(host.status("2-3").await.unwrap(), ExportStatus::Used);
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), relaying)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(host.status("2-3").await.unwrap(), ExportStatus::Available);
}

#[tokio::test]
async fn a_second_export_of_a_device_in_use_is_refused_without_connecting_to_the_server() {
    let imports = Arc::new(AtomicUsize::new(0));
    let counted = imports.clone();
    let server = FakeUsbIpServer::start(move |tcp| {
        list_or_import(tcp, device("2-3"), true, counted.clone(), true)
    })
    .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);
    host.list_devices().await.unwrap();
    let first = host.export(&device("2-3")).await.unwrap();

    let second = host.export(&device("2-3")).await;

    assert!(
        second
            .err()
            .unwrap()
            .message()
            .contains("already being used")
    );
    assert_eq!(
        imports.load(Ordering::SeqCst),
        1,
        "the claim comes before the connection, so the second import never reached the server"
    );
    drop(first);
    assert_eq!(host.status("2-3").await.unwrap(), ExportStatus::Available);
}

#[tokio::test]
async fn an_export_the_server_refuses_gives_the_claim_back() {
    let imports = Arc::new(AtomicUsize::new(0));
    let counted = imports.clone();
    let server = FakeUsbIpServer::start(move |tcp| {
        list_or_import(tcp, device("2-3"), false, counted.clone(), false)
    })
    .await;
    let host =
        UsbIpUpstreamHost::with_deadline("127.0.0.1", server.port, ManualClock::new(), DEADLINE);
    host.list_devices().await.unwrap();

    let export = host.export(&device("2-3")).await;

    assert!(export.err().unwrap().message().contains("would not export"));
    assert_eq!(host.status("2-3").await.unwrap(), ExportStatus::Available);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_attaches_a_device_the_usbip_server_exports_and_traffic_reaches_it() {
    let imports = Arc::new(AtomicUsize::new(0));
    let counted = imports.clone();
    let exported = UsbDevice {
        bus_id: "2-3".into(),
        ..dongle()
    };
    let shared = exported.clone();
    let usbipd = FakeUsbIpServer::start(move |tcp| {
        list_or_import(tcp, shared.clone(), true, counted.clone(), false)
    })
    .await;
    let host = Arc::new(UsbIpUpstreamHost::new(
        "127.0.0.1",
        usbipd.port,
        Arc::new(koppel_usb::SystemClock),
    ));
    let server = ServerSide::start(host, false, true).await;
    let client_kernel = FakeKernel::new();
    let client = ClientSide::start(
        MemoryLinks::new(&server),
        ClientSetup::kernel(client_kernel.clone()),
    )
    .await;

    client.link().await;
    assert_eq!(client.client.current().devices[0].device.bus_id, "2-3");
    assert_eq!(client.client.current().devices[0].device.state, "shared");
    client.client.begin_attach("2-3").await.unwrap();
    eventually("the attach", || client.attached_here()).await;

    let mut driver = client_kernel.driver_stream(1).await;
    echoes(&mut driver).await;
    assert_eq!(imports.load(Ordering::SeqCst), 1);
    client.stop().await;
    server.stop().await;
}

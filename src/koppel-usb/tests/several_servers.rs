//! One client with several servers: each server's devices attach and detach on their own, removing a
//! server releases only its devices, and two servers attaching at once do not meet at one port.

mod common;

use std::fs;
use std::sync::Arc;

use common::*;
use koppel_config::files::{AttachedDevices, ClientConnection};
use koppel_config::{ConfigFile, KoppelPaths, Options, open};
use koppel_usb::{
    ClientOptions, ClientServerDeps, ClientServersView, Common, LinkState, Platform, SystemClock,
    UsbClients,
};

struct Two {
    _dir: TempDir,
    clients: UsbClients,
    kernel: Arc<FakeKernel>,
    servers: Vec<ServerSide>,
    files: Vec<(ConfigFile<ClientConnection>, ConfigFile<AttachedDevices>)>,
}

/// Two servers, each sharing a dongle with the same bus ID, and a client with both, over one kernel.
async fn two() -> Two {
    let dir = TempDir::new("several");
    let paths = KoppelPaths::new(dir.path().join("config"), dir.path().join("keys"));
    fs::create_dir_all(dir.path().join("config")).unwrap();
    fs::write(paths.settings_file(), r#"{ "role": "client" }"#).unwrap();
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    let kernel = FakeKernel::in_memory();
    let mut platform = Platform::unsupported("No driver or vhci-hcd here.");
    platform.attacher = Some(kernel.clone());
    let clients = UsbClients::new(Common {
        settings: files::settings(&settings),
        platform,
        activity: Arc::new(Log::default()),
        clock: Arc::new(SystemClock),
        options: ClientOptions::default(),
    });
    let mut servers = Vec::new();
    let mut kept = Vec::new();
    for n in 1..=2 {
        let server = ServerSide::start(FakeKernel::new(), true, true).await;
        let connection = ConfigFile::new(
            dir.path().join(format!("connection-{n}.json")),
            ClientConnection::default(),
            Options::default(),
        );
        connection.start().await.unwrap();
        let attached = ConfigFile::new(
            dir.path().join(format!("attached-{n}.json")),
            AttachedDevices::default(),
            Options::default(),
        );
        attached.start().await.unwrap();
        connection
            .update(|_| ClientConnection {
                server_url: Some(format!("https://server-{n}.test:5240")),
                server_fingerprint: Some("AB:CD".into()),
                client_id: Some(format!("client-{n}")),
                client_secret: Some(format!("secret-{n}").into()),
                next_server_fingerprint: None,
            })
            .await
            .unwrap();
        clients
            .add(
                &format!("server-{n}"),
                ClientServerDeps {
                    name: format!("Server {n}"),
                    connection: files::connection(&connection),
                    remembered: files::attached(&attached),
                    links: MemoryLinks::new(&server),
                },
            )
            .await
            .unwrap();
        servers.push(server);
        kept.push((connection, attached));
    }
    let two = Two {
        _dir: dir,
        clients,
        kernel,
        servers,
        files: kept,
    };
    let clients = two.clients.clone();
    eventually("both links to list their devices", || {
        let view = clients.current();
        view.servers.len() == 2
            && view
                .servers
                .iter()
                .all(|s| s.link == LinkState::Connected && !s.devices.is_empty())
    })
    .await;
    two
}

fn attached_on(view: &ClientServersView, id: &str) -> bool {
    view.servers
        .iter()
        .find(|s| s.id == id)
        .is_some_and(|s| s.devices.iter().any(|d| d.attached_here))
}

#[tokio::test(start_paused = true)]
async fn two_servers_with_the_same_bus_id_attach_and_detach_on_their_own() {
    let two = two().await;
    two.clients.begin_attach("server-1", "1-1.4").await.unwrap();
    let clients = two.clients.clone();
    eventually("server 1's device", || {
        attached_on(&clients.current(), "server-1")
    })
    .await;
    assert!(!attached_on(&two.clients.current(), "server-2"));

    two.clients.begin_attach("server-2", "1-1.4").await.unwrap();
    eventually("server 2's device", || {
        attached_on(&clients.current(), "server-2")
    })
    .await;
    assert!(attached_on(&two.clients.current(), "server-1"));

    two.clients.detach("server-1", "1-1.4").await.unwrap();
    eventually("server 1's device to be released", || {
        !attached_on(&clients.current(), "server-1")
    })
    .await;
    assert!(attached_on(&two.clients.current(), "server-2"));
    two.clients.stop().await;
}

#[tokio::test(start_paused = true)]
async fn removing_a_server_releases_only_its_devices() {
    let two = two().await;
    two.clients.begin_attach("server-1", "1-1.4").await.unwrap();
    two.clients.begin_attach("server-2", "1-1.4").await.unwrap();
    let clients = two.clients.clone();
    eventually("both attached", || {
        let view = clients.current();
        attached_on(&view, "server-1") && attached_on(&view, "server-2")
    })
    .await;

    two.clients.remove("server-1").await;

    let view = two.clients.current();
    assert_eq!(view.servers.len(), 1);
    assert!(attached_on(&view, "server-2"));
    assert_eq!(two.kernel.detached_ports().len(), 1);
    two.clients.stop().await;
}

#[tokio::test(start_paused = true)]
async fn two_attaches_at_once_use_two_ports() {
    let two = two().await;
    let (a, b) = tokio::join!(
        two.clients.begin_attach("server-1", "1-1.4"),
        two.clients.begin_attach("server-2", "1-1.4"),
    );
    a.unwrap();
    b.unwrap();
    let clients = two.clients.clone();
    eventually("both attached", || {
        let view = clients.current();
        attached_on(&view, "server-1") && attached_on(&view, "server-2")
    })
    .await;
    assert_eq!(two.kernel.attaches(), 2);
    two.clients.stop().await;
}

#[tokio::test(start_paused = true)]
async fn an_unknown_server_is_refused() {
    let two = two().await;
    assert!(two.clients.begin_attach("server-9", "1-1.4").await.is_err());
    assert_eq!(two.servers.len(), 2);
    assert_eq!(two.files.len(), 2);
    two.clients.stop().await;
}

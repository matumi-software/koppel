//! A Rust server and a Rust client, each a whole Koppel in this process with a fake kernel, paired
//! through the code and linked over Skakel: a shared device attaches end to end and its USB traffic
//! crosses, the link and the attached device survive a certificate handover and a server restart,
//! and revoking or rotating the client ends its link with word to pair again.

mod common;

use common::{FakeKernel, Node, TempDir, down, echoes, eventually, linked, pair};
#[cfg(not(windows))]
use common::{free_udp_port, loopback};
use reqwest::Method;
use serde_json::json;

/// The server shares the dongle; the client attaches it and the echo comes back through both
/// kernels, both relays and Skakel's stream.
async fn attach_the_dongle(server: &Node, client: &Node) {
    let status = server
        .call(
            Method::PUT,
            "/api/server/devices/1-1.4/shared",
            Some(json!({ "shared": true })),
        )
        .await
        .0;
    assert_eq!(status, 202);
    eventually("the client to see the shared dongle", || async {
        let devices = client.get("/api/client/devices").await;
        devices["devices"]
            .as_array()
            .and_then(|list| list.iter().find(|d| d["device"]["busId"] == json!("1-1.4")))
            .map(|_| ())
    })
    .await;
    assert_eq!(
        client
            .post("/api/client/devices/1-1.4/attach", json!({}))
            .await,
        202
    );
    attached(client).await;
}

/// Waits until the client shows the dongle attached here.
async fn attached(client: &Node) {
    eventually("the dongle to be attached here", || async {
        let devices = client.get("/api/client/devices").await;
        devices["devices"]
            .as_array()
            .and_then(|list| {
                list.iter().find(|d| {
                    d["device"]["busId"] == json!("1-1.4") && d["attachedHere"] == json!(true)
                })
            })
            .map(|_| ())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_client_attaches_and_survives_a_handover_and_a_server_restart() {
    let (server_dir, client_dir) = (TempDir::new("server"), TempDir::new("client"));
    let server_kernel = FakeKernel::new();
    let client_kernel = FakeKernel::new();
    let server = Node::start(server_dir.path(), "server", None, server_kernel.clone()).await;
    let client = Node::start(client_dir.path(), "client", None, client_kernel.clone()).await;

    let client_id = pair(&server, &client).await;
    let clients = server.get("/api/server/clients").await;
    assert!(
        clients
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == json!(client_id) && c["name"] == json!("bench")),
        "{clients}"
    );
    let connection = client.get("/api/client/connection").await;
    assert_eq!(connection["serverUrl"], json!(server.url()));
    assert_eq!(connection["hasSecret"], json!(true));
    let test = client
        .call(Method::POST, "/api/client/connection/test", None)
        .await
        .1;
    assert_eq!(test["outcome"], json!("ok"), "{test}");

    attach_the_dongle(&server, &client).await;
    let mut driver = client_kernel.driver_stream(1).await;
    echoes(&mut driver, b"before the handover").await;
    let devices = server.get("/api/server/devices").await;
    assert_eq!(
        devices["devices"][0]["state"],
        json!("attached"),
        "{devices}"
    );

    // The handover: the client stores the next certificate before the switch, then pins it.
    let first = server.get("/api/server/identity").await["fingerprint"].clone();
    assert_eq!(
        server
            .post("/api/server/keys/next-certificate", json!({}))
            .await,
        202
    );
    let next = server.get("/api/server/keys").await["nextFingerprint"].clone();
    assert!(next.is_string());
    eventually("the client to store the next certificate", || async {
        (client.get("/api/client/connection").await["nextServerFingerprint"] == next).then_some(())
    })
    .await;
    eventually("the server to see the client has it", || async {
        let clients = server.get("/api/server/clients").await;
        clients
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == json!(client_id) && c["hasNextCertificate"] == json!(true))
            .then_some(())
    })
    .await;
    assert_eq!(
        server
            .post("/api/server/keys/next-certificate/switch", json!({}))
            .await,
        202
    );
    assert_eq!(
        server.get("/api/server/identity").await["fingerprint"],
        next
    );
    eventually("the client to pin the new certificate", || async {
        let connection = client.get("/api/client/connection").await;
        (connection["serverFingerprint"] == next && connection["nextServerFingerprint"].is_null())
            .then_some(())
    })
    .await;
    assert_ne!(first, next);
    linked(&client).await;
    attached(&client).await;
    echoes(&mut driver, b"after the handover").await;
    let test = client
        .call(Method::POST, "/api/client/connection/test", None)
        .await
        .1;
    assert_eq!(test["outcome"], json!("ok"), "{test}");

    // A restart: the client links again on its own and attaches the remembered dongle again.
    let port = server.port;
    server.stop().await;
    down(&client).await;
    let server = Node::start(
        server_dir.path(),
        "server",
        Some(port),
        server_kernel.clone(),
    )
    .await;
    assert_eq!(
        server.get("/api/server/identity").await["fingerprint"],
        next
    );
    linked(&client).await;
    let mut driver = client_kernel.driver_stream(2).await;
    echoes(&mut driver, b"after the restart").await;
    attached(&client).await;

    // Revoking ends the link for good, with word to pair again.
    let status = server
        .call(
            Method::DELETE,
            &format!("/api/server/clients/{client_id}"),
            None,
        )
        .await
        .0;
    assert_eq!(status, 202);
    let reason = down(&client).await;
    assert!(reason.contains("Pair with the server again"), "{reason}");

    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rotating_the_secret_ends_the_link_and_says_to_pair_again() {
    let (server_dir, client_dir) = (TempDir::new("server"), TempDir::new("client"));
    let server = Node::start(server_dir.path(), "server", None, FakeKernel::new()).await;
    let client = Node::start(client_dir.path(), "client", None, FakeKernel::new()).await;
    let client_id = pair(&server, &client).await;

    let (status, body) = server
        .call(
            Method::POST,
            &format!("/api/server/clients/{client_id}/rotate"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let reason = down(&client).await;
    assert!(reason.contains("Pair with the server again"), "{reason}");
    let test = client
        .call(Method::POST, "/api/client/connection/test", None)
        .await
        .1;
    assert_eq!(test["outcome"], json!("rejectedCredentials"), "{test}");

    // Pairing again links again.
    pair(&server, &client).await;

    client.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_the_server_role_ends_the_link_and_coming_back_takes_it_again() {
    let (server_dir, client_dir) = (TempDir::new("server"), TempDir::new("client"));
    let server = Node::start(server_dir.path(), "server", None, FakeKernel::new()).await;
    let client = Node::start(client_dir.path(), "client", None, FakeKernel::new()).await;
    pair(&server, &client).await;

    let status = server
        .call(
            Method::PATCH,
            "/api/settings",
            Some(json!({ "role": "client" })),
        )
        .await
        .0;
    assert_eq!(status, 202);
    down(&client).await;
    let status = server
        .call(
            Method::PATCH,
            "/api/settings",
            Some(json!({ "role": "server" })),
        )
        .await
        .0;
    assert_eq!(status, 202);
    linked(&client).await;

    client.stop().await;
    server.stop().await;
}

// Not on Windows: the client's discovery socket listens on every address, which can bring up a
// firewall prompt for each new test build.
#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn the_link_follows_a_server_that_moved_and_saves_where_it_went() {
    let (server_dir, client_dir) = (TempDir::new("server"), TempDir::new("client"));
    // The client hears the server's discovery answers on loopback, as it would by broadcast.
    let udp = free_udp_port();
    let discovery = Some((udp, loopback(udp)));
    let server = Node::start_with(
        server_dir.path(),
        "server",
        None,
        FakeKernel::new(),
        discovery,
    )
    .await;
    let client = Node::start_with(
        client_dir.path(),
        "client",
        None,
        FakeKernel::new(),
        discovery,
    )
    .await;
    let found = eventually("the client to find the server", || async {
        let servers = client.get("/api/client/discovery").await;
        servers.as_array().and_then(|list| list.first().cloned())
    })
    .await;
    assert_eq!(found["httpsPort"], json!(server.port));
    pair(&server, &client).await;

    // The server comes back on another port with the same certificate.
    server.stop().await;
    let server = Node::start_with(
        server_dir.path(),
        "server",
        None,
        FakeKernel::new(),
        discovery,
    )
    .await;
    eventually("the saved connection to follow the server", || async {
        (client.get("/api/client/connection").await["serverUrl"] == json!(server.url()))
            .then_some(())
    })
    .await;
    linked(&client).await;
    let moved = client.get("/api/activity").await;
    assert!(
        moved
            .as_array()
            .is_some_and(|list| list.iter().any(|e| e["reason"] == json!("serverMoved"))),
        "{moved}"
    );

    client.stop().await;
    server.stop().await;
}

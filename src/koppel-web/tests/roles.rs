//! A Rust server and a Rust client, each a whole Koppel in this process with a fake kernel, paired
//! through the code and linked over Skakel: a shared device attaches end to end and its USB traffic
//! crosses, the link and the attached device survive a certificate handover and a server restart,
//! and revoking or rotating the client ends its link with word to pair again. Skakel's server sees
//! each caller's own address through Koppel's port.

mod common;

use std::net::IpAddr;

use common::{
    FakeKernel, Node, TempDir, down, echoes, eventually, http_client_from, linked, open_pairing,
    pair,
};
#[cfg(not(windows))]
use common::{free_udp_port, loopback};
use reqwest::Method;
use serde_json::{Value, json};

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

/// A nonce of the length Skakel asks for: a revealed one shorter than 43 characters is refused.
fn nonce_of(label: &str) -> String {
    format!("{label:x<43}")
}

/// Opens a pairing request on `server`'s port from the loopback address `from`, with the commitment
/// to the nonce of `label`, and gives the status and the answer.
async fn open_pairing_from(server: &Node, from: &str, label: &str) -> (u16, Value) {
    let from: IpAddr = from.parse().unwrap();
    let commitment = skakel_wire::pairing::commit(&nonce_of(label));
    let answer = http_client_from(Some(from))
        .post(format!("{}/skakel/v3/pair", server.url()))
        .json(&json!({ "clientName": format!("pc at {from}"), "commitment": commitment }))
        .send()
        .await
        .unwrap();
    let status = answer.status().as_u16();
    (status, answer.json().await.unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn callers_at_two_addresses_do_not_share_a_pairing_limit() {
    let dir = TempDir::new("server");
    let server = Node::start(dir.path(), "server", None, FakeKernel::new()).await;
    open_pairing(&server).await;

    // Skakel takes three open pairing requests from one address at a time.
    for n in 0..3 {
        let (status, body) = open_pairing_from(&server, "127.0.0.1", &format!("one-{n}")).await;
        assert_eq!(status, 200, "{body}");
    }
    let (fourth, _) = open_pairing_from(&server, "127.0.0.1", "one-3").await;
    assert_eq!(fourth, 429, "a fourth from 127.0.0.1 is one too many");

    // Another address has a limit of its own, and the server's list shows where it came from.
    let (status, opened) = open_pairing_from(&server, "127.0.0.2", "two").await;
    assert_eq!(
        status, 200,
        "127.0.0.2 does not share 127.0.0.1's limit: {opened}"
    );
    let id = opened["requestId"].as_str().unwrap().to_owned();
    let revealed = http_client_from(Some("127.0.0.2".parse().unwrap()))
        .post(format!("{}/skakel/v3/pair/{id}/reveal", server.url()))
        .json(&json!({ "clientNonce": nonce_of("two") }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(revealed, 204);
    let request = eventually("the server to list the request", || async {
        let pairing = server.get("/api/server/pairing").await;
        pairing["requests"]
            .as_array()
            .and_then(|list| list.iter().find(|r| r["id"] == json!(id)).cloned())
    })
    .await;
    assert_eq!(request["from"], json!("127.0.0.2"), "{request}");

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

#[tokio::test(flavor = "multi_thread")]
async fn a_server_takes_no_pairing_request_until_its_window_is_opened() {
    let dir = TempDir::new("server");
    let server = Node::start(dir.path(), "server", None, FakeKernel::new()).await;

    let overview = server.get("/api/server/pairing").await;
    assert_eq!(overview["window"]["open"], json!(false), "{overview}");
    let (status, body) = open_pairing_from(&server, "127.0.0.1", "closed").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["reason"], json!("pairing-closed"), "{body}");

    open_pairing(&server).await;
    let opened = server.get("/api/server/pairing").await;
    assert!(opened["window"]["until"].is_string(), "{opened}");
    let (status, body) = open_pairing_from(&server, "127.0.0.1", "open").await;
    assert_eq!(status, 200, "{body}");

    let status = server
        .call(Method::DELETE, "/api/server/pairing/window", None)
        .await
        .0;
    assert_eq!(status, 202);
    eventually("the window to close", || async {
        (server.get("/api/server/pairing").await["window"]["open"] == json!(false)).then_some(())
    })
    .await;
    let (status, _) = open_pairing_from(&server, "127.0.0.2", "closed-again").await;
    assert_eq!(status, 403);

    // A window outside what Skakel takes is refused before it reaches Skakel.
    for minutes in [0, 61] {
        let (status, body) = server
            .call(
                Method::POST,
                "/api/server/pairing/window",
                Some(json!({ "minutes": minutes })),
            )
            .await;
        assert_eq!(status, 400, "{body}");
    }

    server.stop().await;
}

/// A server that speaks Skakel 2 only, as Koppel 0.2.0 does: it answers the info endpoint and
/// nothing else, over TLS with a certificate of its own.
async fn skakel_2_server() -> (u16, String, tokio::sync::oneshot::Sender<()>) {
    use axum::Router;
    use axum::routing::get;
    use koppel_web::CertificateSlot;
    use koppel_web::tls;

    let certificate = tls::self_signed(&["127.0.0.1", "localhost"]).unwrap();
    let fingerprint = ring::digest::digest(&ring::digest::SHA256, certificate.der.as_ref())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    let acceptor = tls::acceptor(CertificateSlot::new(certificate.certified)).unwrap();
    let listener =
        koppel_web::bind(Some("127.0.0.1".parse().unwrap()), common::free_port()).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let router = Router::new().route(
        "/skakel/info",
        get(|| async {
            axum::Json(json!({ "name": "old", "app": { "id": "koppel", "version": "0.2.0" }, "versions": [2] }))
        }),
    );
    tokio::spawn(koppel_web::serve(listener, acceptor, router, async {
        let _ = stopped.await;
    }));
    (port, fingerprint, stop)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_on_skakel_2_is_named_when_pairing_and_when_testing_the_connection() {
    let dir = TempDir::new("client");
    let client = Node::start(dir.path(), "client", None, FakeKernel::new()).await;
    let (port, fingerprint, stop) = skakel_2_server().await;
    let url = format!("https://127.0.0.1:{port}");

    let status = client
        .post(
            "/api/client/pairing",
            json!({ "serverUrl": url, "serverFingerprint": fingerprint, "serverName": "old" }),
        )
        .await;
    assert_eq!(status, 202);
    let failed = eventually("the pairing to fail", || async {
        let pairing = client.get("/api/client/pairing").await;
        (pairing["state"] == json!("failed")).then_some(pairing)
    })
    .await;
    let message = failed["message"].as_str().unwrap();
    assert!(
        message.contains("speaks Skakel 2") && message.contains("Update the server"),
        "{message}"
    );

    // The connection a 0.2.0 client saved stays as it is, and its test says why it cannot link.
    let status = client
        .call(
            Method::PUT,
            "/api/client/connection",
            Some(json!({
                "serverUrl": url,
                "serverFingerprint": fingerprint,
                "clientId": "client-0123456789",
                "clientSecret": "a-secret-from-before-the-upgrade",
            })),
        )
        .await
        .0;
    assert_eq!(status, 202);
    let test = eventually("the connection test to answer", || async {
        let test = client
            .call(Method::POST, "/api/client/connection/test", None)
            .await
            .1;
        (test["outcome"] != json!("incomplete")).then_some(test)
    })
    .await;
    assert_eq!(test["outcome"], json!("versionMismatch"), "{test}");
    assert!(
        test["message"]
            .as_str()
            .unwrap()
            .contains("Update the server"),
        "{test}"
    );
    let saved = client.get("/api/client/connection").await;
    assert_eq!(saved["clientId"], json!("client-0123456789"), "{saved}");
    assert_eq!(saved["hasSecret"], json!(true), "{saved}");

    let _ = stop.send(());
    client.stop().await;
}

/// Koppel's `secrets.json`, opened as the program opens it, for a test to read and change while no
/// Koppel runs on that folder.
async fn secrets_in(
    dir: &std::path::Path,
) -> koppel_config::ConfigFile<koppel_config::files::ServerSecrets> {
    let (config, keys) = (dir.join("config"), dir.join("keys"));
    let paths = koppel_config::KoppelPaths::new(&config, &keys);
    let secrets = std::sync::Arc::new(koppel_config::SecretBox::open(&keys).unwrap());
    let file = koppel_config::open::server_secrets(&paths, &secrets);
    file.start().await.unwrap();
    file
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_with_a_token_signing_key_in_its_state_keeps_its_clients_and_drops_the_key() {
    let (server_dir, client_dir) = (TempDir::new("server"), TempDir::new("client"));
    let server = Node::start(server_dir.path(), "server", None, FakeKernel::new()).await;
    let client = Node::start(client_dir.path(), "client", None, FakeKernel::new()).await;
    let client_id = pair(&server, &client).await;
    let port = server.port;
    server.stop().await;

    // The file as an earlier Skakel left it: the same state, and the signing keys it used to keep.
    let (config, keys) = (
        server_dir.path().join("config"),
        server_dir.path().join("keys"),
    );
    let paths = koppel_config::KoppelPaths::new(&config, &keys);
    let secret_box = koppel_config::SecretBox::open(&keys).unwrap();
    let file = paths.server_secrets_file();
    let mut json: Value = serde_json::from_slice(
        &secret_box
            .unprotect(&std::fs::read(&file).unwrap())
            .unwrap(),
    )
    .unwrap();
    let key = json!({
        "keyId": "skakel-0123456789",
        "privateKey": "bm90IGEgcmVhbCBrZXk=",
        "createdAt": "2026-11-09T08:30:15Z",
        "acceptUntil": null,
    });
    json["skakel"]["signingKey"] = key.clone();
    json["skakel"]["retiredSigningKeys"] = json!([key]);
    std::fs::write(
        &file,
        secret_box
            .protect(&serde_json::to_vec(&json).unwrap())
            .unwrap(),
    )
    .unwrap();
    let before = secrets_in(server_dir.path()).await;
    let before_state = before.current().skakel.clone().unwrap();
    before.stop();
    assert_eq!(before_state.clients.len(), 1);

    // The Koppel that starts on it keeps the client, which links again with the credentials it
    // already holds, and the next save leaves the keys out.
    let server = Node::start(server_dir.path(), "server", Some(port), FakeKernel::new()).await;
    linked(&client).await;
    let listed = server.get("/api/server/clients").await;
    assert_eq!(listed[0]["id"], json!(client_id), "{listed}");
    server.stop().await;

    let after = secrets_in(server_dir.path()).await;
    let after_state = after.current().skakel.clone().unwrap();
    after.stop();
    assert_eq!(after_state.clients, before_state.clients);
    assert_eq!(after_state.certificate, before_state.certificate);

    client.stop().await;
}

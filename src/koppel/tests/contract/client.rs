//! The client role, against a real server: adding a server by its pairing code or its server key (a
//! wrong code, one already added, what is checked first), the list of servers and its changes, the
//! connection test, servers found on the network, and the devices of each server. The client target
//! reaches the main one at its own address.

use std::time::Duration;

use crate::support::shape::{self, Kind};
use crate::support::{Pair, Session, eventually, json, tls};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(60);

async fn servers(client: &Session) -> Value {
    let value = client
        .get("/api/v1/client/servers")
        .await
        .expect(200)
        .json();
    shape::fields(&value, shape::CLIENT_SERVERS, "GET /api/v1/client/servers");
    shape::each(&value["servers"], "GET /api/v1/client/servers", |server| {
        shape::fields(server, shape::CLIENT_SERVER, "a server of the client");
    });
    value
}

async fn connection_test(client: &Session, id: &str) -> Value {
    let path = format!("/api/v1/client/servers/{id}/test");
    let answer = client.post_empty(&path).await.expect(200).json();
    shape::fields(
        &answer,
        shape::CONNECTION_TEST,
        "POST /api/v1/client/servers/{id}/test",
    );
    answer
}

async fn client_devices(client: &Session) -> Value {
    let devices = client
        .get("/api/v1/client/devices")
        .await
        .expect(200)
        .json();
    shape::fields(
        &devices,
        shape::CLIENT_DEVICES,
        "GET /api/v1/client/devices",
    );
    shape::each(&devices["servers"], "client servers", |server| {
        shape::fields(
            server,
            shape::CLIENT_SERVER_DEVICES,
            "a server of GET /api/v1/client/devices",
        );
    });
    devices
}

async fn fingerprint(admin: &Session) -> String {
    admin
        .get("/api/v1/server/identity")
        .await
        .expect(200)
        .json()["fingerprint"]
        .as_str()
        .expect("a fingerprint")
        .to_owned()
}

async fn machine_name(admin: &Session) -> String {
    admin.get("/api/v1/system/info").await.expect(200).json()["name"]
        .as_str()
        .expect("a name")
        .to_owned()
}

/// Waits until the client's link to the server `id` is up, and gives the client's device list.
async fn linked(client: &Session, id: &str) -> Value {
    eventually("the client to link", WAIT, || async {
        let devices = client_devices(client).await;
        devices["servers"]
            .as_array()
            .and_then(|list| list.iter().find(|s| s["id"] == json!(id)))
            .is_some_and(|s| s["link"] == json!("connected"))
            .then_some(devices)
    })
    .await
}

#[tokio::test]
async fn a_client_that_was_never_set_up_has_no_servers() {
    let pair = Pair::get().await;
    let client = &pair.client_admin;
    assert_eq!(servers(client).await["servers"], json!([]));
    let devices = client_devices(client).await;
    assert_eq!(devices["servers"], json!([]), "{devices}");
    client
        .post_empty("/api/v1/client/servers/server-none/test")
        .await
        .expect(404);
}

#[tokio::test]
async fn adding_a_server_with_the_code_registers_the_client_and_links_it() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    let client_name = machine_name(client).await;
    let server_name = machine_name(server).await;

    // What is wrong with the request is said before anything is sent to the server.
    let code = pair.show_code().await;
    let add = |body: Value| client.post("/api/v1/client/servers", body);
    add(json!({ "address": "http://elsewhere:5240", "code": code }))
        .await
        .problem(&["address"]);
    add(json!({ "address": "not an address", "code": code }))
        .await
        .problem(&["address"]);
    add(json!({ "address": pair.peer_url, "code": "12" }))
        .await
        .problem(&["code"]);
    add(json!({ "address": pair.peer_url }))
        .await
        .problem(&["code"]);
    add(json!({ "address": pair.peer_url, "code": code, "key": "kp1_x" }))
        .await
        .problem(&["code"]);
    add(json!({ "address": pair.peer_url, "code": code, "name": "x".repeat(81) }))
        .await
        .problem(&["name"]);
    assert_eq!(servers(client).await["servers"], json!([]));

    // A code the server is not showing is refused, and nothing is saved.
    let wrong_code = if code == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    let refused = add(json!({ "address": pair.peer_url, "code": wrong_code }))
        .await
        .expect(422)
        .json();
    assert_eq!(refused["code"], json!("enrolment_failed"), "{refused}");
    assert!(refused["detail"].is_string(), "{refused}");
    assert_eq!(servers(client).await["servers"], json!([]));

    // The right code adds it, whatever path follows the address. A wrong code may have spent the
    // one showing, so the server shows a new one.
    let code = pair.show_code().await;
    let added = add(json!({ "address": format!("{}/ignored/path", pair.peer_url), "code": code }))
        .await
        .expect(201)
        .json();
    shape::fields(&added, shape::CLIENT_SERVER, "the added server");
    assert_eq!(added["name"], json!(server_name));
    assert_eq!(added["serverUrl"], json!(pair.peer_url));
    assert_eq!(
        tls::normalize(added["serverFingerprint"].as_str().unwrap_or_default()),
        server_fingerprint
    );
    assert_eq!(added["hasSecret"], json!(true));
    assert_eq!(added["linkedWith"], json!("code"));
    assert_eq!(added["reconnect"], json!(true));
    let server_id = added["id"].as_str().expect("an id").to_owned();
    let client_id = added["clientId"].as_str().expect("a client id").to_owned();

    let listed = servers(client).await;
    assert_eq!(listed["servers"], json!([added]));
    let clients = server
        .get("/api/v1/server/clients")
        .await
        .expect(200)
        .json();
    let row = clients
        .as_array()
        .and_then(|list| list.iter().find(|c| c["id"] == json!(client_id)))
        .cloned()
        .expect("the server lists the enrolled client");
    assert_eq!(row["name"], json!(client_name));

    // The same server cannot be added twice, with a new code or not.
    let code = pair.show_code().await;
    let twice = add(json!({ "address": pair.peer_url, "code": code }))
        .await
        .expect(409)
        .json();
    assert_eq!(twice["code"], json!("already_added"), "{twice}");
    assert_eq!(
        servers(client).await["servers"].as_array().map(Vec::len),
        Some(1)
    );

    let test = connection_test(client, &server_id).await;
    assert_eq!(test["ok"], json!(true), "{test}");
    assert_eq!(test["outcome"], json!("ok"));

    let devices = linked(client, &server_id).await;
    let shared = server
        .get("/api/v1/server/devices")
        .await
        .expect(200)
        .json();
    let entry = &devices["servers"][0];
    assert_eq!(entry["name"], json!(server_name));
    assert_eq!(
        entry["devices"].as_array().map(Vec::len),
        shared["devices"].as_array().map(Vec::len),
        "the client sees what the server has"
    );
}

#[tokio::test]
async fn adding_a_server_with_its_key_links_it_and_a_wrong_key_is_refused() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let key = server
        .get("/api/v1/server/enrollment/key")
        .await
        .expect(200)
        .json()["key"]
        .as_str()
        .expect("a key")
        .to_owned();

    client
        .post(
            "/api/v1/client/servers",
            json!({ "address": pair.peer_url, "key": "not a key" }),
        )
        .await
        .problem(&["key"]);
    assert_eq!(servers(client).await["servers"], json!([]));

    let added = client
        .post(
            "/api/v1/client/servers",
            json!({ "address": pair.peer_url, "key": key, "name": "by key" }),
        )
        .await
        .expect(201)
        .json();
    assert_eq!(added["name"], json!("by key"));
    assert_eq!(added["linkedWith"], json!("key"));
    let server_id = added["id"].as_str().expect("an id").to_owned();
    linked(client, &server_id).await;
}

#[tokio::test]
async fn a_server_is_renamed_and_its_link_settings_changed_and_checked() {
    let pair = Pair::get().await;
    let client = &pair.client_admin;
    let linked_server = pair.connect().await;
    let path = format!("/api/v1/client/servers/{}", linked_server.server_id);

    for (body, field) in [
        (json!({ "name": "" }), "name"),
        (json!({ "name": "   " }), "name"),
        (json!({ "name": "x".repeat(81) }), "name"),
        (json!({ "retryEverySeconds": 0 }), "retryEverySeconds"),
        (json!({ "retryEverySeconds": 3601 }), "retryEverySeconds"),
        (json!({ "giveUpAfterSeconds": 0 }), "giveUpAfterSeconds"),
        (json!({ "giveUpAfterSeconds": 301 }), "giveUpAfterSeconds"),
    ] {
        client.patch(&path, body).await.problem(&[field]);
    }
    client
        .patch(
            "/api/v1/client/servers/server-none",
            json!({ "name": "nobody" }),
        )
        .await
        .expect(404);
    let untouched = servers(client).await;
    assert_ne!(untouched["servers"][0]["name"], json!("nobody"));

    // The limits themselves are fine, and what is left out stays.
    client
        .patch(
            &path,
            json!({ "name": "x".repeat(80), "retryEverySeconds": 3600, "giveUpAfterSeconds": 300 }),
        )
        .await
        .expect(202)
        .empty();
    client
        .patch(&path, json!({ "reconnect": false }))
        .await
        .expect(202)
        .empty();
    let saved = eventually("the change to be saved", WAIT, || async {
        let saved = servers(client).await["servers"][0].clone();
        (saved["reconnect"] == json!(false)).then_some(saved)
    })
    .await;
    assert_eq!(saved["name"], json!("x".repeat(80)));
    assert_eq!(saved["retryEverySeconds"], json!(3600));
    assert_eq!(saved["giveUpAfterSeconds"], json!(300));
    assert_eq!(saved["clientId"], json!(linked_server.client_id));
    assert_eq!(saved["hasSecret"], json!(true));
}

#[tokio::test]
async fn a_removed_server_is_gone_from_both_lists() {
    let pair = Pair::get().await;
    let client = &pair.client_admin;
    let linked_server = pair.connect().await;
    let path = format!("/api/v1/client/servers/{}", linked_server.server_id);

    client.delete(&path).await.expect(202).empty();
    assert_eq!(servers(client).await["servers"], json!([]));
    assert_eq!(client_devices(client).await["servers"], json!([]));
    client.delete(&path).await.expect(404);
    client.post_empty(&format!("{path}/test")).await.expect(404);
}

#[tokio::test]
async fn the_connection_test_names_what_is_wrong_in_fixed_words() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let linked_server = pair.connect().await;
    let id = linked_server.server_id.as_str();

    let test = connection_test(client, id).await;
    assert_eq!(test["ok"], json!(true), "{test}");
    assert_eq!(test["outcome"], json!("ok"));
    assert!(!test["message"].as_str().unwrap_or_default().is_empty());

    // The server gives the client a new secret, so the one the client holds is wrong, until the
    // server turns this client away for a while (10 in a minute).
    server
        .post_empty(&format!(
            "/api/v1/server/clients/{}/secret",
            linked_server.client_id
        ))
        .await
        .expect(200);
    let mut outcomes = Vec::new();
    for _ in 0..12 {
        let answer = connection_test(client, id).await;
        assert_eq!(answer["ok"], json!(false), "{answer}");
        assert!(!answer["message"].as_str().unwrap_or_default().is_empty());
        let outcome = answer["outcome"].as_str().unwrap_or_default().to_owned();
        let done = outcome == "tooManyRequests";
        outcomes.push(outcome);
        if done {
            break;
        }
    }
    assert_eq!(
        outcomes.first().map(String::as_str),
        Some("rejectedCredentials"),
        "{outcomes:?}"
    );
    assert_eq!(
        outcomes.last().map(String::as_str),
        Some("tooManyRequests"),
        "{outcomes:?}"
    );
    assert!(
        outcomes[..outcomes.len() - 1]
            .iter()
            .all(|o| o == "rejectedCredentials"),
        "{outcomes:?}"
    );
}

// Broadcast reaches the other instance on Linux, in a container too; the Windows runs turn
// discovery off so no firewall prompt comes up.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_client_finds_the_server_on_its_network() {
    let pair = Pair::get().await;
    let server_fingerprint = fingerprint(&pair.server_admin).await;
    let server_name = machine_name(&pair.server_admin).await;
    let version = pair
        .server_admin
        .get("/api/v1/system/info")
        .await
        .expect(200)
        .json()["version"]
        .clone();

    let found = eventually(
        "the client to find the server by broadcast",
        WAIT,
        || async {
            let servers = pair
                .client_admin
                .get("/api/v1/client/discovery")
                .await
                .expect(200)
                .json();
            let mut fields = shape::DISCOVERED_SERVER.to_vec();
            // The address as a URL, for the UI.
            fields.push(("url", Kind::Str));
            shape::each(&servers, "GET /api/v1/client/discovery", |server| {
                shape::fields(server, &fields, "a discovered server");
            });
            servers
                .as_array()
                .and_then(|list| {
                    list.iter()
                        .find(|s| s["fingerprint"] == json!(server_fingerprint))
                })
                .cloned()
        },
    )
    .await;
    assert_eq!(found["name"], json!(server_name));
    assert_eq!(found["version"], version);
    assert_eq!(
        found["url"],
        json!(format!(
            "https://{}:{}",
            found["address"].as_str().unwrap_or_default(),
            found["httpsPort"]
        ))
    );
}

#[tokio::test]
async fn the_client_lists_its_devices_and_takes_attach_and_detach() {
    let pair = Pair::get().await;
    let client = &pair.client_admin;
    let linked_server = pair.connect().await;
    let id = linked_server.server_id.as_str();
    let devices = client_devices(client).await;
    let entry = &devices["servers"][0];
    assert_eq!(entry["id"], json!(id));
    assert!(
        ["unconfigured", "connecting", "connected", "error"]
            .contains(&entry["link"].as_str().unwrap_or_default()),
        "{devices}"
    );
    shape::each(&entry["devices"], "client devices", |device| {
        shape::fields(
            device,
            &[
                ("device", Kind::Object),
                ("attachedHere", Kind::Bool),
                ("port", Kind::OptInt),
                ("attachedSince", Kind::OptTime),
                ("fault", Kind::OptStr),
                ("attachedVia", Kind::OptStr),
            ],
            "a client device",
        );
        shape::fields(&device["device"], shape::DEVICE, "a client device's device");
    });

    let attachment = |server: &str, bus_id: &str| {
        format!("/api/v1/client/servers/{server}/devices/{bus_id}/attachment")
    };
    for bad in ["bogus", "1-", "a-1"] {
        client
            .put_empty(&attachment(id, bad))
            .await
            .problem(&["busId"]);
        client
            .delete(&attachment(id, bad))
            .await
            .problem(&["busId"]);
    }
    client
        .put_empty(&attachment(id, "9-9"))
        .await
        .expect(202)
        .empty();
    client
        .delete(&attachment(id, "9-9"))
        .await
        .expect(202)
        .empty();
    // Detaching what is not attached is fine too.
    client.delete(&attachment(id, "9-9")).await.expect(202);

    // A server the client does not have has no devices to attach.
    client
        .put_empty(&attachment("server-none", "9-9"))
        .await
        .expect(404);
    client
        .delete(&attachment("server-none", "9-9"))
        .await
        .expect(404);
}

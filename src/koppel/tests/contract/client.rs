//! The client role, against a real server: pairing through the code (approve, a wrong code, refuse,
//! cancel, and the confirm step when it would replace a connection), the stored connection and its
//! test, servers found on the network, and the client's devices. The client target reaches the main
//! one at its own address.

use std::time::Duration;

use crate::support::shape::{self, Kind};
use crate::support::{Body, Pair, Session, activity_with, eventually, json, tls, unique};
use reqwest::Method;
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(60);

async fn pairing(client: &Session) -> Value {
    let status = client
        .get("/api/v1/client/pairing")
        .await
        .expect(200)
        .json();
    shape::fields(&status, shape::PAIRING_STATUS, "GET /api/v1/client/pairing");
    status
}

async fn connection(client: &Session) -> Value {
    let value = client
        .get("/api/v1/client/connection")
        .await
        .expect(200)
        .json();
    shape::fields(&value, shape::CONNECTION, "GET /api/v1/client/connection");
    value
}

async fn connection_test(client: &Session) -> Value {
    let answer = client
        .post_empty("/api/v1/client/connection/test")
        .await
        .expect(200)
        .json();
    shape::fields(
        &answer,
        shape::CONNECTION_TEST,
        "POST /api/v1/client/connection/test",
    );
    answer
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

/// Opens `server`'s pairing window, which Skakel 0.3.0 keeps closed until a person opens it, and
/// waits until it shows as open.
async fn open_pairing(server: &Session) {
    server
        .put("/api/v1/server/pairing/window", json!({ "minutes": 5 }))
        .await
        .expect(202)
        .empty();
    eventually("the pairing window to open", WAIT, || async {
        let overview = server
            .get("/api/v1/server/pairing")
            .await
            .expect(200)
            .json();
        (overview["window"]["open"] == json!(true)).then_some(())
    })
    .await;
}

/// Starts pairing on `client` with the server at `url`, and waits for the code.
async fn start_pairing(client: &Session, url: &str, fingerprint: &str, name: &str) -> Value {
    client
        .post(
            "/api/v1/client/pairing",
            json!({ "serverUrl": url, "serverFingerprint": fingerprint, "serverName": name }),
        )
        .await
        .expect(202)
        .empty();
    eventually("the client to show its code", WAIT, || async {
        let status = pairing(client).await;
        match status["state"].as_str() {
            Some("waiting") => Some(status),
            // The 202 already shows requesting; idle here would be the old race.
            Some("requesting") => None,
            other => panic!("pairing went to {other:?}: {status}"),
        }
    })
    .await
}

/// The request on `server` from the client named `client_name`.
async fn request_from(server: &Session, client_name: &str) -> Value {
    eventually("the request to reach the server", WAIT, || async {
        let overview = server
            .get("/api/v1/server/pairing")
            .await
            .expect(200)
            .json();
        shape::fields(
            &overview,
            shape::PAIRING_OVERVIEW,
            "GET /api/v1/server/pairing",
        );
        overview["requests"]
            .as_array()
            .and_then(|requests| {
                requests.iter().find(|r| {
                    r["clientName"] == json!(client_name) && r["state"] == json!("waiting")
                })
            })
            .cloned()
    })
    .await
}

async fn requests_on(server: &Session) -> Vec<Value> {
    server
        .get("/api/v1/server/pairing")
        .await
        .expect(200)
        .json()["requests"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// A client registered on the server through its UI, with its credentials.
async fn new_client(server: &Session) -> (String, String) {
    let created = server
        .post(
            "/api/v1/server/clients",
            json!({ "name": unique("contract-client") }),
        )
        .await
        .expect(201)
        .json();
    (
        created["clientId"].as_str().expect("an id").to_owned(),
        created["clientSecret"]
            .as_str()
            .expect("a secret")
            .to_owned(),
    )
}

async fn save_connection(client: &Session, url: &str, fingerprint: &str, id: &str, secret: &str) {
    client
        .put(
            "/api/v1/client/connection",
            json!({ "serverUrl": url, "serverFingerprint": fingerprint, "clientId": id, "clientSecret": secret }),
        )
        .await
        .expect(202)
        .empty();
}

#[tokio::test]
async fn a_client_that_was_never_set_up_is_unconfigured() {
    let pair = Pair::get().await;
    let stored = connection(&pair.client_admin).await;
    assert!(stored["serverUrl"].is_null(), "{stored}");
    assert_eq!(stored["hasSecret"], json!(false));
    let test = connection_test(&pair.client_admin).await;
    assert_eq!(test["ok"], json!(false));
    assert_eq!(test["outcome"], json!("incomplete"));
    let devices = pair
        .client_admin
        .get("/api/v1/client/devices")
        .await
        .expect(200)
        .json();
    assert_eq!(devices["link"], json!("unconfigured"));
    assert_eq!(pairing(&pair.client_admin).await["state"], json!("idle"));
}

#[tokio::test]
async fn pairing_with_the_code_registers_the_client_and_links_it() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    let server_name = machine_name(server).await;
    let client_name = machine_name(client).await;
    assert_eq!(pairing(client).await["state"], json!("idle"));

    // A connection to this same server already, which an earlier test may have left pointing
    // elsewhere: an approval from the server it is connected to replaces the credentials without asking.
    let (old_id, old_secret) = new_client(server).await;
    save_connection(
        client,
        &pair.peer_url,
        &server_fingerprint,
        &old_id,
        &old_secret,
    )
    .await;

    // The address and fingerprint are checked before anything is sent.
    client
        .post(
            "/api/v1/client/pairing",
            json!({ "serverUrl": "http://elsewhere:5240", "serverFingerprint": server_fingerprint }),
        )
        .await
        .problem(&["serverUrl"]);
    client
        .post(
            "/api/v1/client/pairing",
            json!({ "serverUrl": pair.peer_url, "serverFingerprint": "AB:CD" }),
        )
        .await
        .problem(&["serverFingerprint"]);
    client
        .post("/api/v1/client/pairing", json!({}))
        .await
        .problem(&["serverUrl", "serverFingerprint"]);

    // The fingerprint may be typed in any spelling.
    open_pairing(server).await;
    let typed = server_fingerprint.replace(':', "").to_ascii_lowercase();
    let waiting = start_pairing(
        client,
        &format!("{}/ignored/path", pair.peer_url),
        &typed,
        &server_name,
    )
    .await;
    let code = waiting["code"].as_str().expect("a code").to_owned();
    assert!(
        code.len() == 8 && code.chars().all(|c| c.is_ascii_digit()),
        "eight digits: {code}"
    );
    assert!(waiting["expiresAt"].is_string());
    assert_eq!(waiting["serverName"], json!(server_name));
    assert_eq!(waiting["serverUrl"], json!(pair.peer_url));
    // The status carries the fingerprint as typed or normalized; the digits are what count.
    assert_eq!(
        tls::normalize(waiting["serverFingerprint"].as_str().unwrap_or_default()),
        server_fingerprint
    );

    let request = request_from(server, &client_name).await;
    shape::fields(&request, shape::PAIRING_REQUEST, "a pairing request");
    let request_id = request["id"].as_str().expect("an id").to_owned();
    let approve = format!("/api/v1/server/pairing/requests/{request_id}/approval");

    let wrong_code = if code == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    server
        .post(&approve, json!({ "code": wrong_code }))
        .await
        .problem(&["code"]);
    assert!(
        requests_on(server)
            .await
            .iter()
            .any(|r| r["id"] == json!(request_id)),
        "a wrong code leaves the request"
    );

    server
        .post(&approve, json!({ "code": code }))
        .await
        .expect(202)
        .empty();

    let paired = eventually("the client to be paired", WAIT, || async {
        let status = pairing(client).await;
        (status["state"] == json!("paired")).then_some(status)
    })
    .await;
    assert!(paired["code"].is_null());
    assert_eq!(
        tls::normalize(paired["serverFingerprint"].as_str().unwrap_or_default()),
        server_fingerprint
    );

    let stored = connection(client).await;
    assert_eq!(stored["serverUrl"], json!(pair.peer_url));
    assert_eq!(stored["serverFingerprint"], json!(server_fingerprint));
    assert_eq!(stored["hasSecret"], json!(true));
    let client_id = stored["clientId"].as_str().expect("a client id").to_owned();
    assert_ne!(
        client_id, old_id,
        "the paired credentials replaced the old ones"
    );

    let clients = server
        .get("/api/v1/server/clients")
        .await
        .expect(200)
        .json();
    let row = clients
        .as_array()
        .and_then(|list| list.iter().find(|c| c["id"] == json!(client_id)))
        .cloned()
        .expect("the server lists the paired client");
    assert_eq!(row["name"], json!(client_name));
    assert!(
        !requests_on(server)
            .await
            .iter()
            .any(|r| r["id"] == json!(request_id))
    );
    server
        .post(&approve, json!({ "code": code }))
        .await
        .expect(404);

    let test = connection_test(client).await;
    assert_eq!(test["ok"], json!(true), "{test}");
    assert_eq!(test["outcome"], json!("ok"));

    let linked = eventually("the client to link", WAIT, || async {
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
        (devices["link"] == json!("connected")).then_some(devices)
    })
    .await;
    let shared = server
        .get("/api/v1/server/devices")
        .await
        .expect(200)
        .json();
    assert_eq!(
        linked["devices"].as_array().map(Vec::len),
        shared["devices"].as_array().map(Vec::len),
        "the client sees what the server has"
    );

    let added = activity_with(server, "clientAdded").await;
    let entry = added
        .iter()
        .find(|e| e["entity"]["id"] == json!(client_id))
        .expect("a clientAdded entry for the pairing");
    assert_eq!(entry["outcome"], json!("succeeded"));

    server
        .delete(&format!("/api/v1/server/clients/{old_id}"))
        .await
        .expect(202);
}

#[tokio::test]
async fn a_server_with_its_pairing_closed_tells_the_client_how_to_open_it() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    server
        .delete("/api/v1/server/pairing/window")
        .await
        .expect(202)
        .empty();
    eventually("the pairing window to close", WAIT, || async {
        let overview = server
            .get("/api/v1/server/pairing")
            .await
            .expect(200)
            .json();
        (overview["window"]["open"] == json!(false)).then_some(())
    })
    .await;

    client
        .post(
            "/api/v1/client/pairing",
            json!({ "serverUrl": pair.peer_url, "serverFingerprint": server_fingerprint, "serverName": "main" }),
        )
        .await
        .expect(202)
        .empty();
    let failed = eventually("the client to hear the server is closed", WAIT, || async {
        let status = pairing(client).await;
        match status["state"].as_str() {
            Some("failed") => Some(status),
            Some("requesting") => None,
            other => panic!("pairing went to {other:?}: {status}"),
        }
    })
    .await;
    let message = failed["message"].as_str().expect("a message");
    assert!(
        message.contains("not taking pairing requests") && message.contains("Allow pairing"),
        "{message}"
    );
    client
        .delete("/api/v1/client/pairing")
        .await
        .expect(202)
        .empty();
}

#[tokio::test]
async fn a_refused_request_tells_the_client_and_a_cancelled_one_leaves_the_server() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    let client_name = machine_name(client).await;

    open_pairing(server).await;
    start_pairing(client, &pair.peer_url, &server_fingerprint, "main").await;
    let request = request_from(server, &client_name).await;
    let request_id = request["id"].as_str().expect("an id").to_owned();
    server
        .delete(&format!("/api/v1/server/pairing/requests/{request_id}"))
        .await
        .expect(202)
        .empty();
    eventually("the client to hear it was refused", WAIT, || async {
        (pairing(client).await["state"] == json!("denied")).then_some(())
    })
    .await;
    assert!(
        !requests_on(server)
            .await
            .iter()
            .any(|r| r["id"] == json!(request_id))
    );
    server
        .delete(&format!("/api/v1/server/pairing/requests/{request_id}"))
        .await
        .expect(404);
    let refused = activity_with(server, "clientAdded").await;
    assert!(
        refused.iter().any(|e| e["outcome"] == json!("failed")),
        "a refused pairing is a failed clientAdded entry"
    );

    open_pairing(server).await;
    start_pairing(client, &pair.peer_url, &server_fingerprint, "main").await;
    let request = request_from(server, &client_name).await;
    let request_id = request["id"].as_str().expect("an id").to_owned();
    client
        .delete("/api/v1/client/pairing")
        .await
        .expect(202)
        .empty();
    assert_eq!(pairing(client).await["state"], json!("idle"));
    eventually("the server to drop the withdrawn request", WAIT, || async {
        (!requests_on(server)
            .await
            .iter()
            .any(|r| r["id"] == json!(request_id)))
        .then_some(())
    })
    .await;
    // Cancelling with nothing going on is fine.
    client.delete("/api/v1/client/pairing").await.expect(202);
}

#[tokio::test]
async fn three_wrong_codes_refuse_the_request_and_the_client_hears_it() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    let client_name = machine_name(client).await;

    open_pairing(server).await;
    let waiting = start_pairing(client, &pair.peer_url, &server_fingerprint, "main").await;
    let code = waiting["code"].as_str().expect("a code").to_owned();
    let request = request_from(server, &client_name).await;
    let approve = format!(
        "/api/v1/server/pairing/requests/{}/approval",
        request["id"].as_str().unwrap_or_default()
    );
    let wrong = if code == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    for _ in 0..3 {
        let answer = server.post(&approve, json!({ "code": wrong })).await;
        assert!(
            [400, 404].contains(&answer.status),
            "a wrong code: {}",
            answer.status
        );
    }
    server
        .post(&approve, json!({ "code": code }))
        .await
        .expect(404);
    eventually("the client to hear it was refused", WAIT, || async {
        (pairing(client).await["state"] == json!("denied")).then_some(())
    })
    .await;
}

#[tokio::test]
async fn pairing_with_another_server_waits_for_the_person_to_confirm() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    client
        .post_empty("/api/v1/client/pairing/confirmation")
        .await
        .problem(&["pairing"]);

    // Connected to the main target first.
    let main_fingerprint = fingerprint(server).await;
    let (id, secret) = new_client(server).await;
    save_connection(client, &pair.peer_url, &main_fingerprint, &id, &secret).await;

    // The other server is the client target itself, as both: it serves pairing on its own port.
    client
        .patch("/api/v1/settings", json!({ "role": "both" }))
        .await
        .expect(202);
    let own = client
        .get("/api/v1/server/identity")
        .await
        .expect(200)
        .json();
    let own_url = format!("https://localhost:{}", own["httpsPort"]);
    let own_fingerprint = own["fingerprint"]
        .as_str()
        .expect("a fingerprint")
        .to_owned();
    let client_name = machine_name(client).await;

    let approve_self = || async {
        open_pairing(client).await;
        let waiting = start_pairing(client, &own_url, &own_fingerprint, "itself").await;
        let request = request_from(client, &client_name).await;
        client
            .post(
                &format!(
                    "/api/v1/server/pairing/requests/{}/approval",
                    request["id"].as_str().unwrap_or_default()
                ),
                json!({ "code": waiting["code"] }),
            )
            .await
            .expect(202);
        eventually(
            "the client to ask before replacing its connection",
            WAIT,
            || async {
                let status = pairing(client).await;
                (status["state"] == json!("confirm")).then_some(status)
            },
        )
        .await
    };

    let confirm = approve_self().await;
    assert_eq!(confirm["serverName"], json!("itself"));
    assert_eq!(confirm["serverFingerprint"], json!(own_fingerprint));
    assert!(confirm["message"].is_string());
    assert_eq!(
        connection(client).await["serverFingerprint"],
        json!(main_fingerprint),
        "nothing saved yet"
    );

    // Discard keeps the connection.
    client.delete("/api/v1/client/pairing").await.expect(202);
    assert_eq!(pairing(client).await["state"], json!("idle"));
    assert_eq!(
        connection(client).await["serverFingerprint"],
        json!(main_fingerprint)
    );

    // Replace saves the new one.
    approve_self().await;
    client
        .post_empty("/api/v1/client/pairing/confirmation")
        .await
        .expect(202)
        .empty();
    eventually("the new connection to be saved", WAIT, || async {
        (connection(client).await["serverFingerprint"] == json!(own_fingerprint)).then_some(())
    })
    .await;
    eventually("the client to be paired", WAIT, || async {
        (pairing(client).await["state"] == json!("paired")).then_some(())
    })
    .await;
    client
        .post_empty("/api/v1/client/pairing/confirmation")
        .await
        .problem(&["pairing"]);

    server
        .delete(&format!("/api/v1/server/clients/{id}"))
        .await
        .expect(202);
}

#[tokio::test]
async fn a_saved_connection_is_checked_normalized_and_never_shows_its_secret() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    let (id, secret) = new_client(server).await;
    let valid = json!({
        "serverUrl": pair.peer_url,
        "serverFingerprint": server_fingerprint,
        "clientId": id,
        "clientSecret": secret,
    });
    let with = |field: &str, value: Value| {
        let mut body = valid.clone();
        body[field] = value;
        body
    };

    for (body, fields) in [
        (
            with("serverUrl", json!("http://elsewhere:5240")),
            vec!["serverUrl"],
        ),
        (with("serverUrl", json!("not a url")), vec!["serverUrl"]),
        (
            with("serverFingerprint", json!("AB:CD")),
            vec!["serverFingerprint"],
        ),
        (with("clientId", json!("   ")), vec!["clientId"]),
        (
            json!({}),
            vec!["serverUrl", "serverFingerprint", "clientId"],
        ),
    ] {
        client
            .put("/api/v1/client/connection", body)
            .await
            .problem(&fields);
    }
    client
        .change(
            Method::PUT,
            "/api/v1/client/connection",
            Body::Raw("application/json", "not json".to_owned()),
        )
        .await
        .expect(400);

    // Stored as the server's address alone, the fingerprint in Koppel's spelling, the id trimmed.
    let typed = server_fingerprint.replace(':', "").to_ascii_lowercase();
    client
        .put(
            "/api/v1/client/connection",
            json!({
                "serverUrl": format!("{}/some/path?x=1", pair.peer_url),
                "serverFingerprint": typed,
                "clientId": format!("  {id}  "),
                "clientSecret": secret,
            }),
        )
        .await
        .expect(202)
        .empty();
    let stored = connection(client).await;
    assert_eq!(stored["serverUrl"], json!(pair.peer_url));
    assert_eq!(stored["serverFingerprint"], json!(server_fingerprint));
    assert_eq!(stored["clientId"], json!(id));
    assert_eq!(stored["hasSecret"], json!(true));
    assert!(stored["nextServerFingerprint"].is_null());
    assert!(
        !client
            .get("/api/v1/client/connection")
            .await
            .text()
            .contains(&secret)
    );
    assert_eq!(connection_test(client).await["outcome"], json!("ok"));

    // An empty secret keeps the stored one while the server stays the same, and only then.
    client
        .put("/api/v1/client/connection", with("clientSecret", json!("")))
        .await
        .expect(202);
    assert_eq!(connection(client).await["hasSecret"], json!(true));
    assert_eq!(connection_test(client).await["outcome"], json!("ok"));
    let other = tls::normalize(&"ab".repeat(32));
    client
        .put(
            "/api/v1/client/connection",
            json!({ "serverUrl": pair.peer_url, "serverFingerprint": other, "clientId": id }),
        )
        .await
        .problem(&["clientSecret"]);
    client
        .put(
            "/api/v1/client/connection",
            json!({ "serverUrl": "https://elsewhere:5240", "serverFingerprint": server_fingerprint, "clientId": id }),
        )
        .await
        .problem(&["clientSecret"]);
    assert_eq!(
        connection(client).await["serverFingerprint"],
        json!(server_fingerprint),
        "a refused save changes nothing"
    );

    server
        .delete(&format!("/api/v1/server/clients/{id}"))
        .await
        .expect(202);
}

#[tokio::test]
async fn the_connection_test_names_what_is_wrong_in_fixed_words() {
    let pair = Pair::get().await;
    let (server, client) = (&pair.server_admin, &pair.client_admin);
    let server_fingerprint = fingerprint(server).await;
    let (id, secret) = new_client(server).await;

    let outcome = |url: String, fp: String, secret: String| {
        let (client, id) = (client.clone(), id.clone());
        async move {
            save_connection(&client, &url, &fp, &id, &secret).await;
            let answer = connection_test(&client).await;
            assert!(!answer["message"].as_str().unwrap_or_default().is_empty());
            (
                answer["ok"].clone(),
                answer["outcome"].as_str().unwrap_or_default().to_owned(),
            )
        }
    };

    assert_eq!(
        outcome(
            pair.peer_url.clone(),
            server_fingerprint.clone(),
            secret.clone()
        )
        .await,
        (json!(true), "ok".to_owned())
    );
    assert_eq!(
        outcome(
            pair.peer_url.clone(),
            tls::normalize(&"ab".repeat(32)),
            secret.clone()
        )
        .await,
        (json!(false), "certificateMismatch".to_owned())
    );
    assert_eq!(
        outcome(
            "https://koppel-contract-nowhere.invalid:5240".to_owned(),
            server_fingerprint.clone(),
            secret.clone()
        )
        .await,
        (json!(false), "unreachable".to_owned())
    );

    // Wrong secrets, until the server turns this client away for a while (10 in a minute).
    let mut outcomes = Vec::new();
    for _ in 0..12 {
        let (_, outcome) = outcome(
            pair.peer_url.clone(),
            server_fingerprint.clone(),
            "wrong-secret".to_owned(),
        )
        .await;
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

    server
        .delete(&format!("/api/v1/server/clients/{id}"))
        .await
        .expect(202);
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
    assert!(
        ["unconfigured", "connecting", "connected", "error"]
            .contains(&devices["link"].as_str().unwrap_or_default()),
        "{devices}"
    );
    shape::each(&devices["devices"], "client devices", |device| {
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

    for bad in ["bogus", "1-", "a-1"] {
        client
            .put_empty(&format!("/api/v1/client/devices/{bad}/attachment"))
            .await
            .problem(&["busId"]);
        client
            .delete(&format!("/api/v1/client/devices/{bad}/attachment"))
            .await
            .problem(&["busId"]);
    }
    client
        .put_empty("/api/v1/client/devices/9-9/attachment")
        .await
        .expect(202)
        .empty();
    client
        .delete("/api/v1/client/devices/9-9/attachment")
        .await
        .expect(202)
        .empty();
    // Detaching what is not attached is fine too.
    client
        .delete("/api/v1/client/devices/9-9/attachment")
        .await
        .expect(202);
}

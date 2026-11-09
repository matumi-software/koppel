//! The live push over Server-Sent Events: every event of [`koppel_contract::push::EVENTS`] follows the
//! change that causes it, with the payload the REST read of the same thing has, and the stream ends
//! when its session does. Runs against a target with `KOPPEL_CONTRACT_PUSH=sse`; 0.1.x pushes over
//! SignalR, so against it each test says it was skipped.

use std::time::Duration;

use koppel_contract::push::{EVENTS, Payload};
use koppel_contract::shape::{self, Kind};
use koppel_contract::{
    Events, Main, Next, Pair, Push, Session, json, needs_skakel, password, push_kind, unique,
};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(20);
const OTHER_PASSWORD: &str = "contract-password-2";

/// The events a test below covers. Kept beside [`EVENTS`] so a new event cannot go untested.
const TESTED: &[&str] = &[
    "SettingsChanged",
    "ClientsChanged",
    "ServerKeysChanged",
    "ConnectionChanged",
    "DevicesChanged",
    "ClientDevicesChanged",
    "ActivityRecorded",
    "ServersDiscovered",
    "PairingStatusChanged",
    "PairingRequestsChanged",
];

#[test]
fn every_event_of_the_contract_has_a_test() {
    let mut contract: Vec<&str> = EVENTS.iter().map(|e| e.name).collect();
    let mut tested = TESTED.to_vec();
    contract.sort_unstable();
    tested.sort_unstable();
    assert_eq!(contract, tested);
}

fn sse() -> bool {
    if push_kind() == Push::Sse {
        return true;
    }
    eprintln!("skipped: the target pushes over SignalR, and these tests read Server-Sent Events");
    false
}

/// Checks `value` against the payload the contract gives the event `name`.
#[track_caller]
fn payload(name: &str, value: &Value) {
    let event = EVENTS
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the contract"));
    match event.payload {
        Payload::Object(fields) => shape::fields(value, fields, name),
        Payload::List(fields) => {
            let mut fields = fields.to_vec();
            if name == "ServersDiscovered" {
                // 0.1.x also writes the address as a URL, as GET /api/client/discovery does.
                fields.push(("url", Kind::Str));
            }
            shape::each(value, name, |item| shape::fields(item, &fields, name));
        }
    }
}

/// The live push for `session`, open and answering as an event stream.
async fn open(session: &Session) -> Events {
    let events = session.events().await;
    assert_eq!(events.status, 200, "GET /api/events");
    assert!(
        events.content_type.starts_with("text/event-stream"),
        "{}",
        events.content_type
    );
    events
}

/// Waits for `name` matching `accept`, and checks its payload.
async fn expect(events: &mut Events, name: &str, accept: impl FnMut(&Value) -> bool) -> Value {
    let value = events.wait_for(name, WAIT, accept).await;
    payload(name, &value);
    value
}

#[tokio::test]
async fn settings_changed_carries_the_settings_as_written() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let mut events = open(&main.admin).await;
    main.admin
        .patch("/api/settings", json!({ "shareOnStartup": false }))
        .await
        .expect(202);
    let pushed = expect(&mut events, "SettingsChanged", |s| {
        s["shareOnStartup"] == json!(false)
    })
    .await;
    let read = main.admin.get("/api/settings").await.expect(200).json();
    assert_eq!(pushed, read, "the push carries what a read gives");
}

#[tokio::test]
async fn activity_recorded_carries_the_entry_the_log_has() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    main.admin
        .patch("/api/settings", json!({ "notifyLink": true }))
        .await
        .expect(202);
    let mut events = open(&main.admin).await;
    main.admin
        .patch("/api/settings", json!({ "notifyLink": false }))
        .await
        .expect(202);
    let pushed = expect(&mut events, "ActivityRecorded", |e| {
        e["reason"] == json!("reconfigured")
    })
    .await;
    shape::fields(&pushed["entity"], shape::ENTITY, "ActivityRecorded.entity");
    let log = main
        .admin
        .get("/api/activity?max=500")
        .await
        .expect(200)
        .json();
    assert!(
        log.as_array()
            .is_some_and(|entries| entries.contains(&pushed)),
        "the pushed entry is in the log: {pushed}"
    );
}

#[tokio::test]
async fn clients_changed_carries_the_list_after_an_add_and_a_revoke() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let mut events = open(&main.admin).await;
    let created = main
        .admin
        .post(
            "/api/server/clients",
            json!({ "name": unique("contract-push") }),
        )
        .await
        .expect(201)
        .json();
    let id = created["clientId"].clone();
    expect(&mut events, "ClientsChanged", |list| {
        list.as_array()
            .is_some_and(|l| l.iter().any(|c| c["id"] == id))
    })
    .await;
    main.admin
        .delete(&format!(
            "/api/server/clients/{}",
            id.as_str().unwrap_or_default()
        ))
        .await
        .expect(202);
    expect(&mut events, "ClientsChanged", |list| {
        list.as_array()
            .is_some_and(|l| !l.iter().any(|c| c["id"] == id))
    })
    .await;
}

#[tokio::test]
async fn server_keys_changed_carries_the_keys_after_a_rotation() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let before = main.admin.get("/api/server/keys").await.expect(200).json();
    let mut events = open(&main.admin).await;
    main.admin
        .post_empty("/api/server/keys/signing-key/rotate")
        .await
        .expect(202);
    let pushed = expect(&mut events, "ServerKeysChanged", |k| {
        k["signingKeyId"] != before["signingKeyId"]
    })
    .await;
    let read = main.admin.get("/api/server/keys").await.expect(200).json();
    assert_eq!(pushed, read);
}

#[tokio::test]
async fn connection_changed_carries_the_connection_without_its_secret() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let admin = &main.admin;
    admin
        .patch("/api/settings", json!({ "role": "both" }))
        .await
        .expect(202);
    let mut events = open(admin).await;
    let fingerprint =
        admin.get("/api/server/identity").await.expect(200).json()["fingerprint"].clone();
    let id = unique("client-push");
    let secret = unique("secret");
    admin
        .put(
            "/api/client/connection",
            json!({
                "serverUrl": "https://koppel-contract-elsewhere:5240",
                "serverFingerprint": fingerprint,
                "clientId": id,
                "clientSecret": secret,
            }),
        )
        .await
        .expect(202);
    let pushed = expect(&mut events, "ConnectionChanged", |c| {
        c["clientId"] == json!(id)
    })
    .await;
    assert_eq!(pushed["hasSecret"], json!(true));
    assert!(!pushed.to_string().contains(&secret), "never the secret");
    let read = admin.get("/api/client/connection").await.expect(200).json();
    assert_eq!(pushed, read);
}

#[tokio::test]
async fn client_devices_changed_follows_an_attach_and_a_detach() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let admin = &main.admin;
    admin
        .patch("/api/settings", json!({ "role": "both" }))
        .await
        .expect(202);
    let mut events = open(admin).await;
    admin
        .post_empty("/api/client/devices/9-9/attach")
        .await
        .expect(202);
    expect(&mut events, "ClientDevicesChanged", |_| true).await;
    admin
        .post_empty("/api/client/devices/9-9/detach")
        .await
        .expect(202);
    expect(&mut events, "ClientDevicesChanged", |_| true).await;
}

#[tokio::test]
async fn pairing_status_changed_follows_a_cancel() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let admin = &main.admin;
    admin
        .patch("/api/settings", json!({ "role": "both" }))
        .await
        .expect(202);
    let mut events = open(admin).await;
    admin.delete("/api/client/pairing").await.expect(202);
    let pushed = expect(&mut events, "PairingStatusChanged", |s| {
        s["state"] == json!("idle")
    })
    .await;
    let read = admin.get("/api/client/pairing").await.expect(200).json();
    assert_eq!(pushed, read);
}

#[tokio::test]
async fn devices_changed_follows_a_share_on_a_machine_that_can_share() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let devices = main
        .admin
        .get("/api/server/devices")
        .await
        .expect(200)
        .json();
    let Some(device) = devices["devices"]
        .as_array()
        .and_then(|list| list.first())
        .filter(|_| devices["unavailable"].is_null())
        .cloned()
    else {
        eprintln!(
            "skipped: this target has no USB device it can share ({}), so nothing makes DevicesChanged",
            devices["unavailable"]
        );
        return;
    };
    let bus_id = device["busId"].as_str().unwrap_or_default().to_owned();
    let shared = device["state"] == json!("shared");
    let mut events = open(&main.admin).await;
    main.admin
        .put(
            &format!("/api/server/devices/{bus_id}/shared"),
            json!({ "shared": !shared }),
        )
        .await
        .expect(202);
    expect(&mut events, "DevicesChanged", |_| true).await;
    main.admin
        .put(
            &format!("/api/server/devices/{bus_id}/shared"),
            json!({ "shared": shared }),
        )
        .await
        .expect(202);
}

#[tokio::test]
async fn pairing_requests_changed_follows_a_client_asking_to_pair() {
    let Some(pair) = Pair::get().await else {
        return;
    };
    if !sse() || !needs_skakel("a pairing request") {
        return;
    }
    let identity = pair
        .server_admin
        .get("/api/server/identity")
        .await
        .expect(200)
        .json();
    let mut events = open(&pair.server_admin).await;
    pair.client_admin
        .post(
            "/api/client/pairing",
            json!({ "serverUrl": pair.peer_url, "serverFingerprint": identity["fingerprint"], "serverName": identity["name"] }),
        )
        .await
        .expect(202);
    expect(&mut events, "PairingRequestsChanged", |o| {
        o["requests"].as_array().is_some_and(|r| !r.is_empty())
    })
    .await;
    pair.client_admin
        .delete("/api/client/pairing")
        .await
        .expect(202);
}

#[tokio::test]
async fn servers_discovered_follows_the_client_finding_the_server() {
    let Some(pair) = Pair::get().await else {
        return;
    };
    if !sse() || !needs_skakel("discovery") {
        return;
    }
    let fingerprint = pair
        .server_admin
        .get("/api/server/identity")
        .await
        .expect(200)
        .json()["fingerprint"]
        .clone();
    // Discovery stops outside the client role and starts again in it, which finds the server anew.
    pair.client_admin
        .patch("/api/settings", json!({ "role": "server" }))
        .await
        .expect(202);
    let mut events = open(&pair.client_admin).await;
    pair.client_admin
        .patch("/api/settings", json!({ "role": "client" }))
        .await
        .expect(202);
    let found = events
        .wait_for("ServersDiscovered", Duration::from_secs(60), |list| {
            list.as_array()
                .is_some_and(|l| l.iter().any(|s| s["fingerprint"] == fingerprint))
        })
        .await;
    payload("ServersDiscovered", &found);
}

#[tokio::test]
async fn signing_out_ends_that_sessions_stream_only() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let leaving = main.target.sign_in().await;
    let mut ended = open(&leaving).await;
    let mut staying = open(&main.admin).await;
    leaving.post_empty("/api/auth/logout").await.expect(204);
    assert!(
        ended.ends_within(WAIT).await,
        "signing out ends the session's stream"
    );
    main.admin
        .patch("/api/settings", json!({ "notifyPairing": false }))
        .await
        .expect(202);
    expect(&mut staying, "SettingsChanged", |s| {
        s["notifyPairing"] == json!(false)
    })
    .await;
}

#[tokio::test]
async fn a_password_change_ends_the_other_sessions_streams_and_keeps_its_own() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !sse() {
        return;
    }
    let changer = main.target.sign_in().await;
    let other = main.target.sign_in().await;
    let mut kept = open(&changer).await;
    let mut ended = open(&other).await;
    let changed = changer
        .post(
            "/api/auth/password",
            json!({ "current": password(), "next": OTHER_PASSWORD }),
        )
        .await
        .expect(204);
    assert!(
        ended.ends_within(WAIT).await,
        "the other session's stream ends"
    );
    let renewed = main.target.with_cookie(
        &changed
            .session_cookie()
            .expect("the changing session gets a new cookie"),
    );
    renewed
        .patch("/api/settings", json!({ "notifyPairing": true }))
        .await
        .expect(202);
    match kept.next(WAIT).await {
        Next::Event(..) => {}
        other => panic!("the changing session's stream should carry on, got {other:?}"),
    }
    renewed
        .post(
            "/api/auth/password",
            json!({ "current": OTHER_PASSWORD, "next": password() }),
        )
        .await
        .expect(204);
}

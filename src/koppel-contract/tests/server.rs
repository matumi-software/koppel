//! The server role's UI endpoints: its identity, registered clients and their secrets, the
//! certificate, its USB devices, the pairing window and the pairing requests waiting for a code. Also
//! `/api/remote/devices`, which other Koppel machines call with a token from Skakel's token endpoint.

use std::time::Duration;

use koppel_contract::shape::{self, Kind};
use koppel_contract::{
    Body, Main, Session, Target, activity_with, eventually, json, needs_skakel, skakel_path, unique,
};
use reqwest::Method;
use serde_json::Value;

/// A token from Skakel's token endpoint (OAuth client credentials), or the status it refused with.
async fn token(target: &Target, client_id: &str, secret: &str) -> Result<String, u16> {
    let path = skakel_path(target, "token").await;
    let answer = target
        .anonymous()
        .send(
            Method::POST,
            &path,
            Body::Raw(
                "application/x-www-form-urlencoded",
                format!(
                    "grant_type=client_credentials&client_id={client_id}&client_secret={secret}"
                ),
            ),
            &[],
        )
        .await;
    if answer.status != 200 {
        return Err(answer.status);
    }
    Ok(answer.json()["access_token"]
        .as_str()
        .expect("a token")
        .to_owned())
}

async fn remote_devices(target: &Target, token: &str) -> u16 {
    let bearer = format!("Bearer {token}");
    let answer = target
        .anonymous()
        .send(
            Method::GET,
            "/api/remote/devices",
            Body::None,
            &[("Authorization", bearer.as_str())],
        )
        .await;
    if answer.status == 200 {
        let body = answer.json();
        shape::fields(
            &body,
            &[("devices", Kind::Array)],
            "GET /api/remote/devices",
        );
        shape::each(&body["devices"], "remote devices", |device| {
            shape::fields(device, shape::DEVICE, "a remote device");
        });
    }
    answer.status
}

async fn create_client(admin: &Session, name: &str) -> Value {
    let answer = admin
        .post("/api/server/clients", json!({ "name": name }))
        .await
        .expect(201);
    let created = answer.json();
    shape::fields(
        &created,
        &[
            ("clientId", Kind::Str),
            ("clientSecret", Kind::Str),
            ("name", Kind::Str),
        ],
        "POST /api/server/clients",
    );
    assert_eq!(
        answer.header("location"),
        Some(format!(
            "/api/server/clients/{}",
            created["clientId"].as_str().unwrap_or_default()
        )),
    );
    created
}

async fn clients(admin: &Session) -> Vec<Value> {
    let list = admin.get("/api/server/clients").await.expect(200).json();
    shape::each(&list, "GET /api/server/clients", |client| {
        shape::fields(client, shape::CLIENT, "a client");
    });
    list.as_array().cloned().unwrap_or_default()
}

fn has_entry(entries: &[Value], id: &str) -> bool {
    entries.iter().any(|e| e["entity"]["id"] == json!(id))
}

#[tokio::test]
async fn identity_keys_and_info_name_the_certificate_the_port_presents() {
    let Some(main) = Main::get().await else {
        return;
    };
    let identity = main
        .admin
        .get("/api/server/identity")
        .await
        .expect(200)
        .json();
    shape::fields(
        &identity,
        &[
            ("name", Kind::Str),
            ("fingerprint", Kind::Str),
            ("httpsPort", Kind::Int),
        ],
        "GET /api/server/identity",
    );
    let info = main.admin.get("/api/system/info").await.expect(200).json();
    let keys = main.admin.get("/api/server/keys").await.expect(200).json();
    shape::fields(&keys, shape::KEYS, "GET /api/server/keys");
    assert_eq!(
        identity["fingerprint"],
        json!(main.target.current_certificate().await),
        "the SHA-256 of the certificate the port presents, as upper case hex pairs"
    );
    assert_eq!(identity["fingerprint"], info["fingerprint"]);
    assert_eq!(identity["fingerprint"], keys["fingerprint"]);
    assert_eq!(identity["name"], info["name"]);
    assert_eq!(identity["httpsPort"], info["httpsPort"]);

    if !needs_skakel("/skakel/info") {
        return;
    }
    let skakel = main
        .target
        .anonymous()
        .get("/skakel/info")
        .await
        .expect(200)
        .json();
    assert_eq!(skakel["app"]["id"], json!("koppel"));
    assert_eq!(skakel["app"]["version"], info["version"]);
}

#[tokio::test]
async fn a_client_is_added_its_secret_copied_and_rotated_and_it_is_revoked() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    let name = unique("contract");
    let created = create_client(admin, &format!("  {name}  ")).await;
    let id = created["clientId"].as_str().expect("an id").to_owned();
    let first_secret = created["clientSecret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    assert_eq!(created["name"], json!(name), "the name is trimmed");

    let listed = clients(admin).await;
    let row = listed
        .iter()
        .find(|c| c["id"] == json!(id))
        .expect("the new client is listed");
    assert_eq!(row["name"], json!(name));
    assert_eq!(row["hasSecret"], json!(true));
    assert_eq!(row["hasNextCertificate"], json!(false));

    let secret = admin
        .get(&format!("/api/server/clients/{id}/secret"))
        .await
        .expect(200)
        .json();
    shape::fields(&secret, &[("clientSecret", Kind::Str)], "GET secret");
    assert_eq!(secret["clientSecret"], json!(first_secret));

    // The credentials work for the remote API.
    let tokens = needs_skakel("using the client's credentials for a token");
    let first_token = if tokens {
        let first_token = token(&main.target, &id, &first_secret)
            .await
            .expect("a token for the new client");
        assert_eq!(remote_devices(&main.target, &first_token).await, 200);
        first_token
    } else {
        String::new()
    };

    let rotated = admin
        .post_empty(&format!("/api/server/clients/{id}/rotate"))
        .await
        .expect(200)
        .json();
    shape::fields(&rotated, &[("clientSecret", Kind::Str)], "POST rotate");
    let second_secret = rotated["clientSecret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    assert_ne!(second_secret, first_secret);
    assert_eq!(
        admin
            .get(&format!("/api/server/clients/{id}/secret"))
            .await
            .expect(200)
            .json()["clientSecret"],
        json!(second_secret)
    );
    let second_token = if tokens {
        assert_eq!(
            remote_devices(&main.target, &first_token).await,
            401,
            "rotating blocks the client's tokens at once"
        );
        assert_eq!(token(&main.target, &id, &first_secret).await, Err(401));
        let second_token = token(&main.target, &id, &second_secret)
            .await
            .expect("a token with the new secret");
        assert_eq!(remote_devices(&main.target, &second_token).await, 200);
        second_token
    } else {
        String::new()
    };

    let revoked = admin
        .delete(&format!("/api/server/clients/{id}"))
        .await
        .expect(202);
    revoked.empty();
    assert!(!clients(admin).await.iter().any(|c| c["id"] == json!(id)));
    if tokens {
        assert_eq!(
            remote_devices(&main.target, &second_token).await,
            401,
            "revoking blocks the client's tokens at once"
        );
    }
    admin
        .get(&format!("/api/server/clients/{id}/secret"))
        .await
        .expect(404);
    admin
        .post_empty(&format!("/api/server/clients/{id}/rotate"))
        .await
        .expect(404);
    admin
        .delete(&format!("/api/server/clients/{id}"))
        .await
        .expect(404);

    for reason in ["clientAdded", "secretRotated", "clientRevoked"] {
        let entries = activity_with(admin, reason).await;
        assert!(has_entry(&entries, &id), "a {reason} entry for {id}");
        let entry = entries
            .iter()
            .find(|e| e["entity"]["id"] == json!(id))
            .expect("the entry");
        assert_eq!(entry["entity"]["type"], json!("client"));
        assert_eq!(entry["outcome"], json!("succeeded"));
    }
}

#[tokio::test]
async fn a_client_needs_a_name_of_up_to_80_visible_characters() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    for name in [
        json!(""),
        json!("   "),
        json!("x".repeat(81)),
        json!("\u{200B}\u{200B}"),
        Value::Null,
    ] {
        admin
            .post("/api/server/clients", json!({ "name": name }))
            .await
            .problem(&["name"]);
    }
    admin
        .post("/api/server/clients", json!({}))
        .await
        .problem(&["name"]);
    admin
        .change(
            Method::POST,
            "/api/server/clients",
            Body::Raw("application/json", "not json".to_owned()),
        )
        .await
        .expect(400);

    let longest = create_client(admin, &"x".repeat(80)).await;
    admin
        .delete(&format!(
            "/api/server/clients/{}",
            longest["clientId"].as_str().unwrap_or_default()
        ))
        .await
        .expect(202);
}

#[tokio::test]
async fn unknown_clients_are_not_found() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    admin
        .get("/api/server/clients/client-nobody/secret")
        .await
        .expect(404);
    admin
        .post_empty("/api/server/clients/client-nobody/rotate")
        .await
        .expect(404);
    admin
        .delete("/api/server/clients/client-nobody")
        .await
        .expect(404);
}

#[tokio::test]
async fn the_pairing_window_opens_and_closes_and_is_closed_to_start_with() {
    let Some(main) = Main::get().await else {
        return;
    };
    if !needs_skakel("the pairing window") {
        return;
    }
    let admin = &main.admin;
    let window = || async {
        let overview = admin.get("/api/server/pairing").await.expect(200).json();
        shape::fields(
            &overview,
            shape::PAIRING_OVERVIEW,
            "GET /api/server/pairing",
        );
        let window = overview["window"].clone();
        shape::fields(&window, shape::PAIRING_WINDOW, "the pairing window");
        window
    };

    admin
        .post("/api/server/pairing/window", json!({ "minutes": 0 }))
        .await
        .problem(&["minutes"]);
    admin
        .post("/api/server/pairing/window", json!({ "minutes": 61 }))
        .await
        .problem(&["minutes"]);

    admin
        .post("/api/server/pairing/window", json!({ "minutes": 5 }))
        .await
        .expect(202)
        .empty();
    let open = eventually("the window to open", Duration::from_secs(20), || async {
        let now = window().await;
        (now["open"] == json!(true)).then_some(now)
    })
    .await;
    assert!(
        open["until"].is_string(),
        "an open window has an end: {open}"
    );
    assert_eq!(open["alwaysOpen"], json!(false));

    admin
        .delete("/api/server/pairing/window")
        .await
        .expect(202)
        .empty();
    let closed = eventually("the window to close", Duration::from_secs(20), || async {
        let now = window().await;
        (now["open"] == json!(false)).then_some(now)
    })
    .await;
    assert!(closed["until"].is_null(), "{closed}");
}

#[tokio::test]
async fn a_new_certificate_is_prepared_cancelled_and_switched_to_in_turn() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    let first = admin.get("/api/server/keys").await.expect(200).json();
    assert!(first["nextFingerprint"].is_null());

    // Out of turn: nothing to cancel or switch to.
    admin
        .delete("/api/server/keys/next-certificate")
        .await
        .problem(&["certificate"]);
    admin
        .post_empty("/api/server/keys/next-certificate/switch")
        .await
        .problem(&["certificate"]);

    admin
        .post_empty("/api/server/keys/next-certificate")
        .await
        .expect(202)
        .empty();
    let prepared = admin.get("/api/server/keys").await.expect(200).json();
    shape::fields(&prepared, shape::KEYS, "GET /api/server/keys");
    assert!(prepared["nextFingerprint"].is_string());
    assert!(prepared["nextPreparedAt"].is_string());
    assert_eq!(
        prepared["fingerprint"], first["fingerprint"],
        "not switched yet"
    );
    admin
        .post_empty("/api/server/keys/next-certificate")
        .await
        .problem(&["certificate"]);

    admin
        .delete("/api/server/keys/next-certificate")
        .await
        .expect(202)
        .empty();
    let cancelled = admin.get("/api/server/keys").await.expect(200).json();
    assert!(cancelled["nextFingerprint"].is_null());
    assert!(cancelled["nextPreparedAt"].is_null());

    admin
        .post_empty("/api/server/keys/next-certificate")
        .await
        .expect(202);
    let next = admin.get("/api/server/keys").await.expect(200).json()["nextFingerprint"]
        .as_str()
        .expect("a next fingerprint")
        .to_owned();
    main.target.trust(&next);
    admin
        .post_empty("/api/server/keys/next-certificate/switch")
        .await
        .expect(202)
        .empty();

    let switched = admin.get("/api/server/keys").await.expect(200).json();
    assert_eq!(switched["fingerprint"], json!(next));
    assert!(switched["nextFingerprint"].is_null());
    let identity = admin.get("/api/server/identity").await.expect(200).json();
    assert_eq!(identity["fingerprint"], json!(next));
    let info = admin.get("/api/system/info").await.expect(200).json();
    assert_eq!(info["fingerprint"], json!(next));

    // A new connection gets the new certificate at once.
    assert_eq!(main.target.current_certificate().await, next);

    for reason in [
        "certificatePrepared",
        "certificateCancelled",
        "certificateSwitched",
    ] {
        let entries = activity_with(admin, reason).await;
        assert!(has_entry(&entries, "machine"), "a {reason} entry");
    }
}

#[tokio::test]
async fn the_server_lists_its_devices_and_refuses_a_share_it_cannot_make() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    let devices = admin.get("/api/server/devices").await.expect(200).json();
    shape::fields(&devices, shape::SERVER_DEVICES, "GET /api/server/devices");
    shape::each(&devices["devices"], "server devices", |device| {
        shape::fields(device, shape::DEVICE, "a server device");
    });

    // The kernel's form, NUMBER-NUMBER(.NUMBER)*, up to 31 characters.
    let too_long = format!("1-{}", "1".repeat(30));
    for bad in ["bogus", "1-", "a-1", "1-1.", "1-1:1.0", too_long.as_str()] {
        admin
            .put(
                &format!("/api/server/devices/{bad}/shared"),
                json!({ "shared": true }),
            )
            .await
            .problem(&["busId"]);
    }
    admin
        .change(
            Method::PUT,
            "/api/server/devices/1-1/shared",
            Body::Raw("application/json", "not json".to_owned()),
        )
        .await
        .expect(400);

    if devices["unavailable"].is_string() {
        // A machine without usbip-host, such as a container without the module.
        admin
            .put("/api/server/devices/1-1/shared", json!({ "shared": true }))
            .await
            .problem(&["device"]);
    } else {
        eprintln!("not checked: this target can share, so a refused share cannot be shown");
    }
}

#[tokio::test]
async fn pairing_requests_that_do_not_exist_cannot_be_decided() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    let overview = admin.get("/api/server/pairing").await.expect(200).json();
    shape::fields(
        &overview,
        shape::PAIRING_OVERVIEW,
        "GET /api/server/pairing",
    );
    shape::each(&overview["requests"], "pairing requests", |request| {
        shape::fields(request, shape::PAIRING_REQUEST, "a pairing request");
    });

    admin
        .post(
            "/api/server/pairing/NOSUCHREQUEST/approve",
            json!({ "code": "123456" }),
        )
        .await
        .expect(404);
    admin
        .post_empty("/api/server/pairing/NOSUCHREQUEST/deny")
        .await
        .expect(404);
    admin
        .post_empty("/api/server/pairing/NOSUCHREQUEST/approve")
        .await
        .expect(400);
}

#[tokio::test]
async fn the_remote_api_needs_a_token_and_ignores_the_ui_session() {
    let Some(main) = Main::get().await else {
        return;
    };
    let anonymous = main.target.anonymous();
    let answer = anonymous.get("/api/remote/devices").await.expect(401);
    assert_eq!(answer.header("www-authenticate").as_deref(), Some("Bearer"));
    assert_eq!(remote_devices(&main.target, "not-a-token").await, 401);
    main.admin.get("/api/remote/devices").await.expect(401);
    if needs_skakel("Skakel's token endpoint") {
        assert_eq!(
            token(&main.target, "client-nobody", "wrong").await,
            Err(401)
        );
    }
}

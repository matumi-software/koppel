//! The guards in front of every route: which port and role serve what, `Ui:AllowFrom`, the Origin
//! and `X-Koppel-UI` checks, the security headers, plain HTTP on the port, `/healthz` and the UI's
//! own files.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use koppel_contract::{Answer, Body, Main, Single, Target, json};
use reqwest::Method;

#[track_caller]
fn security_headers(answer: &Answer, what: &str) {
    for (name, value) in [
        ("x-frame-options", "DENY"),
        ("content-security-policy", "frame-ancestors 'none'"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ] {
        assert_eq!(
            answer.header(name).as_deref(),
            Some(value),
            "{name} on {what}"
        );
    }
}

#[tokio::test]
async fn every_answer_carries_the_security_headers() {
    let Some(main) = Main::get().await else {
        return;
    };
    let anonymous = main.target.anonymous();
    security_headers(&anonymous.get("/healthz").await.expect(200), "/healthz");
    security_headers(&anonymous.get("/").await.expect(200), "/");
    security_headers(
        &anonymous.get("/api/auth/state").await.expect(200),
        "/api/auth/state",
    );
    security_headers(&anonymous.get("/api/settings").await.expect(401), "a 401");
    security_headers(
        &main.admin.get("/api/client/connection").await.expect(404),
        "a 404",
    );
    security_headers(
        &main
            .admin
            .send(Method::PATCH, "/api/settings", Body::Json(json!({})), &[])
            .await
            .expect(403),
        "a 403",
    );
    security_headers(
        &main.admin.get("/api/settings").await.expect(200),
        "/api/settings",
    );
}

#[tokio::test]
async fn a_change_needs_the_ui_header_and_a_read_does_not() {
    let Some(main) = Main::get().await else {
        return;
    };
    main.admin
        .send(
            Method::PATCH,
            "/api/settings",
            Body::Json(json!({ "shareOnStartup": true })),
            &[],
        )
        .await
        .expect(403);
    main.admin
        .send(
            Method::PATCH,
            "/api/settings",
            Body::Json(json!({ "shareOnStartup": true })),
            &[("X-Koppel-UI", "yes")],
        )
        .await
        .expect(403);
    main.admin
        .patch("/api/settings", json!({ "shareOnStartup": true }))
        .await
        .expect(202);
    main.admin
        .send(Method::GET, "/api/settings", Body::None, &[])
        .await
        .expect(200);
    // Before the session check, so a page elsewhere learns nothing.
    main.target
        .anonymous()
        .send(
            Method::POST,
            "/api/auth/login",
            Body::Json(json!({ "password": "x" })),
            &[],
        )
        .await
        .expect(403);
}

#[tokio::test]
async fn a_request_from_another_origin_is_refused_and_the_same_origin_is_not() {
    let Some(main) = Main::get().await else {
        return;
    };
    let own = main.target.base().to_owned();
    let other_port = {
        let (scheme_host, port) = own.rsplit_once(':').expect("a port in the base address");
        let port: u16 = port.parse().expect("a port number");
        format!("{scheme_host}:{}", port.wrapping_add(1))
    };

    let get = |origin: String, extra: Option<(&'static str, &'static str)>| {
        let admin = main.admin.clone();
        async move {
            let mut headers = vec![("Origin", origin.as_str())];
            headers.extend(extra);
            admin
                .send(Method::GET, "/api/settings", Body::None, &headers)
                .await
                .status
        }
    };
    assert_eq!(get("https://elsewhere.example".to_owned(), None).await, 403);
    assert_eq!(get(other_port, None).await, 403);
    assert_eq!(get(own.replace("https://", "http://"), None).await, 403);
    assert_eq!(get("null".to_owned(), None).await, 403);
    assert_eq!(get(own.clone(), None).await, 200);
    // Behind a TLS proxy, such as the Vite dev server, the scheme comes from X-Forwarded-Proto.
    assert_eq!(
        get(
            own.replace("https://", "http://"),
            Some(("X-Forwarded-Proto", "http"))
        )
        .await,
        200
    );

    main.admin
        .send(
            Method::PATCH,
            "/api/settings",
            Body::Json(json!({ "shareOnStartup": true })),
            &[("Origin", own.as_str()), ("X-Koppel-UI", "1")],
        )
        .await
        .expect(202);
    // Anonymous routes too.
    main.target
        .anonymous()
        .send(
            Method::GET,
            "/api/auth/state",
            Body::None,
            &[("Origin", "https://elsewhere.example")],
        )
        .await
        .expect(403);
}

#[tokio::test]
async fn each_role_serves_its_own_surfaces_from_the_next_request() {
    let Some(main) = Main::get().await else {
        return;
    };
    let admin = &main.admin;
    let anonymous = main.target.anonymous();
    // (path, answer as server, as client, as both). /api/remote and /skakel are for other machines:
    // without a token the remote API is 401, and the info endpoint is open.
    let table: &[(&str, u16, u16, u16)] = &[
        ("/api/server/identity", 200, 404, 200),
        ("/api/server/clients", 200, 404, 200),
        ("/api/client/connection", 404, 200, 200),
        ("/api/client/devices", 404, 200, 200),
        ("/api/settings", 200, 200, 200),
        ("/api/system/info", 200, 200, 200),
        ("/api/activity", 200, 200, 200),
    ];
    let network: &[(&str, u16, u16, u16)] = &[
        ("/api/remote/devices", 401, 404, 401),
        ("/skakel/info", 200, 404, 200),
    ];
    for (role, column) in [("server", 0), ("client", 1), ("both", 2), ("server", 0)] {
        admin
            .patch("/api/settings", json!({ "role": role }))
            .await
            .expect(202);
        for (path, server, client, both) in table {
            let want = [*server, *client, *both][column];
            assert_eq!(admin.get(path).await.status, want, "{path} as {role}");
        }
        for (path, server, client, both) in network {
            let want = [*server, *client, *both][column];
            assert_eq!(anonymous.get(path).await.status, want, "{path} as {role}");
        }
        let health = anonymous.get("/healthz").await.expect(200);
        assert_eq!(health.text(), "ok", "/healthz as {role}");
    }
}

#[tokio::test]
async fn the_ui_loads_without_a_session() {
    let Some(main) = Main::get().await else {
        return;
    };
    let anonymous = main.target.anonymous();
    for path in ["/", "/index.html", "/some/spa/route"] {
        let answer = anonymous.get(path).await.expect(200);
        assert_eq!(answer.content_type(), "text/html", "{path}");
    }
}

/// One plain HTTP request on the HTTPS port, and the head of the answer.
fn plain_http(authority: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(authority).expect("the port is open");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .expect("the request is sent");
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer).into_owned()
}

#[tokio::test]
async fn plain_http_on_the_port_is_redirected_to_https_at_the_same_address() {
    let Some(main) = Main::get().await else {
        return;
    };
    let authority = main.target.authority().to_owned();
    let path = "/some/page?x=1";
    let answer = tokio::task::spawn_blocking({
        let authority = authority.clone();
        move || plain_http(&authority, path)
    })
    .await
    .expect("the request ran");
    let status = answer.lines().next().unwrap_or_default();
    assert!(
        status.starts_with("HTTP/1.1 301"),
        "plain HTTP answered {status:?}"
    );
    let location = answer
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("location")
                .then(|| value.trim().to_owned())
        })
        .expect("a Location header");
    assert_eq!(location, format!("https://{authority}{path}"));

    // HTTPS on the same port still answers.
    main.target.anonymous().get("/healthz").await.expect(200);
}

#[tokio::test]
async fn the_ui_turns_away_an_address_allow_from_does_not_take_and_the_network_port_is_its_own() {
    let Some(guarded) = Single::get("KOPPEL_CONTRACT_GUARDED_URL").await else {
        return;
    };
    let Ok(network) = std::env::var("KOPPEL_CONTRACT_GUARDED_NETWORK_URL") else {
        koppel_contract::skipped("KOPPEL_CONTRACT_GUARDED_NETWORK_URL");
        return;
    };
    let ui = guarded.target.anonymous();
    let network = Target::new(&network).anonymous();

    // The UI port: every UI path is refused for this address, and the network surface is not here.
    for path in ["/api/auth/state", "/api/settings", "/healthz", "/"] {
        let answer = ui.get(path).await.expect(403);
        assert!(
            answer.text().contains("Ui:AllowFrom"),
            "{path} says which setting decides: {}",
            answer.text()
        );
        security_headers(&answer, path);
    }
    ui.post(
        "/api/auth/setup",
        json!({ "password": "contract-password-1" }),
    )
    .await
    .expect(403);
    ui.get("/skakel/info").await.expect(404);
    ui.get("/api/remote/devices").await.expect(404);

    // The network port: other machines get the network surface, whatever Ui:AllowFrom says, and nothing of the UI.
    let info = network.get("/skakel/info").await.expect(200).json();
    assert_eq!(info["app"]["id"], json!("koppel"));
    network.get("/api/remote/devices").await.expect(401);
    network.get("/api/auth/state").await.expect(404);
    network.get("/healthz").await.expect(404);
}

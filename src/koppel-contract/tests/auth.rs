//! Signing in to the UI on a target that already has its password: what every endpoint answers
//! without a session, the session cookie, wrong passwords, signing out, changing the password, and the
//! cap on password hashes running at once. The first password, the lockout and the rate limit need
//! targets of their own and are in `first_run.rs`.

use koppel_contract::shape::{self, Kind};
use koppel_contract::{
    Body, EVENTS_PATH, Main, Push, Session, activity_with, json, password, push_kind,
};
use reqwest::Method;

const OTHER_PASSWORD: &str = "contract-password-2";

/// The UI's routes a server serves, each needing a session. The ids in them need not exist: the
/// session check comes before the route.
const SERVER_ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/settings"),
    ("PATCH", "/api/settings"),
    ("GET", "/api/system/info"),
    ("GET", "/api/system/checks"),
    ("GET", "/api/activity"),
    ("POST", "/api/auth/logout"),
    ("POST", "/api/auth/password"),
    ("GET", "/api/server/identity"),
    ("GET", "/api/server/clients"),
    ("POST", "/api/server/clients"),
    ("DELETE", "/api/server/clients/client-0"),
    ("GET", "/api/server/clients/client-0/secret"),
    ("POST", "/api/server/clients/client-0/rotate"),
    ("GET", "/api/server/keys"),
    ("POST", "/api/server/keys/next-certificate"),
    ("DELETE", "/api/server/keys/next-certificate"),
    ("POST", "/api/server/keys/next-certificate/switch"),
    ("GET", "/api/server/pairing"),
    ("POST", "/api/server/pairing/window"),
    ("DELETE", "/api/server/pairing/window"),
    ("POST", "/api/server/pairing/0/approve"),
    ("POST", "/api/server/pairing/0/deny"),
    ("GET", "/api/server/devices"),
    ("PUT", "/api/server/devices/1-1/shared"),
    ("GET", "/api/not-a-route-yet"),
];

/// The UI's routes a client serves.
const CLIENT_ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/client/connection"),
    ("PUT", "/api/client/connection"),
    ("POST", "/api/client/connection/test"),
    ("GET", "/api/client/discovery"),
    ("GET", "/api/client/pairing"),
    ("POST", "/api/client/pairing"),
    ("POST", "/api/client/pairing/confirm"),
    ("DELETE", "/api/client/pairing"),
    ("GET", "/api/client/devices"),
    ("POST", "/api/client/devices/1-1/attach"),
    ("POST", "/api/client/devices/1-1/detach"),
];

async fn call(session: &Session, method: &str, path: &str) -> u16 {
    let method = Method::from_bytes(method.as_bytes()).expect("a method");
    let body = if method == Method::GET {
        Body::None
    } else {
        Body::Json(json!({}))
    };
    session.change(method, path, body).await.status
}

#[tokio::test]
async fn every_ui_route_answers_401_without_a_session_and_the_role_comes_first() {
    let Some(main) = Main::get().await else {
        return;
    };
    let anonymous = main.target.anonymous();

    for (method, path) in SERVER_ROUTES {
        assert_eq!(call(&anonymous, method, path).await, 401, "{method} {path}");
    }
    // A route the role does not have is not found, signed in or not.
    for (method, path) in CLIENT_ROUTES {
        assert_eq!(
            call(&anonymous, method, path).await,
            404,
            "{method} {path} on a server"
        );
    }

    main.admin
        .patch("/api/settings", json!({ "role": "both" }))
        .await
        .expect(202);
    for (method, path) in CLIENT_ROUTES.iter().chain(SERVER_ROUTES) {
        assert_eq!(
            call(&anonymous, method, path).await,
            401,
            "{method} {path} on both"
        );
    }

    // A session cookie that is not one is no session.
    let forged = main.target.with_cookie("not-a-session");
    assert_eq!(forged.get("/api/settings").await.status, 401);
    let empty_401 = anonymous.get("/api/settings").await.expect(401);
    assert!(
        empty_401.header("location").is_none(),
        "an API call is never redirected to a sign-in page"
    );
}

#[tokio::test]
async fn state_and_health_answer_without_a_session() {
    let Some(main) = Main::get().await else {
        return;
    };
    let anonymous = main.target.anonymous();

    let state = anonymous.get("/api/auth/state").await.expect(200).json();
    shape::fields(
        &state,
        &[
            ("signedIn", Kind::Bool),
            ("passwordSet", Kind::Bool),
            ("machineName", Kind::Str),
            ("setupOpen", Kind::Bool),
            ("setupUntil", Kind::OptTime),
        ],
        "GET /api/auth/state",
    );
    assert_eq!(state["signedIn"], json!(false));
    assert_eq!(state["passwordSet"], json!(true));
    assert_eq!(state["setupOpen"], json!(false));
    assert!(
        state["setupUntil"].is_null(),
        "null once there is a password"
    );

    let signed_in = main.admin.get("/api/auth/state").await.expect(200).json();
    assert_eq!(signed_in["signedIn"], json!(true));
    let info = main.admin.get("/api/system/info").await.expect(200).json();
    assert_eq!(state["machineName"], info["name"]);

    let health = anonymous.get("/healthz").await.expect(200);
    assert_eq!(health.text(), "ok");
    assert_eq!(health.content_type(), "text/plain");
}

#[tokio::test]
async fn signing_in_sets_a_strict_http_only_session_cookie() {
    let Some(main) = Main::get().await else {
        return;
    };
    let answer = main
        .target
        .anonymous()
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(204);
    answer.empty();

    let header = answer
        .session_set_cookie()
        .expect("signing in sets koppel.ui");
    let lower = header.to_ascii_lowercase();
    for attribute in ["httponly", "samesite=strict", "secure", "path=/"] {
        assert!(lower.contains(attribute), "{attribute} in {header}");
    }
    assert!(
        !lower.contains("expires=") && !lower.contains("max-age="),
        "a session cookie, the 12 hours are in the ticket: {header}"
    );

    let session = main
        .target
        .with_cookie(&answer.session_cookie().expect("a value"));
    session.get("/api/settings").await.expect(200);
}

#[tokio::test]
async fn a_wrong_password_is_refused_without_a_cookie() {
    let Some(main) = Main::get().await else {
        return;
    };
    let anonymous = main.target.anonymous();

    let wrong = anonymous
        .post("/api/auth/login", json!({ "password": OTHER_PASSWORD }))
        .await
        .expect(401);
    assert!(!wrong.error().is_empty());
    assert!(wrong.session_set_cookie().is_none());

    let missing = anonymous
        .post("/api/auth/login", json!({}))
        .await
        .expect(401);
    assert!(!missing.error().is_empty());

    // The setup is over: a second one changes nothing.
    let again = anonymous
        .post("/api/auth/setup", json!({ "password": OTHER_PASSWORD }))
        .await
        .expect(409);
    assert!(!again.error().is_empty());

    // A right password clears the wrong ones, so they never add up to a lockout across tests.
    main.target.sign_in().await;
}

#[tokio::test]
async fn signing_out_ends_the_session_and_a_copy_of_its_cookie() {
    let Some(main) = Main::get().await else {
        return;
    };
    let session = main.target.sign_in().await;
    let copy = main.target.with_cookie(session.cookie().expect("a cookie"));

    let out = session.post_empty("/api/auth/logout").await.expect(204);
    out.empty();
    let cleared = out
        .session_set_cookie()
        .expect("signing out clears the cookie");
    assert!(
        cleared.starts_with("koppel.ui=;"),
        "an empty value: {cleared}"
    );
    assert!(
        cleared
            .to_ascii_lowercase()
            .contains("expires=thu, 01 jan 1970"),
        "expired: {cleared}"
    );

    copy.get("/api/settings").await.expect(401);
    let state = copy.get("/api/auth/state").await.expect(200).json();
    assert_eq!(state["signedIn"], json!(false));
    copy.post_empty("/api/auth/logout").await.expect(401);

    // Other sessions carry on.
    main.admin.get("/api/settings").await.expect(200);
}

#[tokio::test]
async fn changing_the_password_signs_out_other_sessions_and_keeps_this_one() {
    let Some(main) = Main::get().await else {
        return;
    };
    let this = main.admin.clone();
    let other = main.target.sign_in().await;

    let wrong = this
        .post(
            "/api/auth/password",
            json!({ "current": OTHER_PASSWORD, "next": OTHER_PASSWORD }),
        )
        .await
        .expect(400);
    assert!(!wrong.error().is_empty());
    let weak = this
        .post(
            "/api/auth/password",
            json!({ "current": password(), "next": "short" }),
        )
        .await
        .expect(400);
    assert!(!weak.error().is_empty());
    let long = this
        .post(
            "/api/auth/password",
            json!({ "current": password(), "next": "x".repeat(129) }),
        )
        .await
        .expect(400);
    assert!(!long.error().is_empty());
    this.post("/api/auth/password", json!({})).await.expect(400);
    other.get("/api/settings").await.expect(200);

    let changed = this
        .post(
            "/api/auth/password",
            json!({ "current": password(), "next": OTHER_PASSWORD }),
        )
        .await
        .expect(204);
    changed.empty();
    let renewed = main.target.with_cookie(
        &changed
            .session_cookie()
            .expect("this session gets a cookie for the new password"),
    );

    other.get("/api/settings").await.expect(401);
    this.get("/api/settings").await.expect(401);
    renewed.get("/api/settings").await.expect(200);

    let anonymous = main.target.anonymous();
    anonymous
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(401);
    anonymous
        .post("/api/auth/login", json!({ "password": OTHER_PASSWORD }))
        .await
        .expect(204);

    let entries = activity_with(&renewed, "passwordChanged").await;
    let entry = entries.first().expect("a passwordChanged entry");
    shape::fields(entry, shape::ACTIVITY, "passwordChanged");
    assert_eq!(entry["outcome"], json!("succeeded"));
    assert_eq!(entry["entity"]["type"], json!("machine"));

    // Back to the run's password for the tests after this one.
    renewed
        .post(
            "/api/auth/password",
            json!({ "current": OTHER_PASSWORD, "next": password() }),
        )
        .await
        .expect(204);
}

#[tokio::test]
async fn password_hashes_beyond_two_at_once_are_turned_away_at_once() {
    let Some(main) = Main::get().await else {
        return;
    };
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let anonymous = main.target.anonymous();
        tasks.spawn(async move {
            anonymous
                .post("/api/auth/login", json!({ "password": password() }))
                .await
        });
    }

    let mut busy = 0;
    while let Some(answer) = tasks.join_next().await {
        let answer = answer.expect("the task ran");
        match answer.status {
            204 => {}
            429 => {
                busy += 1;
                assert_eq!(answer.header("retry-after").as_deref(), Some("1"));
                assert!(!answer.error().is_empty());
            }
            other => panic!("a login answered {other}: {}", answer.text()),
        }
    }
    assert!(
        busy > 0,
        "twelve logins at once got past two hash slots without one turned away"
    );
}

#[tokio::test]
async fn the_live_push_needs_a_session() {
    let Some(main) = Main::get().await else {
        return;
    };
    match push_kind() {
        Push::SignalR => {
            let path = "/api/hubs/ui/negotiate?negotiateVersion=1";
            let anonymous = main.target.anonymous();
            anonymous
                .send(Method::POST, path, Body::None, &[])
                .await
                .expect(401);
            // The hub is exempt from X-Koppel-UI, since an upgrade cannot carry it, but not from the Origin check.
            main.admin
                .send(Method::POST, path, Body::None, &[])
                .await
                .expect(200);
            main.admin
                .send(
                    Method::POST,
                    path,
                    Body::None,
                    &[("Origin", "https://elsewhere.example")],
                )
                .await
                .expect(403);
        }
        Push::Sse => {
            let anonymous = main.target.anonymous();
            anonymous.get(EVENTS_PATH).await.expect(401);
            // A GET, so no X-Koppel-UI, which an EventSource cannot send, but the Origin check holds.
            let events = main.admin.events().await;
            assert_eq!(events.status, 200);
            assert!(
                events.content_type.starts_with("text/event-stream"),
                "{}",
                events.content_type
            );
            main.admin
                .send(
                    Method::GET,
                    EVENTS_PATH,
                    Body::None,
                    &[("Origin", "https://elsewhere.example")],
                )
                .await
                .expect(403);
        }
    }
}

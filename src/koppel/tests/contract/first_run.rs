//! The first password, the setup window closing, the lockout after wrong passwords and the rate limit
//! on sign-in. Each needs a target in a state no other test may disturb, so each has its own.

use std::time::Duration;

use crate::support::shape::{self, Kind};
use crate::support::{Body, Options, Session, Single, activity_with, eventually, json, password};
use reqwest::Method;

const OTHER_PASSWORD: &str = "contract-password-2";

/// Koppel's default for `RateLimits:AuthPerMinute`, which the limits instance runs with.
const AUTH_PER_MINUTE: usize = 30;

/// Wrong passwords a caller gets before the lock.
const FREE_FAILURES: usize = 5;

fn state_fields() -> &'static [(&'static str, Kind)] {
    &[
        ("signedIn", Kind::Bool),
        ("passwordSet", Kind::Bool),
        ("machineName", Kind::Str),
        ("setupOpen", Kind::Bool),
        ("setupUntil", Kind::OptTime),
    ]
}

async fn state(session: &Session) -> serde_json::Value {
    let state = session.get("/api/auth/state").await.expect(200).json();
    shape::fields(&state, state_fields(), "GET /api/auth/state");
    state
}

#[tokio::test]
async fn the_first_password_is_set_once_then_wrong_passwords_lock_the_caller_out() {
    let fresh = Single::start(Options::default()).await;
    let anonymous = fresh.target.anonymous();

    // Before the password: setup is open until a time, and there is nothing to sign in to.
    let before = state(&anonymous).await;
    assert_eq!(
        before["passwordSet"],
        json!(false),
        "the fresh target must start without a password"
    );
    assert_eq!(before["signedIn"], json!(false));
    assert_eq!(before["setupOpen"], json!(true));
    assert!(before["setupUntil"].is_string(), "when setup closes");
    let login = anonymous
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(409);
    assert!(!login.error().is_empty());
    anonymous.post_empty("/api/auth/logout").await.expect(401);
    anonymous
        .post("/api/auth/password", json!({ "current": "x", "next": "y" }))
        .await
        .expect(401);

    // A password that is not allowed leaves setup open.
    for refused in [
        json!({ "password": "short" }),
        json!({ "password": "x".repeat(129) }),
        json!({}),
    ] {
        let answer = anonymous.post("/api/auth/setup", refused).await.expect(400);
        assert!(!answer.error().is_empty());
    }
    anonymous
        .change(
            Method::POST,
            "/api/auth/setup",
            Body::Raw("application/json", "not json".to_owned()),
        )
        .await
        .expect(400);
    // Anonymous or not, a change needs the UI's header.
    anonymous
        .send(
            Method::POST,
            "/api/auth/setup",
            Body::Json(json!({ "password": password() })),
            &[],
        )
        .await
        .expect(403);
    assert_eq!(state(&anonymous).await["setupOpen"], json!(true));

    // Setup takes the password and signs that browser in.
    let setup = anonymous
        .post("/api/auth/setup", json!({ "password": password() }))
        .await
        .expect(204);
    setup.empty();
    let session = fresh
        .target
        .with_cookie(&setup.session_cookie().expect("setup signs in"));
    session.get("/api/settings").await.expect(200);
    assert_eq!(state(&session).await["signedIn"], json!(true));

    let after = state(&anonymous).await;
    assert_eq!(after["passwordSet"], json!(true));
    assert_eq!(after["setupOpen"], json!(false));
    assert!(after["setupUntil"].is_null());

    let again = anonymous
        .post("/api/auth/setup", json!({ "password": OTHER_PASSWORD }))
        .await
        .expect(409);
    assert!(!again.error().is_empty());
    anonymous
        .post("/api/auth/login", json!({ "password": OTHER_PASSWORD }))
        .await
        .expect(401);

    // The lockout: a right password clears the count, then five wrong ones lock the caller.
    anonymous
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(204);
    for i in 0..FREE_FAILURES {
        anonymous
            .post(
                "/api/auth/login",
                json!({ "password": format!("wrong-{i}") }),
            )
            .await
            .expect(401);
    }
    let locked = anonymous
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(429);
    let retry: u64 = locked
        .header("retry-after")
        .and_then(|s| s.parse().ok())
        .expect("Retry-After in seconds");
    assert!(
        (31..=60).contains(&retry),
        "a lock of a minute, Retry-After {retry}"
    );
    assert!(locked.error().contains("Wait"), "{}", locked.text());
    assert!(locked.session_set_cookie().is_none());

    // The password change counts against the same caller.
    session
        .post(
            "/api/auth/password",
            json!({ "current": password(), "next": OTHER_PASSWORD }),
        )
        .await
        .expect(429);

    let failures = activity_with(&session, "signInFailed").await;
    assert_eq!(
        failures.len(),
        1,
        "one entry when the lock engages, not one per wrong try: {failures:?}"
    );
    shape::fields(&failures[0], shape::ACTIVITY, "signInFailed");
    assert_eq!(failures[0]["outcome"], json!("failed"));
}

#[tokio::test]
async fn setup_is_refused_once_its_window_has_closed() {
    let expired =
        Single::start(Options::default().setting("KOPPEL_UI_SETUP_WINDOW_MINUTES", "1")).await;
    let anonymous = expired.target.anonymous();
    assert_eq!(
        state(&anonymous).await["passwordSet"],
        json!(false),
        "the expired target must start without a password"
    );

    // Started with a window of one minute.
    let closed = eventually(
        "the setup window to close",
        Duration::from_secs(120),
        || async {
            let state = state(&anonymous).await;
            (state["setupOpen"] == json!(false)).then_some(state)
        },
    )
    .await;
    assert_eq!(closed["passwordSet"], json!(false));
    assert!(
        closed["setupUntil"].is_string(),
        "when setup closed, while there is no password"
    );

    let refused = anonymous
        .post("/api/auth/setup", json!({ "password": password() }))
        .await
        .expect(403);
    assert!(!refused.error().is_empty());
    // The window is checked before the password.
    anonymous
        .post("/api/auth/setup", json!({ "password": "short" }))
        .await
        .expect(403);
    let login = anonymous
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(409);
    assert!(!login.error().is_empty());
}

#[tokio::test]
async fn setup_login_and_password_change_share_a_rate_limit_per_address() {
    // The default of 30 a minute, which the other instances raise.
    let limits =
        Single::start(Options::default().setting("KOPPEL_RATE_AUTH_PER_MINUTE", "30")).await;
    let anonymous = limits.target.anonymous();

    for i in 0..AUTH_PER_MINUTE {
        let answer = anonymous
            .post("/api/auth/setup", json!({ "password": "short" }))
            .await;
        assert_eq!(
            answer.status,
            400,
            "request {} of {AUTH_PER_MINUTE}: {}",
            i + 1,
            answer.text()
        );
    }
    anonymous
        .post("/api/auth/setup", json!({ "password": "short" }))
        .await
        .expect(429);
    anonymous
        .post("/api/auth/login", json!({ "password": password() }))
        .await
        .expect(429);
    // The UI polls the state, which is not limited.
    anonymous.get("/api/auth/state").await.expect(200);
}

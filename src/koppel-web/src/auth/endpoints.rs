//! `/api/auth/*`. State, setup and login are anonymous; logout and the password change need the
//! session. Success is 204 with a cookie, a wrong password 401 or 400 and setup after its window, or without the setup token where one is
//! required, 403,
//! each with `{ error }`, and a caller who tried too often, or who arrives while the server is busy
//! hashing other passwords, gets 429 with `Retry-After`.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Extension;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, RETRY_AFTER, SET_COOKIE};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::session::clear_cookie;
use super::throttle::Reservation;
use super::{SetupOutcome, SignedIn, password};
use crate::access::{address_key, throttle_key};
use crate::activity::{ActivityOutcome, ActivityReason, EntityKind, EntityRef};
use crate::json::{self, JsonBody};
use crate::state::AuthApi;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthState {
    signed_in: bool,
    password_set: bool,
    machine_name: String,
    setup_open: bool,
    /// Setup asks for the one-time token the log shows.
    setup_token_required: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    setup_until: Option<OffsetDateTime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PasswordRequest {
    #[serde(default)]
    password: Option<String>,
    /// Setup only: the token the log shows, where one is required.
    #[serde(default)]
    setup_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangeRequest {
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    next: Option<String>,
}

pub(crate) async fn state(
    State(app): State<AuthApi>,
    session: Option<Extension<SignedIn>>,
) -> Response {
    json::ok(&AuthState {
        signed_in: session.is_some(),
        password_set: app.auth.has_password(),
        machine_name: app.host.name.clone(),
        setup_open: app.auth.setup_open(),
        setup_token_required: app.auth.setup_token_required(),
        setup_until: app.auth.setup_until(),
    })
}

/// Sets the first password, for whoever asks first while setup is open, or whoever holds the token
/// where one is required (Linux and Docker, with the UI open beyond this machine). The token is 128
/// random bits, so there is nothing to guess and no caller is locked out; the rate limit and the hash
/// slots hold back a flood.
pub(crate) async fn setup(
    State(app): State<AuthApi>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    JsonBody(request): JsonBody<PasswordRequest>,
) -> Response {
    let auth = &app.auth;
    if auth.has_password() {
        return already_set();
    }
    if !auth.setup_open() {
        return json::error(StatusCode::FORBIDDEN, &auth.closed_reason());
    }
    if !auth.setup_token_is(request.setup_token.as_deref()) {
        tracing::warn!(
            "Setup from {} was refused: the setup token is missing or wrong",
            address_key(remote.ip())
        );
        return json::error(
            StatusCode::FORBIDDEN,
            "The setup token is missing or not right. Copy it from Koppel's log.",
        );
    }
    if let Some(problem) = password::validate(request.password.as_deref()) {
        return json::error(StatusCode::BAD_REQUEST, &problem);
    }
    let Some(_slot) = auth.hash_slot() else {
        return busy();
    };
    match auth
        .set_first_password(request.password.unwrap_or_default())
        .await
    {
        Ok(SetupOutcome::Done) => {
            tracing::warn!(
                "The first UI password was set from {}",
                address_key(remote.ip())
            );
            signed_in(auth.sign_in())
        }
        Ok(SetupOutcome::AlreadySet) => already_set(),
        Ok(SetupOutcome::Closed) => json::error(StatusCode::FORBIDDEN, &auth.closed_reason()),
        Err(e) => json::failed("Setting the first password", e),
    }
}

pub(crate) async fn login(
    State(app): State<AuthApi>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    JsonBody(request): JsonBody<PasswordRequest>,
) -> Response {
    let auth = &app.auth;
    if !auth.has_password() {
        return json::error(
            StatusCode::CONFLICT,
            "No password is set yet. Open the UI to set one.",
        );
    }
    let caller = address_key(remote.ip());
    let key = format!("login:{}", throttle_key(remote.ip()));
    // The lock is read before a slot is taken, so a locked caller cannot hold one. Then a slot,
    // answering at once when none is free, and the try is booked before the password is looked at,
    // so parallel guesses cannot all get through ahead of the lock.
    if let Some(wait) = auth.precheck(&key) {
        return locked(wait);
    }
    let Some(_slot) = auth.hash_slot() else {
        return busy();
    };
    let attempt = auth.reserve(&key);
    if !attempt.granted {
        return locked(attempt.lock);
    }
    if auth.verify(request.password).await {
        auth.clear(&key);
        return signed_in(auth.sign_in());
    }
    fail(&app, attempt, &caller, "Sign-in");
    json::error(StatusCode::UNAUTHORIZED, "That password is not right.")
}

/// Ends the session, so a copy of its cookie is refused too and its live push closes, and removes
/// the cookie from this browser.
pub(crate) async fn logout(
    State(app): State<AuthApi>,
    Extension(session): Extension<SignedIn>,
) -> Response {
    app.auth.sign_out(&session.id);
    with_cookie(StatusCode::NO_CONTENT.into_response(), &clear_cookie())
}

/// Completes on this answer: unlike a setting there is no value to push back, and the new password
/// is never shown again. The confirm rule is for state the server owns and echoes.
pub(crate) async fn change_password(
    State(app): State<AuthApi>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Extension(session): Extension<SignedIn>,
    JsonBody(request): JsonBody<ChangeRequest>,
) -> Response {
    let auth = &app.auth;
    let caller = address_key(remote.ip());
    let key = format!("login:{}", throttle_key(remote.ip()));
    if let Some(wait) = auth.precheck(&key) {
        return locked(wait);
    }
    let Some(_slot) = auth.hash_slot() else {
        return busy();
    };
    let attempt = auth.reserve(&key);
    if !attempt.granted {
        return locked(attempt.lock);
    }
    if !auth.verify(request.current).await {
        fail(&app, attempt, &caller, "Password change");
        return json::error(
            StatusCode::BAD_REQUEST,
            "The current password is not right.",
        );
    }
    auth.clear(&key);
    if let Some(problem) = password::validate(request.next.as_deref()) {
        return json::error(StatusCode::BAD_REQUEST, &problem);
    }
    if let Err(e) = auth
        .change_password(request.next.unwrap_or_default(), &session.id)
        .await
    {
        return json::failed("Changing the password", e);
    }
    tracing::info!("The UI password was changed from {caller}; other sessions are signed out");
    app.activity.record(
        machine(&app),
        ActivityReason::PasswordChanged,
        ActivityOutcome::Succeeded,
        "The UI password was changed and other sessions were signed out",
        None,
    );
    // This session stays as it is: the same cookie goes on working.
    StatusCode::NO_CONTENT.into_response()
}

fn machine(app: &AuthApi) -> EntityRef {
    EntityRef::new("ui", app.host.name.clone(), EntityKind::Machine)
}

// Every failure goes to the log. The activity log keeps 500 entries, so it only hears of the one
// that engages a lock: a burst of guesses is one entry, not a flood.
fn fail(app: &AuthApi, attempt: Reservation, caller: &str, what: &str) {
    tracing::warn!(
        "{what} failed from {caller}: wrong password ({} recent tries)",
        attempt.failures
    );
    if !attempt.lock.is_zero() {
        app.activity.record(
            machine(app),
            ActivityReason::SignInFailed,
            ActivityOutcome::Failed,
            format!(
                "{what} failed {} times from {caller}, which is locked out for {} seconds",
                attempt.failures,
                seconds(attempt.lock)
            ),
            Some("Wrong password.".to_owned()),
        );
    }
}

fn seconds(wait: Duration) -> u64 {
    wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

fn signed_in(cookie: String) -> Response {
    with_cookie(StatusCode::NO_CONTENT.into_response(), &cookie)
}

pub(crate) fn with_cookie(mut response: Response, cookie: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}

fn already_set() -> Response {
    json::error(StatusCode::CONFLICT, "A password is already set.")
}

fn busy() -> Response {
    let mut response = json::error(
        StatusCode::TOO_MANY_REQUESTS,
        "The server is busy checking another password. Try again in a moment.",
    );
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

fn locked(wait: Duration) -> Response {
    let seconds = seconds(wait);
    let mut response = json::error(
        StatusCode::TOO_MANY_REQUESTS,
        &format!("Too many tries. Wait {} and try again.", wait_text(seconds)),
    );
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from(seconds));
    response
}

fn wait_text(seconds: u64) -> String {
    if seconds >= 90 {
        format!("{} minutes", seconds.div_ceil(60))
    } else if seconds == 1 {
        "1 second".to_owned()
    } else {
        format!("{seconds} seconds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_are_worded_as_0_1_words_them() {
        assert_eq!(wait_text(1), "1 second");
        assert_eq!(wait_text(60), "60 seconds");
        assert_eq!(wait_text(90), "2 minutes");
        assert_eq!(wait_text(3600), "60 minutes");
        assert_eq!(seconds(Duration::from_millis(59_001)), 60);
    }
}

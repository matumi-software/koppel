//! `/api/auth/*`. State, setup and login are anonymous; logout and the password change need the
//! session. Success is 204 with a cookie, a wrong password 401 or 400 and setup after its window 403,
//! each with `{ error }`, and a caller who tried too often, or who arrives while the server is busy
//! hashing other passwords, gets 429 with `Retry-After`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, RETRY_AFTER, SET_COOKIE};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::throttle::Reservation;
use super::{SetupOutcome, SignedIn, password, ticket};
use crate::access::address_key;
use crate::activity::{ActivityOutcome, ActivityReason, EntityRef};
use crate::app::App;
use crate::json::{self, JsonBody};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthState {
    signed_in: bool,
    password_set: bool,
    machine_name: String,
    setup_open: bool,
    #[serde(with = "koppel_config::json::date_time_offset::option")]
    setup_until: Option<OffsetDateTime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PasswordRequest {
    #[serde(default)]
    password: Option<String>,
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
    State(app): State<Arc<App>>,
    session: Option<Extension<SignedIn>>,
) -> Response {
    json::ok(&AuthState {
        signed_in: session.is_some(),
        password_set: app.auth.has_password(),
        machine_name: app.host.name.clone(),
        setup_open: app.auth.setup_open(),
        setup_until: app.auth.setup_until(),
    })
}

/// Sets the first password, for whoever asks first while setup is open. There is no secret to
/// guess, so no caller is locked out; the rate limit and the hash slots hold back a flood.
pub(crate) async fn setup(
    State(app): State<Arc<App>>,
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
            signed_in(auth.cookie(None, None))
        }
        Ok(SetupOutcome::AlreadySet) => already_set(),
        Ok(SetupOutcome::Closed) => json::error(StatusCode::FORBIDDEN, &auth.closed_reason()),
        Err(e) => json::failed("Setting the first password", e),
    }
}

pub(crate) async fn login(
    State(app): State<Arc<App>>,
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
    // A slot first, answering at once when none is free, then the try is booked before the
    // password is looked at, so parallel guesses cannot all get through ahead of the lock.
    let Some(_slot) = auth.hash_slot() else {
        return busy();
    };
    let caller = address_key(remote.ip());
    let key = format!("login:{caller}");
    let attempt = auth.reserve(&key);
    if !attempt.granted {
        return locked(attempt.lock);
    }
    if auth.verify(request.password).await {
        auth.clear(&key);
        return signed_in(auth.cookie(None, None));
    }
    fail(&app, attempt, &caller, "Sign-in");
    json::error(StatusCode::UNAUTHORIZED, "That password is not right.")
}

/// Removes the cookie from this browser and revokes the session id, so a copy of the cookie is
/// refused too, and closes the session's live push.
pub(crate) async fn logout(
    State(app): State<Arc<App>>,
    Extension(session): Extension<SignedIn>,
) -> Response {
    if let Err(e) = app.auth.revoke(&session.id, session.issued).await {
        return json::failed("Signing out", e);
    }
    with_cookie(
        StatusCode::NO_CONTENT.into_response(),
        &ticket::clear_cookie(),
    )
}

/// Completes on this answer: unlike a setting there is no value to push back, and the new password
/// is never shown again. The confirm rule is for state the server owns and echoes.
pub(crate) async fn change_password(
    State(app): State<Arc<App>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Extension(session): Extension<SignedIn>,
    JsonBody(request): JsonBody<ChangeRequest>,
) -> Response {
    let auth = &app.auth;
    let Some(_slot) = auth.hash_slot() else {
        return busy();
    };
    let caller = address_key(remote.ip());
    let key = format!("login:{caller}");
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
    // The change moved the stamp; this session gets a cookie for the new one.
    signed_in(auth.cookie(Some(&session.id), Some(session.issued)))
}

fn machine(app: &App) -> EntityRef {
    EntityRef::new("ui", app.host.name.clone(), "machine")
}

// Every failure goes to the log. The activity log keeps 500 entries, so it only hears of the one
// that engages a lock: a burst of guesses is one entry, not a flood.
fn fail(app: &App, attempt: Reservation, caller: &str, what: &str) {
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

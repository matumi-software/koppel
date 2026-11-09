//! The server role's endpoints: its identity, registered clients, certificate and signing key,
//! pairing requests and USB devices. Koppel's own `/api/remote/devices` for other machines is
//! Skakel's server's (`roles/skakel/remote.rs`).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::activity::{ActivityOutcome, ActivityReason, EntityRef};
use crate::app::App;
use crate::json::{self, JsonBody};
use crate::roles::{Approval, RegisterError, is_bus_id};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerIdentity<'a> {
    name: &'a str,
    fingerprint: String,
    https_port: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateClient {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Approve {
    #[serde(default)]
    code: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Share {
    #[serde(default)]
    shared: bool,
}

fn client_entity(id: &str, name: &str) -> EntityRef {
    EntityRef::new(id, name, "client")
}

fn machine(app: &App) -> EntityRef {
    EntityRef::new("machine", app.host.name.clone(), "machine")
}

fn secret(secret: &str) -> Response {
    json::ok(&json!({ "clientSecret": secret }))
}

pub(crate) async fn identity(State(app): State<Arc<App>>) -> Response {
    json::ok(&ServerIdentity {
        name: &app.host.name,
        fingerprint: app.server.fingerprint(),
        https_port: app.host.network_port,
    })
}

pub(crate) async fn clients(State(app): State<Arc<App>>) -> Response {
    json::ok(&app.server.clients())
}

fn name_problem(message: &str) -> Response {
    json::problem(&[("name", message)])
}

/// Answers 201 with the secret; the list follows as `ClientsChanged` once `secrets.json` is
/// written. The secret is kept, encrypted, so it can be copied again later.
pub(crate) async fn create_client(
    State(app): State<Arc<App>>,
    JsonBody(request): JsonBody<CreateClient>,
) -> Response {
    let name = request.name.unwrap_or_default();
    let name = name.trim();
    if name.is_empty() || name.encode_utf16().count() > 80 {
        return name_problem(
            "Give the client a name of up to 80 characters, such as the machine it runs on.",
        );
    }
    let client = match app.server.register(name).await {
        Ok(client) => client,
        Err(RegisterError::EmptyName) => {
            return name_problem(
                "Nothing is left of the name once invisible characters are taken out. Type a name such as the machine it runs on.",
            );
        }
        Err(RegisterError::Failed(e)) => return json::failed("Adding a client", e),
    };
    app.activity.record(
        client_entity(&client.client_id, &client.name),
        ActivityReason::ClientAdded,
        ActivityOutcome::Succeeded,
        format!("Client {} was added", client.name),
        None,
    );
    json::created(
        &format!("/api/server/clients/{}", client.client_id),
        &client,
    )
}

pub(crate) async fn revoke_client(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let name = app.server.name_of(&id).unwrap_or_else(|| id.clone());
    match app.server.revoke(&id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return json::failed("Revoking a client", e),
    }
    app.activity.record(
        client_entity(&id, &name),
        ActivityReason::ClientRevoked,
        ActivityOutcome::Succeeded,
        format!("Client {name} was revoked"),
        None,
    );
    json::accepted()
}

/// One client's secret, fetched only when someone reveals or copies it.
pub(crate) async fn client_secret(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    match app.server.secret_of(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(Some(value)) => secret(&value),
        Some(None) => json::json_answer(
            StatusCode::CONFLICT,
            &json!({ "error": "This client was added before secrets were kept. Rotate the secret to get one you can copy." }),
        ),
    }
}

/// A new secret, given once in the answer; the list follows as `ClientsChanged`.
pub(crate) async fn rotate_secret(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let value = match app.server.rotate_secret(&id).await {
        Ok(Some(value)) => value,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return json::failed("Rotating a client's secret", e),
    };
    let name = app.server.name_of(&id).unwrap_or_else(|| id.clone());
    app.activity.record(
        client_entity(&id, &name),
        ActivityReason::SecretRotated,
        ActivityOutcome::Succeeded,
        format!("The secret for {name} was rotated"),
        None,
    );
    secret(&value)
}

pub(crate) async fn keys(State(app): State<Arc<App>>) -> Response {
    json::ok(&app.server.keys())
}

pub(crate) async fn rotate_signing_key(State(app): State<Arc<App>>) -> Response {
    let id = match app.server.rotate_signing_key().await {
        Ok(id) => id,
        Err(e) => return json::failed("Rotating the signing key", e),
    };
    app.activity.record(
        machine(&app),
        ActivityReason::SigningKeyRotated,
        ActivityOutcome::Succeeded,
        format!("The token signing key was rotated, to {id}"),
        None,
    );
    json::accepted()
}

fn refused(reason: &str) -> Response {
    json::problem(&[("certificate", reason)])
}

/// The first four bytes, as the UI shows them, so a line in the log can be matched to the
/// fingerprint.
fn short(fingerprint: &str) -> String {
    format!("{}…", fingerprint.get(..11).unwrap_or(fingerprint))
}

pub(crate) async fn prepare_certificate(State(app): State<Arc<App>>) -> Response {
    let next = match app.server.prepare_next_certificate().await {
        Ok(Some(next)) => next,
        Ok(None) => {
            return refused(
                "A new certificate is already prepared. Switch to it or cancel it first.",
            );
        }
        Err(e) => return json::failed("Preparing a certificate", e),
    };
    app.activity.record(
        machine(&app),
        ActivityReason::CertificatePrepared,
        ActivityOutcome::Succeeded,
        format!(
            "A new certificate was prepared, fingerprint {}",
            short(&next)
        ),
        None,
    );
    json::accepted()
}

pub(crate) async fn cancel_certificate(State(app): State<Arc<App>>) -> Response {
    let next = match app.server.cancel_next_certificate().await {
        Ok(Some(next)) => next,
        Ok(None) => return refused("There is no new certificate to cancel."),
        Err(e) => return json::failed("Cancelling a certificate", e),
    };
    app.activity.record(
        machine(&app),
        ActivityReason::CertificateCancelled,
        ActivityOutcome::Succeeded,
        format!("The new certificate {} was cancelled", short(&next)),
        None,
    );
    json::accepted()
}

pub(crate) async fn switch_certificate(State(app): State<Arc<App>>) -> Response {
    let done = match app.server.switch_certificate().await {
        Ok(Some(done)) => done,
        Ok(None) => {
            return refused("There is no new certificate to switch to. Prepare one first.");
        }
        Err(e) => return json::failed("Switching the certificate", e),
    };
    let mut summary = format!(
        "The server switched to its new certificate, fingerprint {}",
        short(&done.to)
    );
    if !done.not_received.is_empty() {
        summary.push_str(&format!(
            ". Not received by {}",
            done.not_received.join(", ")
        ));
    }
    app.activity.record(
        machine(&app),
        ActivityReason::CertificateSwitched,
        ActivityOutcome::Succeeded,
        summary,
        None,
    );
    json::accepted()
}

pub(crate) async fn pairing(State(app): State<Arc<App>>) -> Response {
    json::ok(&app.server.pairing())
}

/// The right code answers 202; the request leaving `PairingRequestsChanged` and the new client in
/// `ClientsChanged` confirm it. A wrong code is a validation problem at once.
pub(crate) async fn approve(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<Approve>,
) -> Response {
    match app
        .server
        .approve(&id, request.code.as_deref().unwrap_or_default())
        .await
    {
        Approval::Approved => json::accepted(),
        Approval::WrongCode => json::problem(&[(
            "code",
            "That is not the code the client shows. Check it on the client and type it again.",
        )]),
        Approval::NotFound => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) async fn deny(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if app.server.deny(&id).await {
        json::accepted()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

pub(crate) async fn devices(State(app): State<Arc<App>>) -> Response {
    json::ok(&app.usb_server.current())
}

/// Applies the choice, which is saved in `devices.json` once it took effect, and answers 202; the
/// outcome reaches the UI as `DevicesChanged`.
pub(crate) async fn share(
    State(app): State<Arc<App>>,
    Path(bus_id): Path<String>,
    JsonBody(request): JsonBody<Share>,
) -> Response {
    if !is_bus_id(&bus_id) {
        return json::problem(&[("busId", "That is not a USB bus ID, such as 1-1.4.")]);
    }
    if let Some(reason) = app.usb_server.current().unavailable {
        return json::problem(&[("device", &reason)]);
    }
    app.usb_server.set_shared(&bus_id, request.shared).await;
    json::accepted()
}

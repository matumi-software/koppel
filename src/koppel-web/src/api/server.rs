//! The server role's endpoints: its identity, registered clients, certificate, the pairing window,
//! pairing requests and USB devices. Koppel's own `/api/remote/devices` for other machines is
//! Skakel's server's (`roles/skakel/remote.rs`).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use koppel_usb::{BusId, ServerDevices};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{ApiError, ApiResult, Created, accepted};
use crate::activity::{ActivityOutcome, ActivityReason, EntityKind, EntityRef};
use crate::json::JsonBody;
use crate::roles::{Approval, ClientView, KeysView, NewClient, PairingOverview, RegisterError};
use crate::state::ServerApi;

#[derive(Serialize, TS)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerIdentity {
    name: String,
    fingerprint: String,
    https_port: u16,
}

/// One client's secret.
#[derive(Serialize, TS)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientSecret {
    client_secret: String,
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct CreateClient {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct Approve {
    #[serde(default)]
    code: Option<String>,
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Share {
    #[serde(default)]
    shared: bool,
}

fn client_entity(id: &str, name: &str) -> EntityRef {
    EntityRef::new(id, name, EntityKind::Client)
}

fn machine(app: &ServerApi) -> EntityRef {
    EntityRef::new("machine", app.host.name.clone(), EntityKind::Machine)
}

fn secret(secret: &str) -> Json<ClientSecret> {
    Json(ClientSecret {
        client_secret: secret.to_owned(),
    })
}

pub(crate) async fn identity(State(app): State<ServerApi>) -> Json<ServerIdentity> {
    Json(ServerIdentity {
        name: app.host.name.clone(),
        fingerprint: app.server.fingerprint(),
        https_port: app.host.network_port,
    })
}

pub(crate) async fn clients(State(app): State<ServerApi>) -> Json<Vec<ClientView>> {
    Json(app.server.clients())
}

fn name_problem(message: &str) -> ApiError {
    ApiError::invalid(&[("name", message)])
}

/// Answers 201 with the secret; the list follows as `ClientsChanged` once `secrets.json` is
/// written. The secret is kept, encrypted, so it can be copied again later.
pub(crate) async fn create_client(
    State(app): State<ServerApi>,
    JsonBody(request): JsonBody<CreateClient>,
) -> ApiResult<Created<NewClient>> {
    let name = request.name.unwrap_or_default();
    let name = name.trim();
    if name.is_empty() || name.encode_utf16().count() > 80 {
        return Err(name_problem(
            "Give the client a name of up to 80 characters, such as the machine it runs on.",
        ));
    }
    let client = match app.server.register(name).await {
        Ok(client) => client,
        Err(RegisterError::EmptyName) => {
            return Err(name_problem(
                "Nothing is left of the name once invisible characters are taken out. Type a name such as the machine it runs on.",
            ));
        }
        Err(RegisterError::Failed(e)) => return Err(ApiError::internal("Adding a client", e)),
    };
    app.activity.record(
        client_entity(&client.client_id, &client.name),
        ActivityReason::ClientAdded,
        ActivityOutcome::Succeeded,
        format!("Client {} was added", client.name),
        None,
    );
    Ok(Created(
        format!("/api/v1/server/clients/{}", client.client_id),
        client,
    ))
}

pub(crate) async fn revoke_client(
    State(app): State<ServerApi>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let name = app.server.name_of(&id).unwrap_or_else(|| id.clone());
    match app.server.revoke(&id).await {
        Ok(true) => {}
        Ok(false) => return Err(no_client()),
        Err(e) => return Err(ApiError::internal("Revoking a client", e)),
    }
    app.activity.record(
        client_entity(&id, &name),
        ActivityReason::ClientRevoked,
        ActivityOutcome::Succeeded,
        format!("Client {name} was revoked"),
        None,
    );
    accepted()
}

fn no_client() -> ApiError {
    ApiError::not_found("There is no such client.")
}

/// One client's secret, fetched only when someone reveals or copies it.
pub(crate) async fn client_secret(
    State(app): State<ServerApi>,
    Path(id): Path<String>,
) -> ApiResult<Json<ClientSecret>> {
    match app.server.secret_of(&id) {
        None => Err(no_client()),
        Some(Some(value)) => Ok(secret(&value)),
        Some(None) => Err(ApiError::refused(
            StatusCode::CONFLICT,
            "secret_not_kept",
            "This client was added before secrets were kept. Rotate the secret to get one you can copy.",
        )),
    }
}

/// A new secret, given once in the answer; the list follows as `ClientsChanged`.
pub(crate) async fn rotate_secret(
    State(app): State<ServerApi>,
    Path(id): Path<String>,
) -> ApiResult<Json<ClientSecret>> {
    let value = match app.server.rotate_secret(&id).await {
        Ok(Some(value)) => value,
        Ok(None) => return Err(no_client()),
        Err(e) => return Err(ApiError::internal("Rotating a client's secret", e)),
    };
    let name = app.server.name_of(&id).unwrap_or_else(|| id.clone());
    app.activity.record(
        client_entity(&id, &name),
        ActivityReason::SecretRotated,
        ActivityOutcome::Succeeded,
        format!("The secret for {name} was rotated"),
        None,
    );
    Ok(secret(&value))
}

pub(crate) async fn keys(State(app): State<ServerApi>) -> Json<KeysView> {
    Json(app.server.keys())
}

fn refused(reason: &str) -> ApiError {
    ApiError::invalid(&[("certificate", reason)])
}

/// The first four bytes, as the UI shows them, so a line in the log can be matched to the
/// fingerprint.
fn short(fingerprint: &str) -> String {
    format!("{}…", fingerprint.get(..11).unwrap_or(fingerprint))
}

pub(crate) async fn prepare_certificate(State(app): State<ServerApi>) -> ApiResult<StatusCode> {
    let next = match app.server.prepare_next_certificate().await {
        Ok(Some(next)) => next,
        Ok(None) => {
            return Err(refused(
                "A new certificate is already prepared. Switch to it or cancel it first.",
            ));
        }
        Err(e) => return Err(ApiError::internal("Preparing a certificate", e)),
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
    accepted()
}

pub(crate) async fn cancel_certificate(State(app): State<ServerApi>) -> ApiResult<StatusCode> {
    let next = match app.server.cancel_next_certificate().await {
        Ok(Some(next)) => next,
        Ok(None) => return Err(refused("There is no new certificate to cancel.")),
        Err(e) => return Err(ApiError::internal("Cancelling a certificate", e)),
    };
    app.activity.record(
        machine(&app),
        ActivityReason::CertificateCancelled,
        ActivityOutcome::Succeeded,
        format!("The new certificate {} was cancelled", short(&next)),
        None,
    );
    accepted()
}

pub(crate) async fn switch_certificate(State(app): State<ServerApi>) -> ApiResult<StatusCode> {
    let done = match app.server.switch_certificate().await {
        Ok(Some(done)) => done,
        Ok(None) => {
            return Err(refused(
                "There is no new certificate to switch to. Prepare one first.",
            ));
        }
        Err(e) => return Err(ApiError::internal("Switching the certificate", e)),
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
    accepted()
}

pub(crate) async fn pairing(State(app): State<ServerApi>) -> Json<PairingOverview> {
    Json(app.server.pairing())
}

/// How long a pairing window stays open when the UI does not say. Skakel holds a window to between
/// one minute and one hour.
const DEFAULT_WINDOW_MINUTES: u64 = 10;

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct OpenPairing {
    #[serde(default)]
    #[ts(type = "number", optional)]
    minutes: Option<u64>,
}

/// Opens the pairing window and answers 202; it shows as open in `PairingRequestsChanged` once
/// Skakel's server says so. Opening it again moves the end.
pub(crate) async fn open_pairing(
    State(app): State<ServerApi>,
    JsonBody(request): JsonBody<OpenPairing>,
) -> ApiResult<StatusCode> {
    let minutes = request.minutes.unwrap_or(DEFAULT_WINDOW_MINUTES);
    if !(1..=60).contains(&minutes) {
        return Err(ApiError::invalid(&[(
            "minutes",
            "Pairing can stay open for between 1 and 60 minutes.",
        )]));
    }
    app.server
        .open_pairing(std::time::Duration::from_secs(minutes * 60));
    accepted()
}

/// Closes the pairing window and answers 202; requests that are already open carry on.
pub(crate) async fn close_pairing(State(app): State<ServerApi>) -> ApiResult<StatusCode> {
    app.server.close_pairing();
    accepted()
}

/// The right code answers 202; the request leaving `PairingRequestsChanged` and the new client in
/// `ClientsChanged` confirm it. A wrong code is a validation problem at once.
pub(crate) async fn approve(
    State(app): State<ServerApi>,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<Approve>,
) -> ApiResult<StatusCode> {
    match app
        .server
        .approve(&id, request.code.as_deref().unwrap_or_default())
        .await
    {
        Approval::Approved => accepted(),
        Approval::WrongCode => Err(ApiError::invalid(&[(
            "code",
            "That is not the code the client shows. Check it on the client and type it again.",
        )])),
        Approval::NotFound => Err(no_request()),
        // 0.2.0 answered 404 here, which a person reads as "that request is gone" when it is a
        // write that failed and can be tried again.
        Approval::Failed => Err(ApiError::internal(
            "Approving a pairing request",
            "the approval could not be saved",
        )),
    }
}

fn no_request() -> ApiError {
    ApiError::not_found("There is no such pairing request.")
}

pub(crate) async fn deny(
    State(app): State<ServerApi>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if app.server.deny(&id).await {
        accepted()
    } else {
        Err(no_request())
    }
}

pub(crate) async fn devices(State(app): State<ServerApi>) -> Json<ServerDevices> {
    Json(app.usb_server.current())
}

/// Applies the choice, which is saved in `devices.json` once it took effect, and answers 202; the
/// outcome reaches the UI as `DevicesChanged`.
pub(crate) async fn share(
    State(app): State<ServerApi>,
    Path(bus_id): Path<String>,
    JsonBody(request): JsonBody<Share>,
) -> ApiResult<StatusCode> {
    if bus_id.parse::<BusId>().is_err() {
        return Err(ApiError::invalid(&[(
            "busId",
            "That is not a USB bus ID, such as 1-1.4.",
        )]));
    }
    if let Some(reason) = app.usb_server.current().unavailable {
        return Err(ApiError::invalid(&[("device", &reason)]));
    }
    app.usb_server.set_shared(&bus_id, request.shared).await;
    accepted()
}

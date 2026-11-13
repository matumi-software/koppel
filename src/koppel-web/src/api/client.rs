//! The client role's endpoints: the stored connection and its test, the servers found on the
//! network, pairing, and the devices.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use koppel_usb::{BusId, ClientDevices};

use super::{ApiError, ApiResult, accepted};
use koppel_app::connection::{ConnectionUpdate, ConnectionView, PairStart, https_authority};
use koppel_app::roles::{ConnectionTest, DiscoveredServer, PairingStatus};

use crate::json::JsonBody;
use crate::state::ClientApi;

pub(crate) async fn connection(State(app): State<ClientApi>) -> Json<ConnectionView> {
    Json(ConnectionView::of(&app.connection.current()))
}

/// Answers 202 with no body; the stored connection reaches the UI as `ConnectionChanged` once
/// `connection.json` is written. The secret is never sent back.
pub(crate) async fn save_connection(
    State(app): State<ClientApi>,
    JsonBody(update): JsonBody<ConnectionUpdate>,
) -> ApiResult<StatusCode> {
    let errors = update.validate();
    if !errors.is_empty() {
        return Err(ApiError::invalid(&errors));
    }
    if update.client_secret.as_deref().is_none_or(str::is_empty)
        && !update.keeps_server(&app.connection.current())
    {
        return Err(ApiError::invalid(&[(
            "clientSecret",
            "Enter the client secret. The stored one only carries over while the server address and fingerprint stay the same.",
        )]));
    }
    if let Err(e) = app.connection.update(|current| update.apply(current)).await {
        return Err(ApiError::internal("Saving the connection", e));
    }
    accepted()
}

/// A query, so it answers with its result.
pub(crate) async fn test(State(app): State<ClientApi>) -> Json<ConnectionTest> {
    Json(app.client.test().await)
}

/// Servers found on the local network. The list also follows as `ServersDiscovered`.
pub(crate) async fn discovery(State(app): State<ClientApi>) -> Json<Vec<DiscoveredServer>> {
    Json(app.client.discovered())
}

pub(crate) async fn pairing(State(app): State<ClientApi>) -> Json<PairingStatus> {
    Json(app.client.pairing())
}

/// Answers 202 and pairs in the background; `PairingStatusChanged` carries the code and the outcome.
pub(crate) async fn begin_pairing(
    State(app): State<ClientApi>,
    JsonBody(request): JsonBody<PairStart>,
) -> ApiResult<StatusCode> {
    let errors = request.validate();
    if !errors.is_empty() {
        return Err(ApiError::invalid(&errors));
    }
    let url = request
        .server_url
        .as_deref()
        .and_then(https_authority)
        .unwrap_or_default();
    app.client
        .begin_pairing(
            &url,
            request.server_fingerprint.as_deref().unwrap_or_default(),
            request.server_name.as_deref(),
        )
        .await;
    accepted()
}

/// The server approved, but saving replaces the connection to another server: the person confirms.
pub(crate) async fn confirm_pairing(State(app): State<ClientApi>) -> ApiResult<StatusCode> {
    if app.client.confirm_pairing().await {
        accepted()
    } else {
        Err(ApiError::invalid(&[(
            "pairing",
            "There is nothing to confirm. Start pairing again.",
        )]))
    }
}

pub(crate) async fn cancel_pairing(State(app): State<ClientApi>) -> ApiResult<StatusCode> {
    app.client.cancel_pairing().await;
    accepted()
}

pub(crate) async fn devices(State(app): State<ClientApi>) -> Json<ClientDevices> {
    Json(app.usb_client.current())
}

/// Attach remembers the device, so it is attached again after a restart. Answers 202; the device
/// shows as attached, or with its fault, when `ClientDevicesChanged` arrives.
pub(crate) async fn attach(
    State(app): State<ClientApi>,
    Path(bus_id): Path<String>,
) -> ApiResult<StatusCode> {
    if bus_id.parse::<BusId>().is_err() {
        return Err(not_a_bus_id());
    }
    app.usb_client.attach(&bus_id).await;
    accepted()
}

fn not_a_bus_id() -> ApiError {
    ApiError::invalid(&[("busId", "That is not a USB bus ID.")])
}

/// Detach forgets the device. Detaching one that is not attached is fine too.
pub(crate) async fn detach(
    State(app): State<ClientApi>,
    Path(bus_id): Path<String>,
) -> ApiResult<StatusCode> {
    if bus_id.parse::<BusId>().is_err() {
        return Err(not_a_bus_id());
    }
    app.usb_client.detach(&bus_id).await;
    accepted()
}

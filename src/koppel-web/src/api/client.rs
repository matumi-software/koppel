//! The client role's endpoints: the servers it attaches from (add by code or key, change, remove,
//! test), the servers found on the network, and the devices of every server.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use koppel_app::roles::{
    AddServer, ConnectionTest, DiscoveredServer, ServerSettings, ServerView, ServersError,
    ServersView,
};
use koppel_usb::{BusId, ClientServersView, ClientsError};

use super::{ApiError, ApiResult, accepted};
use crate::json::JsonBody;
use crate::state::ClientApi;

impl From<ServersError> for ApiError {
    fn from(error: ServersError) -> Self {
        match error {
            ServersError::NoSuchServer => Self::not_found(error.to_string()),
            ServersError::Invalid { field, message } => Self::invalid(&[(field, &message)]),
            ServersError::AlreadyAdded => {
                Self::refused(StatusCode::CONFLICT, "already_added", error.to_string())
            }
            ServersError::Enroll(message) => Self::refused(
                StatusCode::UNPROCESSABLE_ENTITY,
                "enrolment_failed",
                message,
            ),
            ServersError::Save(_) => Self::internal("Saving the servers", error),
            ServersError::Usb(error) => error.into(),
        }
    }
}

impl From<ClientsError> for ApiError {
    fn from(error: ClientsError) -> Self {
        match error {
            ClientsError::NoSuchServer => Self::not_found(error.to_string()),
            ClientsError::BusIdBusy => {
                Self::refused(StatusCode::CONFLICT, "bus_id_busy", error.to_string())
            }
            ClientsError::Attach(_) => Self::invalid(&[("busId", "That is not a USB bus ID.")]),
        }
    }
}

pub(crate) async fn servers(State(app): State<ClientApi>) -> Json<ServersView> {
    Json(app.servers.view())
}

/// Enrols with the server and saves it, so it answers when that is done: 201 with the new server,
/// or the reason it was not added. The list also follows as `ServersChanged`.
pub(crate) async fn add_server(
    State(app): State<ClientApi>,
    JsonBody(request): JsonBody<AddServer>,
) -> ApiResult<(StatusCode, Json<ServerView>)> {
    let added = app.servers.add(request).await?;
    Ok((StatusCode::CREATED, Json(added)))
}

/// Answers 202; the change reaches the UI as `ServersChanged` once `connection.json` is written.
pub(crate) async fn change_server(
    State(app): State<ClientApi>,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<ServerSettings>,
) -> ApiResult<StatusCode> {
    app.servers.change(&id, change).await?;
    accepted()
}

/// Releases the server's devices and forgets the server.
pub(crate) async fn remove_server(
    State(app): State<ClientApi>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    app.servers.remove(&id).await?;
    accepted()
}

/// A query, so it answers with its result.
pub(crate) async fn test(
    State(app): State<ClientApi>,
    Path(id): Path<String>,
) -> ApiResult<Json<ConnectionTest>> {
    Ok(Json(app.servers.test(&id).await?))
}

/// Servers found on the local network. The list also follows as `ServersDiscovered`.
pub(crate) async fn discovery(State(app): State<ClientApi>) -> Json<Vec<DiscoveredServer>> {
    Json(app.client.discovered())
}

pub(crate) async fn devices(State(app): State<ClientApi>) -> Json<ClientServersView> {
    Json(app.usb_client.current())
}

/// Attach remembers the device, so it is attached again after a restart. Answers 202; the device
/// shows as attached, or with its fault, when `ClientDevicesChanged` arrives.
pub(crate) async fn attach(
    State(app): State<ClientApi>,
    Path((server_id, bus_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    if bus_id.parse::<BusId>().is_err() {
        return Err(not_a_bus_id());
    }
    match app.usb_client.attach(&server_id, &bus_id).await {
        Err(ClientsError::Attach(_)) | Ok(()) => accepted(),
        Err(other) => Err(other.into()),
    }
}

fn not_a_bus_id() -> ApiError {
    ApiError::invalid(&[("busId", "That is not a USB bus ID.")])
}

/// Detach forgets the device. Detaching one that is not attached is fine too.
pub(crate) async fn detach(
    State(app): State<ClientApi>,
    Path((server_id, bus_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    if bus_id.parse::<BusId>().is_err() {
        return Err(not_a_bus_id());
    }
    app.usb_client.detach(&server_id, &bus_id).await?;
    accepted()
}

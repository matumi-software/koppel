//! The REST API, with 0.1.x's paths, JSON and errors. A change answers 202 with no body (or 201 or
//! 200 where 0.1.x gave something back), and the confirmed state follows over the live push.

pub(crate) mod client;
mod server;
mod settings;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post, put};

use crate::app::App;
use crate::auth::endpoints as auth;
use crate::push;

/// Every route of the UI's API and of Koppel's own network API. The guards in front of them decide
/// who reaches which (see `guard.rs`).
pub(crate) fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/auth/state", get(auth::state))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/password", post(auth::change_password))
        .route("/api/events", get(push::events))
        .route(
            "/api/settings",
            get(settings::get_settings).patch(settings::patch_settings),
        )
        .route("/api/activity", get(settings::activity))
        .route("/api/system/checks", get(settings::checks))
        .route("/api/system/info", get(settings::info))
        .route("/api/server/identity", get(server::identity))
        .route(
            "/api/server/clients",
            get(server::clients).post(server::create_client),
        )
        .route("/api/server/clients/{id}", delete(server::revoke_client))
        .route(
            "/api/server/clients/{id}/secret",
            get(server::client_secret),
        )
        .route(
            "/api/server/clients/{id}/rotate",
            post(server::rotate_secret),
        )
        .route("/api/server/keys", get(server::keys))
        .route(
            "/api/server/keys/signing-key/rotate",
            post(server::rotate_signing_key),
        )
        .route(
            "/api/server/keys/next-certificate",
            post(server::prepare_certificate).delete(server::cancel_certificate),
        )
        .route(
            "/api/server/keys/next-certificate/switch",
            post(server::switch_certificate),
        )
        .route("/api/server/pairing", get(server::pairing))
        .route("/api/server/pairing/{id}/approve", post(server::approve))
        .route("/api/server/pairing/{id}/deny", post(server::deny))
        .route("/api/server/devices", get(server::devices))
        .route("/api/server/devices/{bus_id}/shared", put(server::share))
        .route("/api/remote/devices", get(server::remote_devices))
        .route(
            "/api/client/connection",
            get(client::connection).put(client::save_connection),
        )
        .route("/api/client/connection/test", post(client::test))
        .route("/api/client/discovery", get(client::discovery))
        .route(
            "/api/client/pairing",
            get(client::pairing)
                .post(client::begin_pairing)
                .delete(client::cancel_pairing),
        )
        .route("/api/client/pairing/confirm", post(client::confirm_pairing))
        .route("/api/client/devices", get(client::devices))
        .route("/api/client/devices/{bus_id}/attach", post(client::attach))
        .route("/api/client/devices/{bus_id}/detach", post(client::detach))
}

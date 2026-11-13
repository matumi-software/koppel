//! The REST API of the UI, under `/api/v1`. A change answers 202 with no body (or 201 or 200 where it
//! gives something back), and the confirmed state follows over the live push. A refusal is problem
//! JSON (see [`ApiError`]).
//!
//! Paths name resources, and the verb is the method: a secret is replaced with `POST` on the
//! client's `secret`, a device is attached with `PUT` and detached with `DELETE` on its
//! `attachment`, the pairing window is opened with `PUT` and closed with `DELETE`, and the server
//! switches to its prepared certificate with `PUT` on `current-certificate`.
//!
//! [`router`] is the whole `/api/v1` tree, nested there by the UI router, in four kinds of routes:
//!
//! - **anonymous:** `/auth/state`, the first password and login, which work signed out (the state
//!   says whether the caller is signed in). Setup and login sit behind the sign-in rate limit.
//! - **protected:** everything else, behind [`require_session`](crate::guard::require_session): the
//!   settings, activity, system, the push and sign-out, and the password change (also rate limited).
//!   A route added here needs a session without anyone remembering to ask for one, and so does a
//!   path no route has, which answers 401 signed out and 404 signed in.
//! - **server** and **client:** the role's own routes, protected as well, and not found while the
//!   machine's role lacks them (the role is read on each request, and answers before the session).
//!
//! Every route of the tree is behind the origin check and carries the session when there is one.

pub(crate) mod client;
mod error;
pub(crate) mod server;
pub(crate) mod settings;

#[cfg(test)]
pub(crate) use error::Problem;
pub(crate) use error::{ApiError, ApiResult, Created, accepted};

use std::sync::Arc;

use axum::Router;
use axum::extract::FromRef;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, get, post, put};

use crate::app::App;
use crate::auth::endpoints as auth;
use crate::guard::{self, RoleGate};
use crate::push;
use crate::state::{AuthApi, ClientApi, PushApi, ServerApi, SettingsApi};
use crate::surface::RoleScope;

/// The API's routes with their guards. Nested at `/api`.
pub(crate) fn router(app: &Arc<App>) -> Router {
    let role = |scope| {
        from_fn_with_state(
            RoleGate {
                settings: app.core.settings.clone(),
                scope,
            },
            guard::role_gate,
        )
    };
    let signed_in = || from_fn(guard::require_session);

    let anonymous = Router::new()
        .route("/auth/state", get(auth::state))
        .with_state(AuthApi::from_ref(app));

    // The first password and sign-in: no session yet, and a limit per address on each try.
    let sign_in = Router::new()
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route(
            "/auth/password",
            post(auth::change_password).route_layer(signed_in()),
        )
        .route_layer(from_fn_with_state(
            app.auth_limit.clone(),
            guard::auth_limit,
        ))
        .with_state(AuthApi::from_ref(app));

    let protected = Router::new()
        .merge(
            Router::new()
                .route("/auth/logout", post(auth::logout))
                .with_state(AuthApi::from_ref(app)),
        )
        .merge(
            Router::new()
                .route("/events", get(push::events))
                .with_state(PushApi::from_ref(app)),
        )
        .merge(
            Router::new()
                .route(
                    "/settings",
                    get(settings::get_settings).patch(settings::patch_settings),
                )
                .route("/activity", get(settings::activity))
                .route("/system/checks", get(settings::checks))
                .route("/system/info", get(settings::info))
                .with_state(SettingsApi::from_ref(app)),
        )
        .fallback(|| async { ApiError::not_found("There is no such API path.") })
        .layer(signed_in());

    // The roles' routes. The role gate is outside the session check: a route the role does not have
    // is not found, signed in or not.
    let server = Router::new()
        .route("/server/identity", get(server::identity))
        .route(
            "/server/clients",
            get(server::clients).post(server::create_client),
        )
        .route("/server/clients/{id}", delete(server::revoke_client))
        .route(
            "/server/clients/{id}/secret",
            get(server::client_secret).post(server::rotate_secret),
        )
        .route("/server/keys", get(server::keys))
        .route(
            "/server/keys/next-certificate",
            post(server::prepare_certificate).delete(server::cancel_certificate),
        )
        .route(
            "/server/keys/current-certificate",
            put(server::switch_certificate),
        )
        .route("/server/pairing", get(server::pairing))
        .route(
            "/server/pairing/window",
            put(server::open_pairing).delete(server::close_pairing),
        )
        .route("/server/pairing/requests/{id}", delete(server::deny))
        .route(
            "/server/pairing/requests/{id}/approval",
            post(server::approve),
        )
        .route("/server/devices", get(server::devices))
        .route("/server/devices/{bus_id}/shared", put(server::share))
        .with_state(ServerApi::from_ref(app))
        .route_layer(signed_in())
        .route_layer(role(RoleScope::Server));

    let client = Router::new()
        .route(
            "/client/connection",
            get(client::connection).put(client::save_connection),
        )
        .route("/client/connection/test", post(client::test))
        .route("/client/discovery", get(client::discovery))
        .route(
            "/client/pairing",
            get(client::pairing)
                .post(client::begin_pairing)
                .delete(client::cancel_pairing),
        )
        .route(
            "/client/pairing/confirmation",
            post(client::confirm_pairing),
        )
        .route("/client/devices", get(client::devices))
        .route(
            "/client/devices/{bus_id}/attachment",
            put(client::attach).delete(client::detach),
        )
        .with_state(ClientApi::from_ref(app))
        .route_layer(signed_in())
        .route_layer(role(RoleScope::Client));

    let api = Router::new()
        .merge(protected)
        .merge(server)
        .merge(client)
        .merge(anonymous)
        .merge(sign_in)
        // Anonymous routes see the session too, when there is one. The origin check is outside it, so
        // a request from another site is turned away before its cookie is looked at.
        .layer(from_fn_with_state(app.auth.clone(), guard::session))
        .layer(from_fn(guard::ui_origin));
    // Outermost, so a refusal carries it too.
    guard::no_store(api)
}

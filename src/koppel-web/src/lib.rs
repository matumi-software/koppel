//! Koppel's web server: HTTPS on tokio-rustls with a certificate that can be replaced while it runs,
//! HTTP/1.1 and HTTP/2 by ALPN, plain HTTP answered with a redirect, the built UI served from inside
//! the program, and the UI's server side: sign-in, the REST API and the live push.
//!
//! - [`App`]: everything the UI's server side holds, started from the config files, and
//!   [`App::routers`], one router for each surface (the UI and the network), each with its guards.
//! - [`serve`]: the accept loop for one listener, and [`bind`] to make one.
//! - [`roles`]: the server and client roles on Skakel, and the USB layer behind the UI.
//! - [`push`]: the live push, Server-Sent Events at `/api/events`.
//! - [`tray`]: what the Windows service tells its tray icons.

mod access;
mod activity;
mod api;
mod app;
mod auth;
mod guard;
mod host;
mod json;
pub mod push;
mod redirect;
pub mod roles;
mod routes;
mod server;
mod state;
mod surface;
pub mod tls;
pub mod tray;
mod ui;

use axum::Router;
use axum::routing::get;

pub use access::UiReach;
pub use activity::{ActivityEvent, ActivityLog, ActivityOutcome, ActivityReason, EntityRef};
pub use app::{App, AppOptions, NetworkOptions, StartError, UsbOptions};
pub use auth::reset_password;
pub use host::{Host, machine_name, os_label};
pub use routes::Routers;
pub use server::{bind, serve};
pub use tls::CertificateSlot;

/// The routes that need nothing of the app: `/healthz` for health checks, and the UI for everything
/// else.
pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(get(ui::serve))
}

/// Anonymous, in any role, for a container's health check. Answers as 0.1.x does.
async fn healthz() -> &'static str {
    "ok"
}

/// Why the web server could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// rustls refused the TLS settings.
    #[error("TLS could not be set up: {0}")]
    Tls(#[source] rustls::Error),
    /// A certificate could not be made.
    #[error("the certificate could not be made: {0}")]
    Certificate(String),
}

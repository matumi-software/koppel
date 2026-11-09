//! Koppel's web server: HTTPS on tokio-rustls with a certificate that can be replaced while it runs,
//! HTTP/1.1 and HTTP/2 by ALPN, and the built UI served from inside the program.

mod server;
pub mod tls;
mod ui;

use std::fmt;

use axum::Router;
use axum::routing::get;

pub use server::serve;
pub use tls::CertificateSlot;

/// The routes: `/healthz` for health checks, and the UI for everything else.
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
#[derive(Debug)]
pub enum WebError {
    /// rustls refused the TLS settings.
    Tls(rustls::Error),
    /// A certificate could not be made.
    Certificate(String),
}

impl fmt::Display for WebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tls(e) => write!(f, "TLS could not be set up: {e}"),
            Self::Certificate(e) => write!(f, "the certificate could not be made: {e}"),
        }
    }
}

impl std::error::Error for WebError {}

//! The routers, one per surface, each with the layers that surface needs (see `guard.rs` for what
//! each layer answers):
//!
//! - **UI** ([`ui`]): `/healthz`, the REST API under `/api` ([`crate::api::router`]) and the UI's own
//!   files for everything else. Behind `Ui:AllowFrom`; its API is behind the origin check and the
//!   session.
//! - **Network** ([`network`]): what other Koppel machines call, while the role is server or both:
//!   Koppel's `/api/remote/devices`, and the calls Skakel's server answers, which are its HTTPS
//!   calls under `/skakel` and the gRPC services (the network router's fallback, [`skakel_calls`]).
//!   Skakel checks the tokens.
//!
//! A router answers on the port its listener is on, so a request reaches the surface of its port by
//! construction: when `Ports:Network` is set the two run on listeners of their own and a UI path on the
//! network port is 404, and the other way round. When the two ports are one, [`Routers::merged`] hands
//! each request to one of them by [`is_network_path`].
//!
//! Both are wrapped, outermost first, in the security headers, a panic catcher (a handler that panics
//! answers 500 where the connection would have gone), a trace of each request at debug level, and the
//! refusal of a request with no caller address. The UI router also times a handler out after 30
//! seconds and a request body that stops arriving for 10; the network router does neither, since
//! gRPC calls and USB streams are long and quiet between messages (Skakel limits its own calls: a
//! 30 second header read, a 4 KiB body on its token and pairing calls).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::timeout::{RequestBodyTimeoutLayer, TimeoutLayer};
use tower_http::trace::TraceLayer;
use tower_service::Service;

use crate::app::App;
use crate::guard::{self, RoleGate};
use crate::roles::SkakelServerRole;
use crate::surface::{RoleScope, is_grpc_path, is_network_path, starts_with_segments};
use crate::{api, ui};

/// How long a UI handler may take to answer before the caller gets 408.
const UI_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a UI request's body may go without another piece before it is given up on. A UI body is
/// a few lines of JSON, so a sender slower than this is holding the connection for nothing.
const UI_BODY_TIMEOUT: Duration = Duration::from_secs(10);

/// The two routers of an app.
pub struct Routers {
    /// For the UI's port.
    pub ui: Router,
    /// For the network port.
    pub network: Router,
}

impl Routers {
    pub(crate) fn of(app: &Arc<App>) -> Self {
        Self {
            ui: ui(app),
            network: network(app),
        }
    }

    /// Both on one listener, for when the UI and network ports are the same port: a request goes to
    /// the network router when its path is the network surface's and to the UI router otherwise.
    pub fn merged(self) -> Router {
        let Self { ui, network } = self;
        Router::new().fallback(move |request: Request| {
            let mut router = if is_network_path(request.uri().path()) {
                network.clone()
            } else {
                ui.clone()
            };
            async move {
                match router.call(request).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                }
            }
        })
    }
}

/// What every router has outside its own guards.
fn wrapped(router: Router) -> Router {
    guard::security_headers(
        router
            .layer(from_fn(guard::connection_info))
            .layer(TraceLayer::new_for_http())
            .layer(CatchPanicLayer::new()),
    )
}

/// The UI's router. See the module docs.
pub(crate) fn ui(app: &Arc<App>) -> Router {
    let router = Router::new()
        .route("/healthz", get(crate::healthz))
        .nest("/api/v1", api::router(app))
        .fallback(files)
        .layer(from_fn_with_state(app.access.clone(), guard::allow_from))
        .layer(from_fn(guard::network_paths_are_not_here));
    wrapped(ui_timeouts(router))
}

/// The UI's timeouts: a handler that is not done in 30 seconds answers 408, and a body that goes 10
/// seconds without a piece is an error for whoever reads it (the JSON extractor, which answers 400).
/// The live push is not cut by either: its handler returns at once and the stream is the body of the
/// response, which these do not time.
fn ui_timeouts(router: Router) -> Router {
    router
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            UI_TIMEOUT,
        ))
        .layer(RequestBodyTimeoutLayer::new(UI_BODY_TIMEOUT))
}

/// The UI's files for GET and HEAD, whatever no route took: the file, or index.html for a path of
/// the single page app.
async fn files(request: Request) -> Response {
    if request.method() == Method::GET || request.method() == Method::HEAD {
        let (parts, _) = request.into_parts();
        ui::serve(parts.uri, parts.headers).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// The network surface's router. See the module docs.
pub(crate) fn network(app: &Arc<App>) -> Router {
    let skakel = app.skakel();
    // Skakel's paths are not routes: a route with a parameter would leave it on the request, where
    // Skakel's own router would count it among the parameters of its routes.
    let router = Router::new()
        .route("/api/remote/devices", get(forward))
        .fallback(skakel_calls)
        .with_state(skakel)
        .layer(from_fn_with_state(
            RoleGate {
                settings: app.settings.clone(),
                scope: RoleScope::Server,
            },
            guard::role_gate,
        ));
    wrapped(router)
}

/// What Skakel's server answers, told by its path: its HTTPS calls under `/skakel` and the gRPC
/// services (Skakel's own and Koppel's), by their service names. Any other path is 404.
async fn skakel_calls(State(skakel): State<Arc<SkakelServerRole>>, request: Request) -> Response {
    let path = request.uri().path();
    if starts_with_segments(path, "/skakel") || is_grpc_path(path) {
        forward(State(skakel), request).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Hands a request to Skakel's server, mounted here. It carries the caller's address from the
/// accept loop, which Skakel's limits count by.
async fn forward(State(skakel): State<Arc<SkakelServerRole>>, request: Request) -> Response {
    skakel.serve(request).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::post;
    use futures_util::{StreamExt, stream};
    use tower::ServiceExt;

    use super::*;

    fn echo() -> Router {
        ui_timeouts(Router::new().route("/echo", post(|body: String| async move { body })))
    }

    /// A body that sends `first` and then nothing, ever.
    fn stalled(first: &'static str) -> Body {
        Body::from_stream(
            stream::once(async move { Ok::<_, std::io::Error>(axum::body::Bytes::from(first)) })
                .chain(stream::pending()),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_body_that_stops_arriving_is_given_up_on_after_ten_seconds() {
        let started = tokio::time::Instant::now();
        let response = echo()
            .oneshot(
                HttpRequest::post("/echo")
                    .body(stalled("{\"password\":"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_client_error(), "{}", response.status());
        assert_eq!(started.elapsed(), UI_BODY_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_body_that_keeps_arriving_is_not_cut() {
        let slow = stream::unfold(0u32, |n| async move {
            (n < 3).then(|| {
                (
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"ab")),
                    n + 1,
                )
            })
        })
        .then(|piece| async move {
            tokio::time::sleep(UI_BODY_TIMEOUT - Duration::from_secs(1)).await;
            piece
        });
        let response = echo()
            .oneshot(
                HttpRequest::post("/echo")
                    .body(Body::from_stream(slow))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn a_handler_that_takes_over_30_seconds_answers_408() {
        let slow = Router::new().route(
            "/slow",
            post(|| async {
                tokio::time::sleep(UI_TIMEOUT + Duration::from_secs(1)).await;
            }),
        );
        let response = ui_timeouts(slow)
            .oneshot(HttpRequest::post("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    }
}

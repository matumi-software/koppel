//! The guards in front of the routes, one request at a time through the router with the address the
//! accept loop would give it: the UI's guards cannot be skipped by what a request says about itself,
//! and a real gRPC call still reaches Skakel on the network surface.

mod common;

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Request, StatusCode};
use common::{FakeKernel, Node, TempDir};
use tower::ServiceExt;

/// An address the UI does not answer (`Ui:AllowFrom=local` in the tests): not this machine's.
const STRANGER: [u8; 4] = [203, 0, 113, 9];
const LOCAL: [u8; 4] = [127, 0, 0, 1];

/// A POST to `path` with `content_type`, as if from `from`.
fn post(path: &str, content_type: &str, from: [u8; 4]) -> Request<Body> {
    let mut request = Request::post(path)
        .header(CONTENT_TYPE, content_type)
        .body(Body::from("{}"))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((from, 50_000))));
    request
}

async fn status(node: &Node, request: Request<Body>) -> StatusCode {
    node.app.router().oneshot(request).await.unwrap().status()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_grpc_content_type_does_not_skip_the_ui_guards() {
    let dir = TempDir::new("server");
    let server = Node::start(dir.path(), "server", None, FakeKernel::new()).await;

    // From an address Ui:AllowFrom does not take, whatever the content type says.
    for content_type in [
        "application/json",
        "application/grpc",
        "application/grpc+json",
        "application/grpc+proto",
        "Application/GRPC+JSON; charset=utf-8",
    ] {
        for path in ["/api/auth/login", "/api/auth/setup", "/api/settings"] {
            assert_eq!(
                status(&server, post(path, content_type, STRANGER)).await,
                StatusCode::FORBIDDEN,
                "{path} as {content_type} from a stranger"
            );
        }
    }

    // From this machine, a change still needs the header the UI adds.
    for content_type in ["application/grpc+json", "application/grpc"] {
        for path in ["/api/auth/login", "/api/auth/setup"] {
            assert_eq!(
                status(&server, post(path, content_type, LOCAL)).await,
                StatusCode::FORBIDDEN,
                "{path} as {content_type} without X-Koppel-UI"
            );
        }
    }

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_grpc_call_reaches_skakel_on_the_network_surface() {
    let dir = TempDir::new("server");
    let server = Node::start(dir.path(), "server", None, FakeKernel::new()).await;

    // From the same stranger the UI turns away: Skakel's and Koppel's own services answer anyone,
    // and say the call has no token.
    for path in [
        "/skakel.v3.ControlService/Connect",
        "/skakel.v3.StreamService/Open",
        "/koppel.remote.v1.RemoteDevices/Watch",
    ] {
        let response = server
            .app
            .router()
            .oneshot(post(path, "application/grpc", STRANGER))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let grpc_status = response
            .headers()
            .get("grpc-status")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        assert_eq!(
            grpc_status.as_deref(),
            Some("16"),
            "{path} answers UNAUTHENTICATED, as Skakel's own"
        );
    }

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_only_machine_serves_no_grpc() {
    let dir = TempDir::new("client");
    let client = Node::start(dir.path(), "client", None, FakeKernel::new()).await;

    for path in [
        "/skakel.v3.ControlService/Connect",
        "/koppel.remote.v1.RemoteDevices/Watch",
    ] {
        assert_eq!(
            status(&client, post(path, "application/grpc", STRANGER)).await,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    client.stop().await;
}

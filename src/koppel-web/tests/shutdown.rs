//! Stopping the web server while a browser has the live push open. The push is a response that never
//! ends on its own, so a stop used to wait out the whole grace period (3 seconds) for it. The stream
//! now ends when the server stops, and the stop takes no longer than it would without one.
//!
//! This one runs in real time over a real connection: a clock that is paused jumps ahead whenever the
//! runtime is waiting for the network. The paused-clock tests of the stream itself are in
//! `src/push.rs`.

mod common;

use std::time::{Duration, Instant};

use common::{FakeKernel, Node, TempDir, http_client_from};

/// Well under the 3 second grace a stream that did not end would cost.
const QUICK: Duration = Duration::from_millis(1500);

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_live_push_open_does_not_wait_out_the_grace_period() {
    let dir = TempDir::new("shutdown");
    let node = Node::start(dir.path(), "server", None, FakeKernel::new()).await;

    // The same stop with nothing open, to see what the stop costs by itself.
    let quiet_dir = TempDir::new("shutdown-quiet");
    let quiet = Node::start(quiet_dir.path(), "server", None, FakeKernel::new()).await;
    let began = Instant::now();
    quiet.stop().await;
    let baseline = began.elapsed();

    let client = http_client_from(None);
    let mut events = client
        .get(format!("{}/api/v1/events", node.url()))
        .header("Cookie", node.cookie())
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), 200);
    assert_eq!(
        events.headers()["content-type"],
        "text/event-stream",
        "the stream is open"
    );

    let began = Instant::now();
    node.stop().await;
    let took = began.elapsed();
    assert!(
        took < QUICK,
        "stopping with a stream open took {took:?} (with none open, {baseline:?})"
    );

    // The stream is over: the client reads its end, or an error, not more waiting.
    let end = tokio::time::timeout(Duration::from_secs(2), async {
        while let Ok(Some(_)) = events.chunk().await {}
    })
    .await;
    assert!(end.is_ok(), "the stream ended");
}

/// A request that was already in flight when Skakel's server stopped finds it gone. It gets an
/// answer, not a panic on the handler's task.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_after_the_stop_is_answered_and_does_not_panic() {
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use tower::ServiceExt;

    let dir = TempDir::new("shutdown-late");
    let node = Node::start(dir.path(), "server", None, FakeKernel::new()).await;
    let router = node.app.router();
    let cookie = node.cookie().to_owned();
    node.stop().await;

    for path in ["/api/v1/server/clients", "/api/v1/server/keys"] {
        let mut request = Request::builder()
            .uri(path)
            .header("Cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50_000,
            ))));
        let response = router.clone().oneshot(request).await.unwrap();
        assert!(
            response.status().is_success(),
            "{path} answered {}",
            response.status()
        );
    }
}

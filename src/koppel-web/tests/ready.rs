//! `/readyz` next to `/healthz`: anonymous, the same in any role, states and nothing else. The
//! mapping from what the USB layer says to those states is tested in `src/ready.rs`; this runs the
//! real route on a running Koppel.

mod common;

use common::{FakeKernel, Node, TempDir, http_client_from};
use serde_json::{Value, json};

async fn readyz(node: &Node) -> (u16, Value) {
    let answer = http_client_from(None)
        .get(format!("{}/readyz", node.url()))
        .send()
        .await
        .unwrap();
    let status = answer.status().as_u16();
    (status, answer.json().await.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn readyz_answers_without_a_session_in_every_role() {
    for role in ["server", "client", "both"] {
        let dir = TempDir::new(&format!("ready-{role}"));
        let node = Node::start(dir.path(), role, None, FakeKernel::new()).await;
        let (status, body) = readyz(&node).await;
        assert_eq!(status, 200, "{role}: {body}");
        assert_eq!(
            body,
            json!({
                "ready": true,
                "gateway": "off",
                "certificate": "loaded",
                "link": "unconfigured",
            }),
            "{role}"
        );
        node.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn healthz_stays_a_plain_ok() {
    let dir = TempDir::new("ready-healthz");
    let node = Node::start(dir.path(), "server", None, FakeKernel::new()).await;
    let answer = http_client_from(None)
        .get(format!("{}/healthz", node.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status().as_u16(), 200);
    assert_eq!(answer.text().await.unwrap(), "ok");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_system_info_carries_the_build() {
    let dir = TempDir::new("ready-build");
    let node = Node::start(dir.path(), "server", None, FakeKernel::new()).await;
    let info = node.get("/api/v1/system/info").await;
    assert_eq!(info["commit"], "abc123def");
    assert_eq!(info["built"], "2026-01-01");
    node.stop().await;
}

//! A new live push stream starts with the current state of every resource the machine's role has,
//! under the names the changes use, each equal to what the GET of the same resource answers.

mod common;

use std::time::Duration;

use common::{FakeKernel, Node, TempDir, http_client_from};
use serde_json::Value;

/// The first events of a stream, as (name, data), until `count` have come.
async fn first_events(node: &Node, count: usize) -> Vec<(String, Value)> {
    let mut response = http_client_from(None)
        .get(format!("{}/api/v1/events", node.url()))
        .header("Cookie", node.cookie())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut text = String::new();
    let mut events = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(10), async {
        while events.len() < count {
            let Some(chunk) = response.chunk().await.unwrap() else {
                break;
            };
            text.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
            while let Some(end) = text.find("\n\n") {
                let block: String = text.drain(..end + 2).collect();
                let name = block.lines().find_map(|l| l.strip_prefix("event:"));
                let data = block.lines().find_map(|l| l.strip_prefix("data:"));
                if let (Some(name), Some(data)) = (name, data) {
                    events.push((
                        name.trim().to_owned(),
                        serde_json::from_str(data.trim()).unwrap(),
                    ));
                }
            }
        }
    })
    .await;
    assert!(read.is_ok(), "the snapshot did not come: {events:?}");
    events
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_opens_with_the_state_of_each_resource_the_role_has() {
    let dir = TempDir::new("snapshot-both");
    let node = Node::start(dir.path(), "both", None, FakeKernel::new()).await;
    let events = first_events(&node, 9).await;
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "SettingsChanged",
            "ClientsChanged",
            "ServerKeysChanged",
            "PairingRequestsChanged",
            "DevicesChanged",
            "ConnectionChanged",
            "ServersDiscovered",
            "PairingStatusChanged",
            "ClientDevicesChanged",
        ]
    );
    for (name, path) in [
        ("SettingsChanged", "/api/v1/settings"),
        ("ClientsChanged", "/api/v1/server/clients"),
        ("ServerKeysChanged", "/api/v1/server/keys"),
        ("PairingRequestsChanged", "/api/v1/server/pairing"),
        ("DevicesChanged", "/api/v1/server/devices"),
        ("ConnectionChanged", "/api/v1/client/connection"),
        ("ServersDiscovered", "/api/v1/client/discovery"),
        ("PairingStatusChanged", "/api/v1/client/pairing"),
        ("ClientDevicesChanged", "/api/v1/client/devices"),
    ] {
        let pushed = &events.iter().find(|(n, _)| n == name).unwrap().1;
        assert_eq!(pushed, &node.get(path).await, "{name} against GET {path}");
    }
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_only_machine_gets_no_client_state() {
    let dir = TempDir::new("snapshot-server");
    let node = Node::start(dir.path(), "server", None, FakeKernel::new()).await;
    let events = first_events(&node, 5).await;
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "SettingsChanged",
            "ClientsChanged",
            "ServerKeysChanged",
            "PairingRequestsChanged",
            "DevicesChanged",
        ]
    );
    node.stop().await;
}

//! The activity log's words, and what the USB layer tells it: sharing, plugging and unplugging,
//! attaches and releases, and the client's link going up or down. The web side defines nothing of
//! its own: it records with these types, including the entries that are not about USB, and
//! implements [`ActivitySink`] over its log.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::BusId;

/// What an event is about, sent as `device`, `client`, `machine` or `link`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EntityKind {
    Device,
    Client,
    Machine,
    Link,
}

/// What an event is about: a device, a client, this machine or a server link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EntityRef {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntityKind,
}

impl EntityRef {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: EntityKind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
        }
    }

    pub fn device(bus_id: &BusId, name: &str) -> Self {
        Self {
            id: bus_id.to_string(),
            name: name.to_owned(),
            kind: EntityKind::Device,
        }
    }

    pub fn link(name: &str) -> Self {
        Self {
            id: "link".to_owned(),
            name: name.to_owned(),
            kind: EntityKind::Link,
        }
    }
}

/// Why an event was recorded: 0.1.x's `ActivityReason`. The USB layer records the first eight; the
/// web side the rest. Sent as a camelCase string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ActivityReason {
    Shared,
    Unshared,
    Attached,
    Detached,
    Plugged,
    Unplugged,
    LinkUp,
    LinkDown,
    ClientAdded,
    ClientRevoked,
    SecretRotated,
    Reconfigured,
    SignInFailed,
    PasswordChanged,
    CertificatePrepared,
    CertificateCancelled,
    CertificateSwitched,
    ServerMoved,
}

/// Whether the thing the event describes happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ActivityOutcome {
    Succeeded,
    Failed,
}

/// Records activity. The log stamps the id and the time. `detail` carries the reason when the
/// outcome is `Failed`, and the client ID on a server's attach and release.
pub trait ActivitySink: Send + Sync {
    fn record(
        &self,
        entity: EntityRef,
        reason: ActivityReason,
        outcome: ActivityOutcome,
        summary: String,
        detail: Option<String>,
    );
}

/// Records nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoActivity;

impl ActivitySink for NoActivity {
    fn record(
        &self,
        _: EntityRef,
        _: ActivityReason,
        _: ActivityOutcome,
        _: String,
        _: Option<String>,
    ) {
    }
}

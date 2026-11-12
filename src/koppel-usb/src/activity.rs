//! What the USB layer tells the activity log: sharing, plugging and unplugging, attaches and
//! releases, and the client's link going up or down. The log itself lives with the web side,
//! which implements [`ActivitySink`].

use serde::Serialize;

use crate::BusId;

/// What an event is about: a device or a server link, sent as `device` or `link`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntityKind {
    Device,
    Link,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Link => "link",
        }
    }
}

/// What an event is about: a device or a server link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityRef {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntityKind,
}

impl EntityRef {
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

/// Why an event was recorded: the reasons 0.1.x's `ActivityReason` has for USB. Sent as a
/// camelCase string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
}

/// Whether the thing the event describes happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

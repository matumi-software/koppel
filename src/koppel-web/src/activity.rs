//! The activity log: a bounded log in memory, newest first. It survives a page reload and is shared by
//! every browser, but starts empty when the process starts. Each entry is pushed as
//! `ActivityRecorded` once it is stored.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use time::OffsetDateTime;

use tokio::sync::broadcast;

use crate::push::{self, Push};

/// How many entries the log keeps.
const CAPACITY: usize = 500;

/// Entries a watcher may fall behind by before it misses some.
const WATCH_BACKLOG: usize = 64;

pub use koppel_usb::{ActivityOutcome, ActivityReason, EntityKind, EntityRef};

/// One thing that happened. `summary` is the sentence the UI shows; `detail` carries the reason when
/// it failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp_utc: OffsetDateTime,
    pub entity: EntityRef,
    pub reason: ActivityReason,
    pub outcome: ActivityOutcome,
    pub summary: String,
    pub detail: Option<String>,
}

/// The log. Recording stamps the id and the time.
pub struct ActivityLog {
    events: Mutex<VecDeque<ActivityEvent>>,
    push: Push,
    recorded: broadcast::Sender<ActivityEvent>,
}

impl ActivityLog {
    pub fn new(push: Push) -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            push,
            recorded: broadcast::channel(WATCH_BACKLOG).0,
        }
    }

    pub fn record(
        &self,
        entity: EntityRef,
        reason: ActivityReason,
        outcome: ActivityOutcome,
        summary: impl Into<String>,
        detail: Option<String>,
    ) -> ActivityEvent {
        let now = OffsetDateTime::now_utc();
        let event = ActivityEvent {
            id: time_ordered_id(now),
            timestamp_utc: now,
            entity,
            reason,
            outcome,
            summary: summary.into(),
            detail,
        };
        {
            let mut events = self.lock();
            events.push_front(event.clone());
            events.truncate(CAPACITY);
        }
        self.push.send(push::ACTIVITY_RECORDED, &event);
        // Nobody watching is fine.
        let _ = self.recorded.send(event.clone());
        event
    }

    /// Each entry recorded from now on, for the tray's notifications.
    pub fn watch(&self) -> broadcast::Receiver<ActivityEvent> {
        self.recorded.subscribe()
    }

    /// The newest `max` entries, only those about `entity` when it is given.
    pub fn recent(&self, max: usize, entity: Option<&str>) -> Vec<ActivityEvent> {
        self.lock()
            .iter()
            .filter(|e| entity.is_none_or(|id| e.entity.id == id))
            .take(max)
            .cloned()
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<ActivityEvent>> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 32 hex digits that sort by time, as .NET's `Guid.CreateVersion7().ToString("N")`: 48 bits of
/// milliseconds, the version, then random bits.
fn time_ordered_id(now: OffsetDateTime) -> String {
    let millis = (now.unix_timestamp_nanos() / 1_000_000) as u64 & 0xFFFF_FFFF_FFFF;
    let random = crate::auth::random_bytes::<10>();
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&random);
    bytes[6] = 0x70 | (bytes[6] & 0x0F);
    bytes[8] = 0x80 | (bytes[8] & 0x3F);
    hex(&bytes)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(log: &ActivityLog, id: &str) {
        log.record(
            EntityRef::new(id, id, EntityKind::Machine),
            ActivityReason::Reconfigured,
            ActivityOutcome::Succeeded,
            "x",
            None,
        );
    }

    #[test]
    fn newest_first_bounded_and_filtered() {
        let log = ActivityLog::new(Push::new());
        for i in 0..(CAPACITY + 5) {
            record(&log, &format!("e{i}"));
        }
        let all = log.recent(1000, None);
        assert_eq!(all.len(), CAPACITY);
        assert_eq!(all[0].entity.id, format!("e{}", CAPACITY + 4));
        assert_eq!(log.recent(1, Some("e10")).len(), 1);
        assert!(log.recent(5, Some("e0")).is_empty());
    }

    #[test]
    fn written_as_0_1_writes_it() {
        let log = ActivityLog::new(Push::new());
        let event = log.record(
            EntityRef::new("machine", "PC", EntityKind::Machine),
            ActivityReason::SignInFailed,
            ActivityOutcome::Failed,
            "s",
            Some("d".into()),
        );
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["reason"], "signInFailed");
        assert_eq!(json["outcome"], "failed");
        assert_eq!(json["entity"]["type"], "machine");
        assert_eq!(event.id.len(), 32);
        assert!(json["timestampUtc"].as_str().unwrap().ends_with('Z'));
    }
}

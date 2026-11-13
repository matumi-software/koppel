//! `/api/v1/settings`, `/api/v1/activity` and `/api/v1/system/*`.

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use koppel_config::files::{KoppelSettings, Role};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use ts_rs::TS;

use super::{ApiError, ApiResult, accepted};
use crate::json::JsonBody;
use crate::state::SettingsApi;
use koppel_app::activity::{ActivityEvent, ActivityOutcome, ActivityReason, EntityKind, EntityRef};

/// A partial change to the settings. A field left out or null stays as it is.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct SettingsPatch {
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    share_on_startup: Option<bool>,
    #[serde(default)]
    setup_complete: Option<bool>,
    #[serde(default)]
    notify_pairing: Option<bool>,
    #[serde(default)]
    notify_link: Option<bool>,
    #[serde(default)]
    notify_attach_failures: Option<bool>,
}

impl SettingsPatch {
    /// What the patch changes in `current`, in words for the activity log, such as "Role set to
    /// client". Empty when it changes nothing, so repeating a change cannot flood the log.
    fn describe(&self, current: &KoppelSettings) -> String {
        let turned = |what: &str, next: Option<bool>, now: bool| {
            next.filter(|value| *value != now)
                .map(|value| format!("{what} turned {}", if value { "on" } else { "off" }))
        };
        [
            self.role
                .filter(|role| *role != current.role)
                .map(|role| format!("Role set to {}", role.as_str())),
            self.share_on_startup
                .filter(|share| *share != current.share_on_startup)
                .map(|share| {
                    format!(
                        "Share saved devices on startup turned {}",
                        if share { "on" } else { "off" }
                    )
                }),
            (self.setup_complete == Some(true) && !current.setup_complete)
                .then(|| "Setup finished".to_owned()),
            turned(
                "Notifications for pairing requests",
                self.notify_pairing,
                current.notify_pairing,
            ),
            turned(
                "Notifications for the link to the server",
                self.notify_link,
                current.notify_link,
            ),
            turned(
                "Notifications for failed attaches",
                self.notify_attach_failures,
                current.notify_attach_failures,
            ),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(". ")
    }

    fn apply(&self, current: &KoppelSettings) -> KoppelSettings {
        KoppelSettings {
            role: self.role.unwrap_or(current.role),
            share_on_startup: self.share_on_startup.unwrap_or(current.share_on_startup),
            setup_complete: self.setup_complete.unwrap_or(current.setup_complete),
            notify_pairing: self.notify_pairing.unwrap_or(current.notify_pairing),
            notify_link: self.notify_link.unwrap_or(current.notify_link),
            notify_attach_failures: self
                .notify_attach_failures
                .unwrap_or(current.notify_attach_failures),
        }
    }
}

/// The settings as stored.
pub(crate) async fn get_settings(State(app): State<SettingsApi>) -> Json<KoppelSettings> {
    Json(app.settings.current().as_ref().clone())
}

/// Writes a change and answers 202 with no body: the confirmed settings reach the UI as
/// `SettingsChanged`, never in the answer.
pub(crate) async fn patch_settings(
    State(app): State<SettingsApi>,
    JsonBody(patch): JsonBody<SettingsPatch>,
) -> ApiResult<StatusCode> {
    let summary = patch.describe(&app.settings.current());
    if let Err(e) = app.settings.update(|current| patch.apply(current)).await {
        return Err(ApiError::internal("Saving the settings", e));
    }
    if !summary.is_empty() {
        app.activity.record(
            EntityRef::new("machine", app.host.name.clone(), EntityKind::Machine),
            ActivityReason::Reconfigured,
            ActivityOutcome::Succeeded,
            summary,
            None,
        );
    }
    accepted()
}

/// `GET /api/v1/activity?entity=&max=`: recent entries, newest first, 100 unless `max` says otherwise,
/// from 1 to 500.
pub(crate) async fn activity(
    State(app): State<SettingsApi>,
    RawQuery(query): RawQuery,
) -> ApiResult<Json<Vec<ActivityEvent>>> {
    let query = query.unwrap_or_default();
    let mut max = 100i64;
    let mut entity = None;
    for (name, value) in parse_query(&query) {
        if name.eq_ignore_ascii_case("max") {
            if value.is_empty() {
                continue;
            }
            match value.parse::<i32>() {
                Ok(n) => max = i64::from(n),
                Err(_) => {
                    return Err(ApiError::invalid(&[(
                        "max",
                        "Give max as a whole number from 1 to 500.",
                    )]));
                }
            }
        } else if name.eq_ignore_ascii_case("entity") {
            entity = Some(value);
        }
    }
    let max = max.clamp(1, 500) as usize;
    Ok(Json(app.activity.recent(max, entity.as_deref())))
}

/// A query string's pairs, percent-decoded, `+` read as a space.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let pair = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or_default();
                match u8::from_str_radix(pair, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What this machine can do and what is wrong with its files: sharing and attaching first, then a
/// line for each file that was not used.
pub(crate) async fn checks(State(app): State<SettingsApi>) -> Json<Vec<koppel_usb::SystemCheck>> {
    let mut checks = vec![app.usb_server.check(), app.usb_client.check()];
    let files = [
        (
            "settings",
            "settings.json was not applied.",
            app.settings.problem(),
        ),
        (
            "devices",
            "devices.json was not used.",
            app.usb_server.problem(),
        ),
        (
            "attached",
            "attached.json was not used.",
            app.usb_client.problem(),
        ),
    ];
    for (id, title, problem) in files {
        if let Some(problem) = problem {
            checks.push(koppel_usb::SystemCheck::new(id, false, title, &problem));
        }
    }
    Json(checks)
}

#[derive(Serialize, TS)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemInfo {
    name: String,
    version: String,
    commit: String,
    built: String,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    started_at: OffsetDateTime,
    role: Role,
    ui_port: u16,
    https_port: u16,
    fingerprint: String,
    os: String,
    tray: bool,
}

/// What the side panel shows about this machine.
pub(crate) async fn info(State(app): State<SettingsApi>) -> Json<SystemInfo> {
    let host = &app.host;
    Json(SystemInfo {
        name: host.name.clone(),
        version: host.version.clone(),
        commit: host.commit.clone(),
        built: host.built.clone(),
        started_at: host.started_at,
        role: app.settings.current().role,
        ui_port: host.ui_port,
        https_port: host.network_port,
        fingerprint: app.server.fingerprint(),
        os: host.os.clone(),
        tray: host.tray,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_says_what_it_changes_and_nothing_when_it_changes_nothing() {
        let current = KoppelSettings::default();
        let patch = SettingsPatch {
            role: Some(Role::Client),
            share_on_startup: Some(true),
            notify_link: Some(false),
            ..SettingsPatch::default()
        };
        assert_eq!(
            patch.describe(&current),
            "Role set to client. Notifications for the link to the server turned off"
        );
        assert_eq!(SettingsPatch::default().describe(&current), "");
        assert_eq!(patch.apply(&current).role, Role::Client);
    }

    #[test]
    fn queries_are_decoded() {
        assert_eq!(
            parse_query("max=5&entity=a%20b+c&x"),
            vec![
                ("max".into(), "5".into()),
                ("entity".into(), "a b c".into()),
                ("x".into(), String::new())
            ]
        );
        assert_eq!(decode("100%"), "100%");
    }
}

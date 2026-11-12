//! What each config file holds. The JSON is plain serde: camelCase names, dates as RFC 3339 strings
//! and an object that keeps the order it was written in.
//!
//! The files are the ones 0.1.x wrote, and a file from 0.1.x or 0.2.0 reads as it is: a `null` where a
//! list or a map belongs reads as empty, names Koppel wrote that nothing reads any more (`isComplete`,
//! `hasPassword`) are skipped, and a hand-edited `settings.json` may spell a name or a role with a
//! capital. `secrets.json` is the exception: its certificates were PKCS#12 before 0.2.1, and
//! `koppel-upgrade` rewrites that file once, before it is read here.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use skakel_server::SkakelState;
use time::OffsetDateTime;

use crate::redact::Secret;

/// Which side of the link this machine plays. Written as `server`, `client` or `both`; a hand edit
/// may capitalise it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    #[serde(alias = "Server")]
    Server,
    #[serde(alias = "Client")]
    Client,
    #[serde(alias = "Both")]
    Both,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
            Self::Both => "both",
        }
    }
}

/// `null` reads as the type's default, as it did where 0.1.x wrote a missing list as `null`.
fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// `settings.json`: plain, and people may edit it by hand. A name missing from the file takes the
/// value a new install writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KoppelSettings {
    #[serde(alias = "Role")]
    pub role: Role,
    /// Share the devices that were shared before a restart. 0.1.x read a missing value as false.
    #[serde(alias = "ShareOnStartup")]
    pub share_on_startup: bool,
    /// False until the first-run setup in the UI is finished.
    #[serde(alias = "SetupComplete")]
    pub setup_complete: bool,
    /// Windows app: the tray says when a client waits to pair.
    #[serde(alias = "NotifyPairing")]
    pub notify_pairing: bool,
    /// Windows app: the tray says when the link to the server is lost, and when it is back.
    #[serde(alias = "NotifyLink")]
    pub notify_link: bool,
    /// Windows app: the tray says when a device could not be attached.
    #[serde(alias = "NotifyAttachFailures")]
    pub notify_attach_failures: bool,
}

impl Default for KoppelSettings {
    /// What a new install writes: server, and shared devices shared again on start.
    fn default() -> Self {
        Self {
            role: Role::Server,
            share_on_startup: true,
            setup_complete: false,
            notify_pairing: true,
            notify_link: true,
            notify_attach_failures: true,
        }
    }
}

/// `devices.json` (server role): the bus IDs the user chose to share, with the VID:PID each had.
/// Plain.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedDevices {
    #[serde(default, deserialize_with = "or_default")]
    pub bus_ids: Vec<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub vid_pids: IndexMap<String, String>,
}

/// `attached.json` (client role): the devices this client attached and the server they belong to.
/// Plain.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedDevices {
    #[serde(default)]
    pub server_fingerprint: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub bus_ids: Vec<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub vid_pids: IndexMap<String, String>,
}

/// `connection.json` (client role): the server this client links to and its credentials.
/// Encrypted. A part that is not set is left out of the file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConnection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    /// The certificate pinned for this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<Secret<String>>,
    /// The certificate the server said it will switch to, trusted as well until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_server_fingerprint: Option<String>,
}

impl ClientConnection {
    /// Every part needed to link is there.
    pub fn is_complete(&self) -> bool {
        self.server_url.is_some()
            && self.server_fingerprint.is_some()
            && self.client_id.is_some()
            && self.client_secret.is_some()
    }
}

/// `ui-auth.json`: the UI password's hash, the stamp that ends every session when it changes, and
/// the sessions that signed out with when each would have ended. Encrypted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCredentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<Secret<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_stamp: Option<String>,
    #[serde(default, with = "rfc3339_map")]
    pub revoked_sessions: IndexMap<String, OffsetDateTime>,
}

impl UiCredentials {
    /// What a new install writes: no password and an empty stamp.
    pub fn empty() -> Self {
        Self {
            password_hash: None,
            security_stamp: Some(String::new()),
            revoked_sessions: IndexMap::new(),
        }
    }

    pub fn has_password(&self) -> bool {
        self.password_hash.is_some()
    }

    /// Why the value is not acceptable, as 0.1.x's validator says it, or `None`.
    pub fn invalid(&self) -> Option<String> {
        match &self.password_hash {
            Some(hash) if !password_hash_is_well_formed(hash) => {
                Some("the password hash is not in the format this version writes".into())
            }
            _ => None,
        }
    }
}

/// A map of dates as RFC 3339 strings. `null` reads as an empty map.
mod rfc3339_map {
    use super::*;

    #[derive(Deserialize)]
    struct Date(#[serde(with = "time::serde::rfc3339")] OffsetDateTime);

    struct DateRef<'a>(&'a OffsetDateTime);

    impl Serialize for DateRef<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            time::serde::rfc3339::serialize(self.0, serializer)
        }
    }

    pub fn serialize<S: serde::Serializer>(
        map: &IndexMap<String, OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(map.iter().map(|(k, v)| (k, DateRef(v))))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<IndexMap<String, OffsetDateTime>, D::Error> {
        let dates = Option::<IndexMap<String, Date>>::deserialize(deserializer)?;
        Ok(dates
            .unwrap_or_default()
            .into_iter()
            .map(|(k, Date(v))| (k, v))
            .collect())
    }
}

/// Mirrors `UiPassword.IsWellFormed`: `pbkdf2-sha256$iterations$salt$hash` with at least 600,000
/// iterations, a salt of at least 16 bytes and a 32 byte hash, both base64.
pub fn password_hash_is_well_formed(stored: &str) -> bool {
    use base64::Engine;
    let parts: Vec<&str> = stored.split('$').collect();
    let [prefix, iterations, salt, hash] = parts.as_slice() else {
        return false;
    };
    let iterations_ok = !iterations.is_empty()
        && iterations.bytes().all(|b| b.is_ascii_digit())
        && iterations.parse::<i32>().is_ok_and(|n| n >= 600_000);
    if *prefix != "pbkdf2-sha256" || !iterations_ok {
        return false;
    }
    let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text).ok();
    matches!((decode(salt), decode(hash)), (Some(salt), Some(hash)) if salt.len() >= 16 && hash.len() == 32)
}

/// `secrets.json`: Skakel's server state, which is the clients and the certificates. Encrypted.
/// Skakel's own type is stored as it is, so the file holds what the server saves and nothing is
/// copied field by field. Empty until the server first saves.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skakel: Option<SkakelState>,
}

impl ServerSecrets {
    /// The file as Koppel writes it: Skakel's state only.
    pub fn of(state: SkakelState) -> Self {
        Self {
            skakel: Some(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Result<KoppelSettings, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    #[test]
    fn settings_read_with_defaults_and_hand_edited_spellings() {
        assert_eq!(settings(r#"{"role":"client"}"#).unwrap().role, Role::Client);
        let shouted = settings(r#"{"Role":"Client","ShareOnStartup":false}"#).unwrap();
        assert_eq!(
            (shouted.role, shouted.share_on_startup),
            (Role::Client, false)
        );
        assert_eq!(settings(r#"{"role":"Both"}"#).unwrap().role, Role::Both);
        assert_eq!(
            settings(r#"{"role":"client","extra":5}"#).unwrap().role,
            Role::Client
        );
        let empty = settings("{}").unwrap();
        assert_eq!(empty, KoppelSettings::default());
        assert!(empty.share_on_startup);
    }

    #[test]
    fn bad_settings_are_refused() {
        for refused in [
            r#"{"role":1}"#,
            r#"{"role":null}"#,
            r#"{"role":"server, client"}"#,
            r#"{"shareOnStartup":"true"}"#,
            r#"{"role":"client",}"#,
        ] {
            assert!(settings(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn a_null_list_or_map_reads_as_empty_and_is_written_as_empty() {
        let devices: SharedDevices =
            serde_json::from_str(r#"{"busIds":null,"vidPids":null}"#).unwrap();
        assert_eq!(devices, SharedDevices::default());
        assert_eq!(
            serde_json::to_string(&devices).unwrap(),
            r#"{"busIds":[],"vidPids":{}}"#
        );
        let attached: AttachedDevices = serde_json::from_str(
            r#"{"serverFingerprint":"AA","busIds":["1-1"],"vidPids":{"1-1":"1209:0001"}}"#,
        )
        .unwrap();
        assert_eq!(
            attached.vid_pids.get("1-1").map(String::as_str),
            Some("1209:0001")
        );
    }

    #[test]
    fn what_0_1_x_wrote_for_the_encrypted_files_still_reads() {
        // `isComplete` and `hasPassword` were computed by 0.1.x and are skipped; nulls are not set.
        let connection: ClientConnection = serde_json::from_str(
            r#"{"serverUrl":"https://a:5240","serverFingerprint":"AA","clientId":"client-1","clientSecret":"s","nextServerFingerprint":null,"isComplete":true}"#,
        )
        .unwrap();
        assert!(connection.is_complete());
        assert_eq!(connection.next_server_fingerprint, None);
        let written = serde_json::to_string(&connection).unwrap();
        assert!(
            !written.contains("null") && !written.contains("isComplete"),
            "{written}"
        );

        let credentials: UiCredentials = serde_json::from_str(
            r#"{"passwordHash":null,"securityStamp":"abc","revokedSessions":{"s1":"2026-11-09T08:30:15.1234567+00:00","s2":"2026-11-09T08:30:15Z"},"hasPassword":false}"#,
        )
        .unwrap();
        assert!(!credentials.has_password());
        assert_eq!(credentials.revoked_sessions.len(), 2);
        let none: UiCredentials = serde_json::from_str(r#"{"revokedSessions":null}"#).unwrap();
        assert!(none.revoked_sessions.is_empty());
        let written = serde_json::to_string(&credentials).unwrap();
        assert!(
            !written.contains("null") && !written.contains("hasPassword"),
            "{written}"
        );
        let back: UiCredentials = serde_json::from_str(&written).unwrap();
        assert_eq!(back, credentials);
        assert_eq!(
            back.revoked_sessions.keys().collect::<Vec<_>>(),
            ["s1", "s2"],
            "the order is kept"
        );
    }

    /// Every secret in every file struct is a sentinel here; none may come out of `{:?}`, plain or
    /// pretty, however deep the struct is nested.
    #[test]
    fn formatting_a_config_struct_shows_no_secret() {
        let connection = ClientConnection {
            server_url: Some("https://a:5240".into()),
            server_fingerprint: Some("AA".into()),
            client_id: Some("client-1".into()),
            client_secret: Some("SENTINEL-LINK-SECRET".into()),
            next_server_fingerprint: None,
        };
        let credentials = UiCredentials {
            password_hash: Some("SENTINEL-PASSWORD-HASH".into()),
            ..UiCredentials::empty()
        };
        let secrets: ServerSecrets = serde_json::from_str(
            r#"{"skakel":{"clients":[{"id":"client-1","name":"bench","secretHash":"sha256$AAAA","createdAt":"2026-11-09T08:30:15Z","generation":1,"secret":"SENTINEL-CLIENT-SECRET"}],"certificate":{"certificate":"AAAA","privateKey":"SENTINEL-PRIVATE-KEY"}}}"#,
        )
        .unwrap();

        let shown = [
            format!("{connection:?}\n{connection:#?}"),
            format!("{credentials:?}\n{credentials:#?}"),
            format!("{secrets:?}\n{secrets:#?}"),
        ]
        .join("\n");

        assert!(!shown.contains("SENTINEL"), "{shown}");
        assert!(shown.contains("Secret(<redacted>)"));
        // Redacted in memory only: the files still hold the values.
        let written = serde_json::to_string(&connection).unwrap();
        assert!(written.contains("SENTINEL-LINK-SECRET"));
        let written = serde_json::to_string(&secrets).unwrap();
        assert!(written.contains("SENTINEL-PRIVATE-KEY"));
    }

    #[test]
    fn password_hashes_are_checked_as_0_1_checks_them() {
        let good = "pbkdf2-sha256$600000$Buc4cbpmUJFP6mXwVKll2w==$MLwUOgSKos+g2yhMD07OXIyRt6tOX9c/UXzB50xrRzY=";
        assert!(password_hash_is_well_formed(good));
        assert!(!password_hash_is_well_formed(
            &good.replace("600000", "599999")
        ));
        assert!(!password_hash_is_well_formed(
            &good.replace("pbkdf2-sha256", "pbkdf2")
        ));
        assert!(!password_hash_is_well_formed(
            "pbkdf2-sha256$600000$c2FsdA==$aGFzaA=="
        ));
        assert!(!password_hash_is_well_formed(
            "pbkdf2-sha256$+600000$Buc4cbpmUJFP6mXwVKll2w==$MLwUOgSKos+g2yhMD07OXIyRt6tOX9c/UXzB50xrRzY="
        ));
    }
}

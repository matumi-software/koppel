//! What each config file holds, field for field as Koppel 0.1.x writes it. Field order is the
//! order the C# records declare, which is the order the JSON has. A property the C# record only
//! computes (`isComplete`, `hasPassword`) is written as 0.1.x writes it and ignored when read.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use time::OffsetDateTime;

use crate::json::{self, OrderedMap};
use crate::redact::Secret;

/// Which side of the link this machine plays. Written as `server`, `client` or `both`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    Server,
    Client,
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

impl Serialize for Role {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Read as .NET's enum converter reads it with integers turned off: a name in any case, with
/// spaces around it ignored, or several joined by commas, which .NET combines as flags (so
/// `"server, client"` is client). A number, or a string of digits, is refused.
impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RoleVisitor;
        impl Visitor<'_> for RoleVisitor {
            type Value = Role;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("server, client or both, as a name")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Role, E> {
                let mut flags = 0u8;
                for part in text.split(',') {
                    flags |= match part.trim().to_ascii_lowercase().as_str() {
                        "server" => 0,
                        "client" => 1,
                        "both" => 2,
                        _ => {
                            return Err(E::custom(format_args!(
                                "{text:?} is not a role. Use server, client or both."
                            )));
                        }
                    };
                }
                match flags {
                    0 => Ok(Role::Server),
                    1 => Ok(Role::Client),
                    2 => Ok(Role::Both),
                    _ => Err(E::custom("Use server, client or both.")),
                }
            }
        }
        deserializer.deserialize_str(RoleVisitor)
    }
}

fn yes() -> bool {
    true
}

/// `settings.json`: plain, and people may edit it by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KoppelSettings {
    /// A missing role is the enum's first value, as .NET reads it.
    #[serde(default)]
    pub role: Role,
    /// Share the devices that were shared before a restart. Missing reads as true, the same as a
    /// new install writes. 0.1.x read a missing value as false.
    #[serde(default = "yes")]
    pub share_on_startup: bool,
    /// False until the first-run setup in the UI is finished.
    #[serde(default)]
    pub setup_complete: bool,
    /// Windows app: the tray says when a client waits to pair.
    #[serde(default = "yes")]
    pub notify_pairing: bool,
    /// Windows app: the tray says when the link to the server is lost, and when it is back.
    #[serde(default = "yes")]
    pub notify_link: bool,
    /// Windows app: the tray says when a device could not be attached.
    #[serde(default = "yes")]
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
/// Plain. A list .NET read as null stays null when written back.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedDevices {
    #[serde(default)]
    pub bus_ids: Option<Vec<String>>,
    #[serde(default)]
    pub vid_pids: Option<OrderedMap<String>>,
}

impl SharedDevices {
    /// What a new install writes: nothing shared.
    pub fn empty() -> Self {
        Self {
            bus_ids: Some(Vec::new()),
            vid_pids: None,
        }
    }
}

/// `attached.json` (client role): the devices this client attached and the server they belong to.
/// Plain.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedDevices {
    #[serde(default)]
    pub server_fingerprint: Option<String>,
    #[serde(default)]
    pub bus_ids: Option<Vec<String>>,
    #[serde(default)]
    pub vid_pids: Option<OrderedMap<String>>,
}

impl AttachedDevices {
    /// What a new install writes: nothing remembered.
    pub fn empty() -> Self {
        Self {
            server_fingerprint: None,
            bus_ids: Some(Vec::new()),
            vid_pids: None,
        }
    }
}

/// `connection.json` (client role): the server this client links to and its credentials.
/// Encrypted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConnection {
    #[serde(default)]
    pub server_url: Option<String>,
    /// The certificate pinned for this server.
    #[serde(default)]
    pub server_fingerprint: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<Secret<String>>,
    /// The certificate the server said it will switch to, trusted as well until then.
    #[serde(default)]
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

impl Serialize for ClientConnection {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Written<'a> {
            server_url: &'a Option<String>,
            server_fingerprint: &'a Option<String>,
            client_id: &'a Option<String>,
            client_secret: &'a Option<Secret<String>>,
            next_server_fingerprint: &'a Option<String>,
            is_complete: bool,
        }
        Written {
            server_url: &self.server_url,
            server_fingerprint: &self.server_fingerprint,
            client_id: &self.client_id,
            client_secret: &self.client_secret,
            next_server_fingerprint: &self.next_server_fingerprint,
            is_complete: self.is_complete(),
        }
        .serialize(serializer)
    }
}

/// `ui-auth.json`: the UI password's hash, the stamp that ends every session when it changes, and
/// the sessions that signed out with when each would have ended. Encrypted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCredentials {
    #[serde(default)]
    pub password_hash: Option<Secret<String>>,
    #[serde(default)]
    pub security_stamp: Option<String>,
    #[serde(default, with = "json::date_map")]
    pub revoked_sessions: Option<OrderedMap<OffsetDateTime>>,
}

impl UiCredentials {
    /// What a new install writes: no password and an empty stamp.
    pub fn empty() -> Self {
        Self {
            password_hash: None,
            security_stamp: Some(String::new()),
            revoked_sessions: None,
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

impl Serialize for UiCredentials {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Written<'a> {
            password_hash: &'a Option<Secret<String>>,
            security_stamp: &'a Option<String>,
            #[serde(with = "json::date_map")]
            revoked_sessions: &'a Option<OrderedMap<OffsetDateTime>>,
            has_password: bool,
        }
        Written {
            password_hash: &self.password_hash,
            security_stamp: &self.security_stamp,
            revoked_sessions: &self.revoked_sessions,
            has_password: self.has_password(),
        }
        .serialize(serializer)
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

/// `secrets.json`: Skakel's server state, plus the fields Koppel 0.1.0 wrote there before Skakel,
/// which are only read. Encrypted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSecrets {
    #[serde(default)]
    pub skakel: Option<SkakelState>,
    /// Koppel 0.1.0: the RSA signing key, base64 PKCS#8.
    #[serde(default)]
    pub signing_key: Option<Secret<String>>,
    /// Koppel 0.1.0: the certificate, base64 PKCS#12.
    #[serde(default)]
    pub certificate_pfx: Option<Secret<String>>,
    /// Koppel 0.1.0: the registered clients.
    #[serde(default)]
    pub clients: Option<Vec<LegacyClient>>,
    #[serde(default)]
    pub signing_key_id: Option<String>,
    #[serde(default, with = "json::date_time_offset::option")]
    pub signing_key_since: Option<OffsetDateTime>,
    #[serde(default)]
    pub next_certificate: Option<LegacyNextCertificate>,
}

impl ServerSecrets {
    /// The file as Koppel 0.1.1 and later write it: Skakel's state only.
    pub fn of(state: SkakelState) -> Self {
        Self {
            skakel: Some(state),
            ..Self::default()
        }
    }
}

/// Skakel 0.1's server state (`SkakelState` in the .NET Skakel.Server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkakelState {
    pub clients: Vec<SkakelClientRecord>,
    /// The key access tokens were signed with before Skakel 0.3.0, whose tokens are random strings
    /// the server remembers. Read from an older file, never written: the next save drops it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<SkakelKeyRecord>,
    /// Keys that no longer signed, as above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retired_signing_keys: Vec<SkakelKeyRecord>,
    /// The certificate the server presents: base64 PKCS#12 with its private key and no password.
    pub certificate_pfx: Secret<String>,
    /// The certificate prepared for a handover.
    #[serde(default)]
    pub next_certificate: Option<SkakelNextCertificateRecord>,
}

fn first_generation() -> i32 {
    1
}

/// A registered client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkakelClientRecord {
    pub id: String,
    pub name: String,
    /// `sha256$` and the base64 SHA-256 of the secret.
    pub secret_hash: String,
    #[serde(with = "json::date_time_offset")]
    pub created_at: OffsetDateTime,
    #[serde(default = "first_generation")]
    pub generation: i32,
    /// The secret itself: Koppel sets Skakel's `KeepClientSecrets`.
    #[serde(default)]
    pub secret: Option<Secret<String>>,
}

/// A token signing key: RSA, base64 PKCS#8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkakelKeyRecord {
    pub key_id: String,
    pub private_key: Secret<String>,
    #[serde(with = "json::date_time_offset")]
    pub created_at: OffsetDateTime,
    #[serde(default, with = "json::date_time_offset::option")]
    pub accept_until: Option<OffsetDateTime>,
}

/// The certificate prepared for a handover, and the clients that have its fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkakelNextCertificateRecord {
    pub pfx: Secret<String>,
    #[serde(with = "json::date_time_offset")]
    pub prepared_at: OffsetDateTime,
    pub received_by: Vec<String>,
}

/// A client as Koppel 0.1.0 kept it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyClient {
    pub id: String,
    pub name: String,
    pub secret_hash: String,
    #[serde(with = "json::date_time_offset")]
    pub created_at: OffsetDateTime,
    #[serde(default)]
    pub secret: Option<Secret<String>>,
    #[serde(default = "first_generation")]
    pub generation: i32,
}

/// A prepared next certificate as Koppel 0.1.0 kept it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyNextCertificate {
    pub pfx: Secret<String>,
    #[serde(with = "json::date_time_offset")]
    pub prepared_at: OffsetDateTime,
    pub received_by: Vec<String>,
}

/// What became of a Koppel 0.1.0 `secrets.json` carried over to Skakel's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarriedOver {
    pub state: SkakelState,
    /// Clients whose secret hash Skakel cannot check (0.1.0's PBKDF2 one): they pair again.
    pub dropped: Vec<LegacyClient>,
}

impl ServerSecrets {
    /// Skakel's state, carried over from Koppel 0.1.0's fields when that is what the file holds,
    /// as `ServerSecretsStore.CarryOver` does it. `Ok(None)` for a file with neither, which is a
    /// first start. `Err` when 0.1.0's key or certificate cannot be read: 0.1.x then started
    /// fresh and kept the file aside.
    pub fn skakel_state(&self, now: OffsetDateTime) -> Result<Option<CarriedOver>, String> {
        if let Some(state) = &self.skakel {
            return Ok(Some(CarriedOver {
                state: state.clone(),
                dropped: Vec::new(),
            }));
        }
        let (Some(signing_key), Some(pfx)) = (&self.signing_key, &self.certificate_pfx) else {
            return Ok(None);
        };
        use base64::Engine;
        let key = base64::engine::general_purpose::STANDARD
            .decode(signing_key)
            .map_err(|e| format!("the signing key is not base64 ({e})"))?;
        ring::rsa::KeyPair::from_pkcs8(&key)
            .map_err(|e| format!("the signing key is not an RSA PKCS#8 key ({e})"))?;
        crate::certificate::ServerCertificate::from_pfx_base64(pfx)
            .map_err(|e| format!("the certificate cannot be read ({e})"))?;

        let (kept, dropped): (Vec<LegacyClient>, Vec<LegacyClient>) = self
            .clients
            .clone()
            .unwrap_or_default()
            .into_iter()
            .partition(|c| c.secret_hash.starts_with("sha256$"));
        let state = SkakelState {
            clients: kept
                .iter()
                .map(|c| SkakelClientRecord {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    secret_hash: c.secret_hash.clone(),
                    created_at: c.created_at,
                    generation: c.generation,
                    secret: c.secret.clone(),
                })
                .collect(),
            signing_key: Some(SkakelKeyRecord {
                key_id: self
                    .signing_key_id
                    .clone()
                    .unwrap_or_else(|| "koppel-1".into()),
                private_key: signing_key.clone(),
                created_at: self.signing_key_since.unwrap_or(now),
                accept_until: None,
            }),
            retired_signing_keys: Vec::new(),
            certificate_pfx: pfx.clone(),
            next_certificate: self.next_certificate.as_ref().map(|next| {
                SkakelNextCertificateRecord {
                    pfx: next.pfx.clone(),
                    prepared_at: next.prepared_at,
                    received_by: next
                        .received_by
                        .iter()
                        .filter(|id| kept.iter().any(|c| &c.id == *id))
                        .cloned()
                        .collect(),
                }
            }),
        };
        Ok(Some(CarriedOver { state, dropped }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Result<KoppelSettings, String> {
        json::from_slice(text.as_bytes()).map_err(|e| e.to_string())
    }

    // Each case is one that System.Text.Json with Koppel's options was run on, and its answer.
    #[test]
    fn settings_are_read_as_dotnet_reads_them() {
        assert_eq!(settings(r#"{"role":"client"}"#).unwrap().role, Role::Client);
        let shouted = settings(r#"{"Role":"Client","ShareOnStartup":true}"#).unwrap();
        assert_eq!(
            (shouted.role, shouted.share_on_startup),
            (Role::Client, true)
        );
        assert_eq!(settings(r#"{"role":"CLIENT"}"#).unwrap().role, Role::Client);
        assert_eq!(settings(r#"{"role":"Both"}"#).unwrap().role, Role::Both);
        assert_eq!(
            settings(r#"{"role":" client"}"#).unwrap().role,
            Role::Client
        );
        assert_eq!(
            settings(r#"{"role":"server, client"}"#).unwrap().role,
            Role::Client
        );
        assert_eq!(
            settings(r#"{"role":"client","role":"both"}"#).unwrap().role,
            Role::Both
        );
        assert_eq!(
            settings(r#"{"role":"client","extra":5}"#).unwrap().role,
            Role::Client
        );
        let empty = settings("{}").unwrap();
        assert_eq!(
            empty,
            KoppelSettings {
                role: Role::Server,
                share_on_startup: true,
                setup_complete: false,
                notify_pairing: true,
                notify_link: true,
                notify_attach_failures: true
            }
        );
        assert_eq!(empty, KoppelSettings::default());
    }

    #[test]
    fn enums_as_numbers_and_other_bad_values_are_refused() {
        for refused in [
            r#"{"role":1}"#,
            r#"{"role":"1"}"#,
            r#"{"role":null}"#,
            r#"{"role":"client, both"}"#,
            r#"{"shareOnStartup":"true"}"#,
            r#"{"role":"client",}"#,
            r#"{"role":"client"} // x"#,
        ] {
            assert!(settings(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn computed_properties_are_written_and_ignored_when_read() {
        let connection = ClientConnection {
            server_url: Some("https://a:5240".into()),
            server_fingerprint: Some("AA".into()),
            client_id: Some("client-1".into()),
            client_secret: Some("s".into()),
            next_server_fingerprint: None,
        };
        let written = String::from_utf8(json::to_vec(&connection).unwrap()).unwrap();
        assert!(written.contains("\"isComplete\": true"));
        let back: ClientConnection = json::from_slice(written.as_bytes()).unwrap();
        assert_eq!(back, connection);

        let written = String::from_utf8(json::to_vec(&UiCredentials::empty()).unwrap()).unwrap();
        assert!(written.contains("\"hasPassword\": false"));
    }

    /// Every secret in every file struct is a sentinel here; none may come out of `{:?}`, plain or
    /// pretty, however deep the struct is nested.
    #[test]
    fn formatting_a_config_struct_shows_no_secret() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let state = SkakelState {
            clients: vec![SkakelClientRecord {
                id: "client-1".into(),
                name: "bench".into(),
                secret_hash: "sha256$AAAA".into(),
                created_at: now,
                generation: 1,
                secret: Some("SENTINEL-CLIENT-SECRET".into()),
            }],
            signing_key: SkakelKeyRecord {
                key_id: "k1".into(),
                private_key: "SENTINEL-SIGNING-KEY".into(),
                created_at: now,
                accept_until: None,
            },
            retired_signing_keys: vec![SkakelKeyRecord {
                key_id: "k0".into(),
                private_key: "SENTINEL-RETIRED-KEY".into(),
                created_at: now,
                accept_until: None,
            }],
            certificate_pfx: "SENTINEL-PFX".into(),
            next_certificate: Some(SkakelNextCertificateRecord {
                pfx: "SENTINEL-NEXT-PFX".into(),
                prepared_at: now,
                received_by: Vec::new(),
            }),
        };
        let legacy = ServerSecrets {
            skakel: Some(state.clone()),
            signing_key: Some("SENTINEL-LEGACY-KEY".into()),
            certificate_pfx: Some("SENTINEL-LEGACY-PFX".into()),
            clients: Some(vec![LegacyClient {
                id: "c".into(),
                name: "n".into(),
                secret_hash: "pbkdf2".into(),
                created_at: now,
                secret: Some("SENTINEL-LEGACY-SECRET".into()),
                generation: 1,
            }]),
            signing_key_id: None,
            signing_key_since: None,
            next_certificate: Some(LegacyNextCertificate {
                pfx: "SENTINEL-LEGACY-NEXT".into(),
                prepared_at: now,
                received_by: Vec::new(),
            }),
        };
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

        let shown = [
            format!(
                "{state:?}
{state:#?}"
            ),
            format!(
                "{legacy:?}
{legacy:#?}"
            ),
            format!(
                "{connection:?}
{connection:#?}"
            ),
            format!(
                "{credentials:?}
{credentials:#?}"
            ),
        ]
        .join(
            "
",
        );

        assert!(!shown.contains("SENTINEL"), "{shown}");
        assert!(shown.contains("Secret(<redacted>)"));
        // Redacted in memory only: the files still hold the values.
        let written = String::from_utf8(json::to_vec(&connection).unwrap()).unwrap();
        assert!(written.contains("SENTINEL-LINK-SECRET"));
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

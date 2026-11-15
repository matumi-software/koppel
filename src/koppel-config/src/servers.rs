//! The servers a client attaches from, as `connection.json` and `attached.json` hold them once a client
//! can have several (`docs/several-servers.md`).
//!
//! Both files used to describe one server, in flat fields. A flat file reads as a list of one, whose id is
//! [`FIRST_SERVER`], and is written as a list the next time it is saved. Nothing in the old shape is
//! lost, and nothing in the new shape is needed to read the old one.

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use koppel_secrets::Secret;

use crate::files::{AttachedDevices, ClientConnection};

/// The id the server of an old, flat file gets. A server added later gets an id of its own.
pub const FIRST_SERVER: &str = "server-1";

/// Whether a link tries again after it drops: on unless a person turns it off.
pub const DEFAULT_RECONNECT: bool = true;

/// How often a link that dropped tries again, in seconds.
pub const DEFAULT_RETRY_EVERY_SECONDS: u32 = 30;

/// How long one attempt to connect may take before it is given up, in seconds.
pub const DEFAULT_GIVE_UP_AFTER_SECONDS: u32 = 10;

fn reconnect() -> bool {
    DEFAULT_RECONNECT
}

fn retry_every() -> u32 {
    DEFAULT_RETRY_EVERY_SECONDS
}

fn give_up_after() -> u32 {
    DEFAULT_GIVE_UP_AFTER_SECONDS
}

/// One server this client attaches from: where it is, the certificate pinned for it, the credentials the
/// server gave and how the link behaves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    /// Koppel's own name for it, such as `server-3fa9c01b2e`. It names the server's part of
    /// `attached.json` and the routes that act on it.
    pub id: String,
    /// What the server calls itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// How it was linked: `pairing`, `code` or `key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_with: Option<String>,
    /// Try again after the link drops.
    #[serde(default = "reconnect")]
    pub reconnect: bool,
    /// Seconds between tries, while `reconnect` is on.
    #[serde(default = "retry_every")]
    pub retry_every_seconds: u32,
    /// Seconds one try may take.
    #[serde(default = "give_up_after")]
    pub give_up_after_seconds: u32,
}

impl ServerEntry {
    /// An entry with no server in it yet, and the link settings a new one gets.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            server_url: None,
            server_fingerprint: None,
            client_id: None,
            client_secret: None,
            next_server_fingerprint: None,
            linked_with: None,
            reconnect: DEFAULT_RECONNECT,
            retry_every_seconds: DEFAULT_RETRY_EVERY_SECONDS,
            give_up_after_seconds: DEFAULT_GIVE_UP_AFTER_SECONDS,
        }
    }

    /// Every part needed to link is there.
    pub fn is_complete(&self) -> bool {
        self.server_url.is_some()
            && self.server_fingerprint.is_some()
            && self.client_id.is_some()
            && self.client_secret.is_some()
    }

    /// The entry an old, flat connection makes.
    pub fn from_connection(id: impl Into<String>, connection: &ClientConnection) -> Self {
        Self {
            server_url: connection.server_url.clone(),
            server_fingerprint: connection.server_fingerprint.clone(),
            client_id: connection.client_id.clone(),
            client_secret: connection.client_secret.clone(),
            next_server_fingerprint: connection.next_server_fingerprint.clone(),
            ..Self::new(id)
        }
    }

    /// This server as the single connection the links and the UI of one server work from.
    pub fn connection(&self) -> ClientConnection {
        ClientConnection {
            server_url: self.server_url.clone(),
            server_fingerprint: self.server_fingerprint.clone(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            next_server_fingerprint: self.next_server_fingerprint.clone(),
        }
    }
}

/// `connection.json` (client role): every server this client attaches from, in the order they were added.
/// Encrypted. It reads the flat file of one server as a list of one, and always writes a list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientServers {
    pub servers: Vec<ServerEntry>,
}

impl ClientServers {
    pub fn find(&self, id: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|server| server.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut ServerEntry> {
        self.servers.iter_mut().find(|server| server.id == id)
    }

    /// An id no server here has, from `make` (which should make a new one each time).
    pub fn free_id(&self, mut make: impl FnMut() -> String) -> String {
        loop {
            let id = make();
            if self.find(&id).is_none() {
                return id;
            }
        }
    }
}

#[derive(Serialize)]
struct Written<'a> {
    servers: &'a [ServerEntry],
}

impl Serialize for ClientServers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Written {
            servers: &self.servers,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ClientServers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("connection.json is not an object"))?;
        if let Some(list) = object.get("servers") {
            let list = if list.is_null() {
                Value::Array(Vec::new())
            } else {
                list.clone()
            };
            let servers: Vec<ServerEntry> =
                serde_json::from_value(list).map_err(D::Error::custom)?;
            return Ok(Self { servers });
        }
        // The flat file of one server: no `servers`. An empty object is no server at all.
        let connection: ClientConnection =
            serde_json::from_value(value).map_err(D::Error::custom)?;
        let anything = connection.server_url.is_some()
            || connection.server_fingerprint.is_some()
            || connection.client_id.is_some()
            || connection.client_secret.is_some();
        Ok(Self {
            servers: if anything {
                vec![ServerEntry::from_connection(FIRST_SERVER, &connection)]
            } else {
                Vec::new()
            },
        })
    }
}

/// What one server's devices were remembered as: the bus IDs this client attached and the VID:PID each had.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedFromServer {
    #[serde(default, deserialize_with = "crate::files::or_default")]
    pub bus_ids: Vec<String>,
    #[serde(default, deserialize_with = "crate::files::or_default")]
    pub vid_pids: IndexMap<String, String>,
}

/// `attached.json` (client role): the devices this client attached, by the server they came from, since
/// two servers can offer the same bus ID. Plain. It reads the flat file of one server (with the
/// fingerprint it was made for) as the part of [`FIRST_SERVER`], and always writes the list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedByServer {
    pub servers: IndexMap<String, AttachedFromServer>,
    /// The fingerprint a flat file recorded, so the first server is only matched to it if it is the
    /// same server. Kept until the first save after the server was matched, then left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_fingerprint: Option<String>,
}

impl<'de> Deserialize<'de> for AttachedByServer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("attached.json is not an object"))?;
        if let Some(list) = object.get("servers") {
            let servers: IndexMap<String, AttachedFromServer> = if list.is_null() {
                IndexMap::new()
            } else {
                serde_json::from_value(list.clone()).map_err(D::Error::custom)?
            };
            let legacy_fingerprint = object
                .get("legacyFingerprint")
                .and_then(Value::as_str)
                .map(str::to_owned);
            return Ok(Self {
                servers,
                legacy_fingerprint,
            });
        }
        let flat: AttachedDevices = serde_json::from_value(value).map_err(D::Error::custom)?;
        let mut servers = IndexMap::new();
        if !flat.bus_ids.is_empty() || !flat.vid_pids.is_empty() {
            servers.insert(
                FIRST_SERVER.to_owned(),
                AttachedFromServer {
                    bus_ids: flat.bus_ids,
                    vid_pids: flat.vid_pids,
                },
            );
        }
        Ok(Self {
            servers,
            legacy_fingerprint: flat.server_fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_connection_reads_as_one_server_and_is_written_as_a_list() {
        let servers: ClientServers = serde_json::from_str(
            r#"{"serverUrl":"https://a:5240","serverFingerprint":"AA","clientId":"client-1","clientSecret":"s","nextServerFingerprint":"BB"}"#,
        )
        .unwrap();

        assert_eq!(servers.servers.len(), 1);
        let first = &servers.servers[0];
        assert_eq!(first.id, FIRST_SERVER);
        assert_eq!(first.server_url.as_deref(), Some("https://a:5240"));
        assert_eq!(first.next_server_fingerprint.as_deref(), Some("BB"));
        assert!(first.is_complete());
        assert!(first.reconnect);
        assert_eq!(first.retry_every_seconds, DEFAULT_RETRY_EVERY_SECONDS);
        let written = serde_json::to_string(&servers).unwrap();
        assert!(written.starts_with(r#"{"servers":["#), "{written}");
        assert!(!written.contains(r#""serverUrl":"https://a:5240","serverFingerprint":"AA","clientId":"client-1","clientSecret":"s"}"#), "{written}");
        let back: ClientServers = serde_json::from_str(&written).unwrap();
        assert_eq!(back, servers);
    }

    #[test]
    fn an_empty_or_unset_connection_is_no_server() {
        for text in [
            "{}",
            r#"{"isComplete":false}"#,
            r#"{"servers":null}"#,
            r#"{"servers":[]}"#,
        ] {
            let servers: ClientServers = serde_json::from_str(text).unwrap();
            assert!(servers.servers.is_empty(), "{text}");
        }
    }

    #[test]
    fn several_servers_keep_their_order_and_their_link_settings() {
        let mut one = ServerEntry::new("server-a");
        one.server_url = Some("https://a:5240".into());
        let mut two = ServerEntry::new("server-b");
        two.reconnect = false;
        two.retry_every_seconds = 5;
        two.give_up_after_seconds = 3;
        two.linked_with = Some("key".into());
        let servers = ClientServers {
            servers: vec![one, two],
        };

        let back: ClientServers =
            serde_json::from_str(&serde_json::to_string(&servers).unwrap()).unwrap();

        assert_eq!(back, servers);
        let second = back.find("server-b").unwrap();
        assert!(!second.reconnect);
        assert_eq!(
            (second.retry_every_seconds, second.give_up_after_seconds),
            (5, 3)
        );
    }

    #[test]
    fn a_secret_is_written_to_the_file_and_never_shown() {
        let mut entry = ServerEntry::new("server-a");
        entry.client_secret = Some("SENTINEL-LINK-SECRET".into());
        let servers = ClientServers {
            servers: vec![entry],
        };

        assert!(!format!("{servers:?}\n{servers:#?}").contains("SENTINEL"));
        assert!(
            serde_json::to_string(&servers)
                .unwrap()
                .contains("SENTINEL-LINK-SECRET")
        );
    }

    #[test]
    fn a_free_id_is_one_no_server_has() {
        let servers = ClientServers {
            servers: vec![ServerEntry::new("a"), ServerEntry::new("b")],
        };
        let mut made = ["a", "b", "c"].into_iter();

        assert_eq!(servers.free_id(|| made.next().unwrap().to_owned()), "c");
    }

    #[test]
    fn a_flat_attached_file_becomes_the_first_servers_part_with_its_fingerprint() {
        let attached: AttachedByServer = serde_json::from_str(
            r#"{"serverFingerprint":"AA","busIds":["1-1"],"vidPids":{"1-1":"1209:0001"}}"#,
        )
        .unwrap();

        let first = attached.servers.get(FIRST_SERVER).unwrap();
        assert_eq!(first.bus_ids, ["1-1"]);
        assert_eq!(
            first.vid_pids.get("1-1").map(String::as_str),
            Some("1209:0001")
        );
        assert_eq!(attached.legacy_fingerprint.as_deref(), Some("AA"));
        let back: AttachedByServer =
            serde_json::from_str(&serde_json::to_string(&attached).unwrap()).unwrap();
        assert_eq!(back, attached);
    }

    #[test]
    fn attached_devices_of_two_servers_with_the_same_bus_id_stay_apart() {
        let mut attached = AttachedByServer::default();
        for (id, vid_pid) in [("server-a", "1209:0001"), ("server-b", "046d:c52b")] {
            let mut part = AttachedFromServer::default();
            part.bus_ids.push("1-1".into());
            part.vid_pids.insert("1-1".into(), vid_pid.into());
            attached.servers.insert(id.into(), part);
        }

        let back: AttachedByServer =
            serde_json::from_str(&serde_json::to_string(&attached).unwrap()).unwrap();

        assert_eq!(back.servers["server-a"].vid_pids["1-1"], "1209:0001");
        assert_eq!(back.servers["server-b"].vid_pids["1-1"], "046d:c52b");
        assert_eq!(back.legacy_fingerprint, None);
    }

    #[test]
    fn nulls_where_a_list_belongs_read_as_empty() {
        let attached: AttachedByServer =
            serde_json::from_str(r#"{"servers":{"server-a":{"busIds":null,"vidPids":null}}}"#)
                .unwrap();

        assert!(attached.servers["server-a"].bus_ids.is_empty());
    }
}

//! What the REST endpoints and the push talk to, with 0.1.x's JSON shapes, and what fills them:
//!
//! - [`SkakelServerRole`]: the server's registered clients, its certificate, the pairing window and
//!   pairing requests, over skakel-server and `secrets.json`.
//! - [`SkakelClientRole`]: the servers found on the network, with skakel-client.
//! - [`Servers`]: the servers a client attaches from: adding one with a code or a key, removing it,
//!   its link settings and its connection test.
//! - [`UsbServerRole`] and [`UsbClientRole`]: koppel-usb's server and client with the push and the
//!   system checks. Their views are koppel-usb's own types, serialised as they are.
//!
//! A role pushes its own changes through [`crate::push::Push`] once they are saved.

pub mod skakel;
mod usb;
pub mod usb_store;

use serde::Serialize;
use time::OffsetDateTime;
use ts_rs::TS;

pub use crate::servers::{AddServer, ServerSettings, Servers, ServersError, ServersParts};
pub use skakel::{SkakelClientRole, SkakelServerRole};
pub use usb::UsbActivity;
pub use usb::{UsbClientRole, UsbServerRole};

/// A change that could not be saved. The caller gets 500 and the log gets the whole chain.
#[derive(Debug, thiserror::Error)]
pub enum RoleError {
    /// Skakel's server refused or could not write the change.
    #[error(transparent)]
    Server(#[from] skakel_server::ServerError),
    /// Skakel's server has stopped with Koppel.
    #[error("Skakel's server is not running")]
    NotRunning,
}

/// One registered client, as the server's UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub id: String,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[ts(type = "string | null")]
    pub last_seen: Option<OffsetDateTime>,
    /// It has the fingerprint of the certificate prepared for a handover.
    pub has_next_certificate: bool,
    /// Its secret is kept, so it can be copied again.
    pub has_secret: bool,
    /// How it was linked: `pairing`, `code` or `key`. Empty for a client made before that was kept.
    #[serde(default)]
    pub linked_with: Option<String>,
}

/// A pairing code the server is showing: eight digits that work once, until `expires_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentCodeView {
    pub code: String,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    pub expires_at: OffsetDateTime,
}

/// What the server shows for enrolling a client without approving it: the pairing code, if one is
/// showing, and how many times the server key has been rotated since the server started, so the UI
/// can tell a rotation it asked for has happened. The key itself is not pushed: the UI asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentView {
    pub code: Option<EnrollmentCodeView>,
    pub key_revision: u32,
}

impl EnrollmentView {
    /// No code showing: a server that has just started.
    pub fn empty() -> Self {
        Self {
            code: None,
            key_revision: 0,
        }
    }
}

/// The server key, as the text a person copies to another computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentKey {
    pub key: String,
}

/// A client just added, with its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct NewClient {
    pub client_id: String,
    pub client_secret: String,
    pub name: String,
}

/// The server's certificate, and the next one while a handover is under way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct KeysView {
    pub fingerprint: String,
    pub next_fingerprint: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[ts(type = "string | null")]
    pub next_prepared_at: Option<OffsetDateTime>,
}

/// The server switched to its prepared certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    pub to: String,
    /// The names of the clients that had not said they have it.
    pub not_received: Vec<String>,
}

/// Whether the server takes new pairing requests. Skakel 0.3.0 takes them only while a person at
/// the server has opened a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct PairingWindowView {
    pub open: bool,
    /// The server takes requests at any time (not something Koppel sets; kept so the UI need not
    /// guess if a build ever does).
    pub always_open: bool,
    /// When the window closes by itself. `None` when it is closed.
    #[serde(with = "time::serde::rfc3339::option")]
    #[ts(type = "string | null")]
    pub until: Option<OffsetDateTime>,
}

impl PairingWindowView {
    pub fn closed() -> Self {
        Self {
            open: false,
            always_open: false,
            until: None,
        }
    }
}

/// The pairing window and the requests waiting on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct PairingOverview {
    pub requests: Vec<PairingRequestView>,
    /// The server had its fill of requests for now and turns new ones away.
    pub budget_exhausted: bool,
    pub window: PairingWindowView,
}

impl PairingOverview {
    /// No requests and the window closed: a server that has just started.
    pub fn empty() -> Self {
        Self {
            requests: Vec::new(),
            budget_exhausted: false,
            window: PairingWindowView::closed(),
        }
    }
}

/// One pairing request on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct PairingRequestView {
    pub id: String,
    pub client_name: String,
    pub from: String,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    pub requested_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    pub expires_at: OffsetDateTime,
    pub state: RequestState,
    pub message: Option<String>,
}

/// Where a pairing request on the server stands. Skakel 0.3.0 words it as `waiting` or `denied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub enum RequestState {
    Waiting,
    Denied,
}

impl RequestState {
    /// The state Skakel's word stands for. Skakel documents two; a word added later reads as
    /// waiting, which keeps the request on the list for a person to decide.
    pub fn of(word: &str) -> Self {
        match word {
            "denied" => Self::Denied,
            _ => Self::Waiting,
        }
    }
}

/// What approving a pairing request came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Approved,
    WrongCode,
    NotFound,
    /// The approval could not be saved. The request stays, so the person can try again.
    Failed,
}

/// Why a client could not be added.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// Nothing is left of the name once control and format characters are taken out.
    #[error("the name is empty")]
    EmptyName,
    #[error("the client could not be saved")]
    Failed(#[source] RoleError),
}

/// How a connection test ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Ok,
    Incomplete,
    Unreachable,
    CertificateMismatch,
    RejectedCredentials,
    ClientUnknown,
    VersionMismatch,
    UnexpectedAnswer,
    TooManyRequests,
}

impl Outcome {
    /// The fixed wording of 0.1.x. A version mismatch has none of its own: it always carries
    /// Skakel's words, which name both sides' versions.
    pub fn message(self) -> &'static str {
        match self {
            Self::Ok => "Connected.",
            Self::Incomplete => {
                "Fill in the server address, fingerprint, client ID and secret first."
            }
            Self::Unreachable => {
                "Could not reach the server. Check the address and that the server is running."
            }
            Self::CertificateMismatch => {
                "The server's certificate does not match the fingerprint. Copy the fingerprint from the server's Clients section again."
            }
            Self::RejectedCredentials => {
                "The server refused the client ID or secret. Check them, or ask for a new client."
            }
            Self::UnexpectedAnswer => {
                "The server did not answer like a Koppel server. Check the address and the port."
            }
            Self::TooManyRequests => {
                "The server is turning requests away for now, from this address or, after too many wrong secrets, for this client. Try again in a few minutes."
            }
            Self::ClientUnknown | Self::VersionMismatch => {
                "The server accepted the token but no longer knows this client. Ask for a new client."
            }
        }
    }
}

/// How a connection test ended: a fixed set, with fixed wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTest {
    pub ok: bool,
    pub outcome: Outcome,
    pub message: String,
}

impl ConnectionTest {
    /// The outcome with its own words.
    pub fn of(outcome: Outcome) -> Self {
        Self::saying(outcome, outcome.message())
    }

    /// The outcome with other words, such as Skakel's.
    pub fn saying(outcome: Outcome, message: impl Into<String>) -> Self {
        Self {
            ok: outcome == Outcome::Ok,
            outcome,
            message: message.into(),
        }
    }
}

/// A server found on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredServer {
    pub name: String,
    pub address: String,
    pub https_port: u16,
    pub fingerprint: String,
    pub version: Option<String>,
    pub other_addresses: Vec<String>,
    /// The address as a URL, as 0.1.x also wrote it.
    pub url: String,
}

/// One server this client attaches from, as the UI lists it. Never the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    /// Koppel's id for the server, which names it in the routes.
    pub id: String,
    /// What the server calls itself.
    pub name: Option<String>,
    pub server_url: Option<String>,
    pub server_fingerprint: Option<String>,
    pub client_id: Option<String>,
    pub has_secret: bool,
    pub next_server_fingerprint: Option<String>,
    /// How it was linked: `pairing`, `code` or `key`.
    pub linked_with: Option<String>,
    /// Try again after the link drops.
    pub reconnect: bool,
    /// Seconds between tries while `reconnect` is on.
    pub retry_every_seconds: u32,
    /// Seconds one try may take.
    pub give_up_after_seconds: u32,
}

impl ServerView {
    pub fn of(entry: &koppel_config::ServerEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            server_url: entry.server_url.clone(),
            server_fingerprint: entry.server_fingerprint.clone(),
            client_id: entry.client_id.clone(),
            has_secret: entry.client_secret.is_some(),
            next_server_fingerprint: entry.next_server_fingerprint.clone(),
            linked_with: entry.linked_with.clone(),
            reconnect: entry.reconnect,
            retry_every_seconds: entry.retry_every_seconds,
            give_up_after_seconds: entry.give_up_after_seconds,
        }
    }
}

/// Every server this client attaches from, in the order they were added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[cfg_attr(any(test, feature = "test-util"), derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct ServersView {
    pub servers: Vec<ServerView>,
}

impl ServersView {
    pub fn of(servers: &koppel_config::ClientServers) -> Self {
        Self {
            servers: servers.servers.iter().map(ServerView::of).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word<T: Serialize>(value: T) -> String {
        serde_json::to_value(value)
            .expect("serialises")
            .as_str()
            .expect("a word")
            .to_owned()
    }

    #[test]
    fn the_wire_words_are_0_1_xs() {
        let outcomes = [
            (Outcome::Ok, "ok"),
            (Outcome::Incomplete, "incomplete"),
            (Outcome::Unreachable, "unreachable"),
            (Outcome::CertificateMismatch, "certificateMismatch"),
            (Outcome::RejectedCredentials, "rejectedCredentials"),
            (Outcome::ClientUnknown, "clientUnknown"),
            (Outcome::VersionMismatch, "versionMismatch"),
            (Outcome::UnexpectedAnswer, "unexpectedAnswer"),
            (Outcome::TooManyRequests, "tooManyRequests"),
        ];
        for (outcome, text) in outcomes {
            assert_eq!(word(outcome), text);
        }
        assert_eq!(word(RequestState::Waiting), "waiting");
        assert_eq!(word(RequestState::Denied), "denied");
        assert_eq!(RequestState::of("denied"), RequestState::Denied);
        assert_eq!(RequestState::of("anything else"), RequestState::Waiting);
    }

    #[test]
    fn only_ok_is_ok_and_the_words_are_fixed() {
        let ok = ConnectionTest::of(Outcome::Ok);
        assert!(ok.ok);
        assert_eq!(ok.message, "Connected.");
        let mismatch = ConnectionTest::saying(Outcome::VersionMismatch, "Skakel's words");
        assert!(!mismatch.ok);
        assert_eq!(mismatch.message, "Skakel's words");
        assert!(!ConnectionTest::of(Outcome::Unreachable).ok);
    }

    #[test]
    fn a_register_error_keeps_its_source() {
        use std::error::Error;
        let error = RegisterError::Failed(RoleError::Server(skakel_server::ServerError::Store(
            std::io::Error::other("disk"),
        )));
        assert!(error.source().is_some());
    }
}

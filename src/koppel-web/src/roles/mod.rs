//! The seams the REST endpoints and the push talk to, with 0.1.x's JSON shapes, and what fills them:
//!
//! - [`ServerRole`]: the server's registered clients, its certificate, the pairing window and
//!   pairing requests. [`SkakelServerRole`] runs skakel-server over `secrets.json`.
//! - [`ClientRole`]: the client's connection test, the servers found on the network and pairing.
//!   [`SkakelClientRole`] does them with skakel-client.
//! - [`UsbServer`] and [`UsbClient`]: the devices, sharing, attaching and detaching, and the
//!   client's link. [`UsbServerRole`] and [`UsbClientRole`] are koppel-usb's server and client,
//!   whose link to the server is Skakel's (`skakel::SkakelLinks`).
//!
//! An implementation pushes its own changes through [`crate::push::Push`] once they are saved.

pub mod skakel;
mod usb;

use std::future::Future;
use std::pin::Pin;

use serde::Serialize;
use time::OffsetDateTime;

pub use skakel::{SkakelClientRole, SkakelServerRole};
pub(crate) use usb::UsbActivity;
pub use usb::{UsbClientRole, UsbServerRole};

/// A boxed future, so the traits can be used as `dyn`.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A change that could not be saved, in a few words for the log. The caller gets 500.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleError(pub String);

impl std::fmt::Display for RoleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RoleError {}

/// One registered client, as the server's UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub id: String,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_seen: Option<OffsetDateTime>,
    /// It has the fingerprint of the certificate prepared for a handover.
    pub has_next_certificate: bool,
    /// Its secret is kept, so it can be copied again.
    pub has_secret: bool,
}

/// A client just added, with its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewClient {
    pub client_id: String,
    pub client_secret: String,
    pub name: String,
}

/// The server's certificate, and the next one while a handover is under way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysView {
    pub fingerprint: String,
    pub next_fingerprint: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingWindowView {
    pub open: bool,
    /// The server takes requests at any time (not something Koppel sets; kept so the UI need not
    /// guess if a build ever does).
    pub always_open: bool,
    /// When the window closes by itself. `None` when it is closed.
    #[serde(with = "time::serde::rfc3339::option")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequestView {
    pub id: String,
    pub client_name: String,
    pub from: String,
    #[serde(with = "time::serde::rfc3339")]
    pub requested_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub state: String,
    pub message: Option<String>,
}

/// What approving a pairing request came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Approved,
    WrongCode,
    NotFound,
}

/// Why a client could not be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// Nothing is left of the name once control and format characters are taken out.
    EmptyName,
    Failed(RoleError),
}

/// The server role: Skakel's clients, certificates and pairing. See the module docs.
pub trait ServerRole: Send + Sync {
    /// The SHA-256 fingerprint of the certificate presented now.
    fn fingerprint(&self) -> String;
    fn clients(&self) -> Vec<ClientView>;
    /// A client's name, or `None` when there is no such client.
    fn name_of(&self, id: &str) -> Option<String>;
    /// `None` when there is no such client; `Some(None)` when its secret was not kept.
    fn secret_of(&self, id: &str) -> Option<Option<String>>;
    fn register<'a>(&'a self, name: &'a str) -> Fut<'a, Result<NewClient, RegisterError>>;
    /// False when there was no such client.
    fn revoke<'a>(&'a self, id: &'a str) -> Fut<'a, Result<bool, RoleError>>;
    /// The new secret, or `None` when there was no such client.
    fn rotate_secret<'a>(&'a self, id: &'a str) -> Fut<'a, Result<Option<String>, RoleError>>;
    fn keys(&self) -> KeysView;
    /// The new certificate's fingerprint, or `None` when one is already prepared.
    fn prepare_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>>;
    /// The cancelled certificate's fingerprint, or `None` when none was prepared.
    fn cancel_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>>;
    /// `None` when there is no prepared certificate to switch to.
    fn switch_certificate(&self) -> Fut<'_, Result<Option<Switched>, RoleError>>;
    fn pairing(&self) -> PairingOverview;
    /// Takes new pairing requests for `duration` from now (Skakel holds it between one minute and
    /// one hour). The window shows in [`ServerRole::pairing`] and over the push once Skakel's
    /// server says it is open.
    fn open_pairing(&self, duration: std::time::Duration);
    /// Closes the window now.
    fn close_pairing(&self);
    fn approve<'a>(&'a self, id: &'a str, code: &'a str) -> Fut<'a, Approval>;
    /// False when there was no such request.
    fn deny<'a>(&'a self, id: &'a str) -> Fut<'a, bool>;
}

/// How a connection test ended: a fixed set, with fixed wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTest {
    pub ok: bool,
    /// `ok`, `incomplete`, `unreachable`, `certificateMismatch`, `rejectedCredentials`,
    /// `clientUnknown`, `versionMismatch`, `unexpectedAnswer` or `tooManyRequests`.
    pub outcome: String,
    pub message: String,
}

/// A server found on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// Where the client's pairing stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingStatus {
    /// `idle`, `requesting`, `waiting`, `confirm`, `paired`, `denied`, `expired` or `failed`.
    pub state: String,
    pub server_name: Option<String>,
    pub server_url: Option<String>,
    pub code: Option<String>,
    pub message: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    pub server_fingerprint: Option<String>,
}

impl PairingStatus {
    pub fn idle() -> Self {
        Self::of("idle", None)
    }

    pub fn of(state: &str, message: Option<String>) -> Self {
        Self {
            state: state.to_owned(),
            server_name: None,
            server_url: None,
            code: None,
            message,
            expires_at: None,
            server_fingerprint: None,
        }
    }
}

/// The client role: the link to a server, discovery and pairing. See the module docs. The stored
/// connection itself is `connection.json`, which the web side reads and writes.
pub trait ClientRole: Send + Sync {
    fn test(&self) -> Fut<'_, ConnectionTest>;
    fn discovered(&self) -> Vec<DiscoveredServer>;
    fn pairing(&self) -> PairingStatus;
    fn begin_pairing<'a>(
        &'a self,
        server_url: &'a str,
        fingerprint: &'a str,
        server_name: Option<&'a str>,
    ) -> Fut<'a, ()>;
    /// False when there is nothing to confirm.
    fn confirm_pairing(&self) -> Fut<'_, bool>;
    fn cancel_pairing(&self) -> Fut<'_, ()>;
}

/// A USB device as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub bus_id: String,
    pub vid_pid: String,
    pub name: String,
    pub manufacturer: Option<String>,
    pub speed: String,
    /// `local`, `shared`, `attached` or `offline`.
    pub state: String,
    pub attached_to: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub attached_since: Option<OffsetDateTime>,
    pub fault: Option<String>,
    pub serial: Option<String>,
    pub driver: Option<String>,
    pub interfaces: Option<Vec<String>>,
}

/// What the server's UI shows: its devices, or why it cannot share any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDevices {
    pub devices: Vec<DeviceView>,
    pub unavailable: Option<String>,
}

/// A remote device as the client's UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDeviceView {
    pub device: DeviceView,
    pub attached_here: bool,
    pub port: Option<i32>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub attached_since: Option<OffsetDateTime>,
    pub fault: Option<String>,
    pub attached_via: Option<String>,
}

/// What the client's UI shows: the link to the server and the devices it shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDevices {
    /// `unconfigured`, `connecting`, `connected` or `error`.
    pub link: String,
    pub link_message: Option<String>,
    pub devices: Vec<ClientDeviceView>,
    pub unavailable: Option<String>,
    pub gateway: bool,
    pub gateway_fault: Option<String>,
}

/// One thing Koppel needs from this machine, for the first-run checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemCheck {
    pub id: String,
    pub ok: bool,
    pub title: String,
    pub detail: String,
}

impl SystemCheck {
    pub fn new(id: &str, ok: bool, title: &str, detail: &str) -> Self {
        Self {
            id: id.to_owned(),
            ok,
            title: title.to_owned(),
            detail: detail.to_owned(),
        }
    }
}

/// The server's USB side (#86 on Linux, #87 on Windows).
pub trait UsbServer: Send + Sync {
    fn current(&self) -> ServerDevices;
    /// Applies a share choice; the outcome follows as `DevicesChanged`.
    fn set_shared<'a>(&'a self, bus_id: &'a str, shared: bool) -> Fut<'a, ()>;
    /// Whether this machine can share, for `/api/system/checks`.
    fn check(&self) -> SystemCheck;
    /// Why `devices.json` was not used, or `None`.
    fn problem(&self) -> Option<String>;
}

/// The client's USB side (#86 on Linux, #87 on Windows).
pub trait UsbClient: Send + Sync {
    fn current(&self) -> ClientDevices;
    /// Remembers the device and attaches it; the outcome follows as `ClientDevicesChanged`.
    fn attach<'a>(&'a self, bus_id: &'a str) -> Fut<'a, ()>;
    /// Detaches the device and forgets it.
    fn detach<'a>(&'a self, bus_id: &'a str) -> Fut<'a, ()>;
    /// Whether this machine can attach, for `/api/system/checks`.
    fn check(&self) -> SystemCheck;
    /// Why `attached.json` was not used, or `None`.
    fn problem(&self) -> Option<String>;
}

/// A kernel bus ID such as `1-1.4`: a bus number, a dash, then the port path, at most 31
/// characters, as 0.1.x's `UsbBusId` checks it.
pub fn is_bus_id(text: &str) -> bool {
    if text.is_empty() || text.len() > 31 {
        return false;
    }
    let Some((bus, ports)) = text.split_once('-') else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    digits(bus) && ports.split('.').all(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_ids_are_the_kernels_form() {
        for good in ["1-1", "1-1.4", "12-3.4.5", "0-0"] {
            assert!(is_bus_id(good), "{good}");
        }
        let too_long = format!("1-{}", "1".repeat(30));
        for bad in [
            "bogus", "1-", "a-1", "1-1.", "1-1:1.0", "-1", "1--1", "", &too_long,
        ] {
            assert!(!is_bus_id(bad), "{bad}");
        }
    }
}

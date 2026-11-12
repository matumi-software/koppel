//! The client role's endpoints: the stored connection and its test, the servers found on the
//! network, pairing, and the devices.

use axum::extract::{Path, State};
use axum::response::Response;
use koppel_config::files::ClientConnection;
use koppel_usb::BusId;
use serde::{Deserialize, Serialize};

use crate::json::{self, JsonBody};
use crate::state::ClientApi;

/// What the UI may see of the connection: never the secret, only whether one is stored.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionView {
    server_url: Option<String>,
    server_fingerprint: Option<String>,
    client_id: Option<String>,
    has_secret: bool,
    next_server_fingerprint: Option<String>,
}

impl ConnectionView {
    pub(crate) fn of(connection: &ClientConnection) -> Self {
        Self {
            server_url: connection.server_url.clone(),
            server_fingerprint: connection.server_fingerprint.clone(),
            client_id: connection.client_id.clone(),
            has_secret: connection.client_secret.is_some(),
            next_server_fingerprint: connection.next_server_fingerprint.clone(),
        }
    }
}

/// A change to the connection. A null or empty secret keeps the stored one, but only while the
/// server stays the same.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionUpdate {
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    server_fingerprint: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PairStart {
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    server_fingerprint: Option<String>,
    #[serde(default)]
    server_name: Option<String>,
}

const URL_PROBLEM: &str = "Use the server's HTTPS address, such as https://lab-rack-3:5240.";
const FINGERPRINT_PROBLEM: &str =
    "Paste the SHA-256 fingerprint shown in the server's Clients section.";

/// The scheme, host and port of an HTTPS address, as .NET's `GetLeftPart(UriPartial.Authority)`
/// gives it: lower case, and without the port when it is 443. `None` when it is not one.
fn https_authority(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .get(..8)
        .filter(|s| s.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(after) = host_port.strip_prefix('[') {
        let (v6, rest) = after.split_once(']')?;
        v6.parse::<std::net::Ipv6Addr>().ok()?;
        let port = match rest {
            "" => None,
            _ => Some(rest.strip_prefix(':')?),
        };
        (format!("[{}]", v6.to_ascii_lowercase()), port)
    } else {
        let (host, port) = match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        };
        let valid = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'));
        if !valid {
            return None;
        }
        (host.to_lowercase(), port)
    };
    let port = match port {
        None | Some("") => None,
        Some(port) => {
            let number: u16 = port.parse().ok()?;
            (number != 443).then_some(number)
        }
    };
    Some(match port {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
    })
}

/// A fingerprint's hex digits alone.
fn digits(fingerprint: Option<&str>) -> String {
    fingerprint
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect()
}

/// A SHA-256 fingerprint: 64 hex digits, with or without separators.
fn is_fingerprint(fingerprint: Option<&str>) -> bool {
    digits(fingerprint).len() == 64
}

fn same_fingerprint(a: Option<&str>, b: Option<&str>) -> bool {
    digits(a).eq_ignore_ascii_case(&digits(b))
}

/// The spelling Koppel stores: upper case hex in pairs, separated by colons.
pub(crate) fn normalize(fingerprint: &str) -> String {
    let hex = digits(Some(fingerprint)).to_ascii_uppercase();
    hex.as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

impl ConnectionUpdate {
    /// What a server approved by pairing: its address and fingerprint and the credentials it gave.
    pub(crate) fn paired(
        server_url: &str,
        fingerprint: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Self {
        Self {
            server_url: Some(server_url.to_owned()),
            server_fingerprint: Some(fingerprint.to_owned()),
            client_id: Some(client_id.to_owned()),
            client_secret: Some(client_secret.to_owned()),
        }
    }

    /// True while `saved` is the connection this update made: its client ID, address and
    /// fingerprint.
    pub(crate) fn matches(&self, saved: &ClientConnection) -> bool {
        let url = self.server_url.as_deref().and_then(https_authority);
        saved.client_id.as_deref() == self.client_id.as_deref().map(str::trim)
            && saved
                .server_url
                .as_deref()
                .zip(url.as_deref())
                .is_some_and(|(saved, url)| saved.eq_ignore_ascii_case(url))
            && same_fingerprint(
                saved.server_fingerprint.as_deref(),
                self.server_fingerprint.as_deref(),
            )
    }

    fn validate(&self) -> Vec<(&'static str, &'static str)> {
        let mut errors = Vec::new();
        if self
            .server_url
            .as_deref()
            .and_then(https_authority)
            .is_none()
        {
            errors.push(("serverUrl", URL_PROBLEM));
        }
        if !is_fingerprint(self.server_fingerprint.as_deref()) {
            errors.push(("serverFingerprint", FINGERPRINT_PROBLEM));
        }
        if self
            .client_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
        {
            errors.push(("clientId", "Enter the client ID the server gave you."));
        }
        errors
    }

    /// True when the update points at the server the connection already has: the same address, and
    /// its fingerprint or the next one that server announced.
    pub(crate) fn keeps_server(&self, current: &ClientConnection) -> bool {
        let url = self.server_url.as_deref().and_then(https_authority);
        let fingerprint = self.server_fingerprint.as_deref();
        current
            .server_url
            .as_deref()
            .zip(url.as_deref())
            .is_some_and(|(stored, url)| stored.eq_ignore_ascii_case(url))
            && (same_fingerprint(current.server_fingerprint.as_deref(), fingerprint)
                || (current.next_server_fingerprint.is_some()
                    && same_fingerprint(current.next_server_fingerprint.as_deref(), fingerprint)))
    }

    /// The stored secret was issued by the stored server, so it carries over only when the server
    /// is unchanged. For another server it never does.
    pub(crate) fn apply(&self, current: &ClientConnection) -> ClientConnection {
        let keeps = self.keeps_server(current);
        let fingerprint = self.server_fingerprint.as_deref();
        ClientConnection {
            server_url: self.server_url.as_deref().and_then(https_authority),
            server_fingerprint: fingerprint.map(normalize),
            client_id: self.client_id.as_deref().map(|id| id.trim().to_owned()),
            client_secret: match self.client_secret.as_deref() {
                Some(secret) if !secret.is_empty() => Some(secret.into()),
                _ if keeps => current.client_secret.clone(),
                _ => None,
            },
            // The next certificate belongs to the server that announced it, and is not needed once it
            // is the pinned one.
            next_server_fingerprint: if keeps
                && !same_fingerprint(current.next_server_fingerprint.as_deref(), fingerprint)
            {
                current.next_server_fingerprint.clone()
            } else {
                None
            },
        }
    }
}

pub(crate) async fn connection(State(app): State<ClientApi>) -> Response {
    json::ok(&ConnectionView::of(&app.connection.current()))
}

/// Answers 202 with no body; the stored connection reaches the UI as `ConnectionChanged` once
/// `connection.json` is written. The secret is never sent back.
pub(crate) async fn save_connection(
    State(app): State<ClientApi>,
    JsonBody(update): JsonBody<ConnectionUpdate>,
) -> Response {
    let errors = update.validate();
    if !errors.is_empty() {
        return json::problem(&errors);
    }
    if update.client_secret.as_deref().is_none_or(str::is_empty)
        && !update.keeps_server(&app.connection.current())
    {
        return json::problem(&[(
            "clientSecret",
            "Enter the client secret. The stored one only carries over while the server address and fingerprint stay the same.",
        )]);
    }
    if let Err(e) = app.connection.update(|current| update.apply(current)).await {
        return json::failed("Saving the connection", e);
    }
    json::accepted()
}

/// A query, so it answers with its result.
pub(crate) async fn test(State(app): State<ClientApi>) -> Response {
    json::ok(&app.client.test().await)
}

/// Servers found on the local network. The list also follows as `ServersDiscovered`.
pub(crate) async fn discovery(State(app): State<ClientApi>) -> Response {
    json::ok(&app.client.discovered())
}

pub(crate) async fn pairing(State(app): State<ClientApi>) -> Response {
    json::ok(&app.client.pairing())
}

/// Answers 202 and pairs in the background; `PairingStatusChanged` carries the code and the outcome.
pub(crate) async fn begin_pairing(
    State(app): State<ClientApi>,
    JsonBody(request): JsonBody<PairStart>,
) -> Response {
    let check = ConnectionUpdate {
        server_url: request.server_url.clone(),
        server_fingerprint: request.server_fingerprint.clone(),
        client_id: Some("pairing".into()),
        client_secret: None,
    };
    let errors = check.validate();
    if !errors.is_empty() {
        return json::problem(&errors);
    }
    let url = request
        .server_url
        .as_deref()
        .and_then(https_authority)
        .unwrap_or_default();
    app.client
        .begin_pairing(
            &url,
            request.server_fingerprint.as_deref().unwrap_or_default(),
            request.server_name.as_deref(),
        )
        .await;
    json::accepted()
}

/// The server approved, but saving replaces the connection to another server: the person confirms.
pub(crate) async fn confirm_pairing(State(app): State<ClientApi>) -> Response {
    if app.client.confirm_pairing().await {
        json::accepted()
    } else {
        json::problem(&[(
            "pairing",
            "There is nothing to confirm. Start pairing again.",
        )])
    }
}

pub(crate) async fn cancel_pairing(State(app): State<ClientApi>) -> Response {
    app.client.cancel_pairing().await;
    json::accepted()
}

pub(crate) async fn devices(State(app): State<ClientApi>) -> Response {
    json::ok(&app.usb_client.current())
}

/// Attach remembers the device, so it is attached again after a restart. Answers 202; the device
/// shows as attached, or with its fault, when `ClientDevicesChanged` arrives.
pub(crate) async fn attach(State(app): State<ClientApi>, Path(bus_id): Path<String>) -> Response {
    if bus_id.parse::<BusId>().is_err() {
        return json::problem(&[("busId", "That is not a USB bus ID.")]);
    }
    app.usb_client.attach(&bus_id).await;
    json::accepted()
}

/// Detach forgets the device. Detaching one that is not attached is fine too.
pub(crate) async fn detach(State(app): State<ClientApi>, Path(bus_id): Path<String>) -> Response {
    if bus_id.parse::<BusId>().is_err() {
        return json::problem(&[("busId", "That is not a USB bus ID.")]);
    }
    app.usb_client.detach(&bus_id).await;
    json::accepted()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_keeps_its_scheme_host_and_port_only() {
        assert_eq!(
            https_authority("https://Koppel-Contract-Main:5240/some/path?x=1").as_deref(),
            Some("https://koppel-contract-main:5240")
        );
        assert_eq!(
            https_authority("https://host:443").as_deref(),
            Some("https://host")
        );
        assert_eq!(
            https_authority("HTTPS://[FE80::1]:5240").as_deref(),
            Some("https://[fe80::1]:5240")
        );
        for bad in [
            "http://elsewhere:5240",
            "not a url",
            "https://",
            "https://host:x",
            "https://a b",
        ] {
            assert_eq!(https_authority(bad), None, "{bad}");
        }
    }

    #[test]
    fn fingerprints_compare_by_their_digits() {
        let typed = "ab".repeat(32);
        assert!(is_fingerprint(Some(&typed)));
        assert!(!is_fingerprint(Some("AB:CD")));
        assert_eq!(normalize(&typed).len(), 95);
        assert!(normalize(&typed).starts_with("AB:AB"));
        assert!(same_fingerprint(Some(&typed), Some(&normalize(&typed))));
    }

    #[test]
    fn the_secret_carries_over_only_to_the_same_server() {
        let fingerprint = "AB".repeat(32);
        let current = ClientConnection {
            server_url: Some("https://a:5240".into()),
            server_fingerprint: Some(normalize(&fingerprint)),
            client_id: Some("c".into()),
            client_secret: Some("s".into()),
            next_server_fingerprint: None,
        };
        let same = ConnectionUpdate {
            server_url: Some("https://A:5240/x".into()),
            server_fingerprint: Some(fingerprint.to_lowercase()),
            client_id: Some(" c ".into()),
            client_secret: Some(String::new()),
        };
        assert!(same.keeps_server(&current));
        let applied = same.apply(&current);
        assert_eq!(
            applied.client_secret.as_deref().map(String::as_str),
            Some("s")
        );
        assert_eq!(applied.client_id.as_deref(), Some("c"));
        let other = ConnectionUpdate {
            server_url: Some("https://b:5240".into()),
            ..same
        };
        assert!(!other.keeps_server(&current));
        assert_eq!(other.apply(&current).client_secret, None);
    }
}

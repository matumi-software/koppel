//! Comparing the addresses and certificates the saved connection names.

use http::Uri;
use koppel_config::files::ClientConnection;

/// The URL as an `http::Uri`, with `https` assumed when there is no scheme.
fn parse(url: &str) -> Option<Uri> {
    if url.contains("://") {
        url.parse().ok()
    } else {
        format!("https://{url}").parse().ok()
    }
}

/// The authority of a URL such as `https://Host:5240/x`, in lower case and without the scheme's
/// default port, so two spellings of one address compare equal.
pub(super) fn authority(url: &str) -> String {
    let Some(uri) = parse(url) else {
        return url.to_ascii_lowercase();
    };
    let Some(authority) = uri.authority() else {
        return String::new();
    };
    let host = authority.host().to_ascii_lowercase();
    let default = if uri
        .scheme_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("http"))
    {
        80
    } else {
        443
    };
    match authority.port_u16() {
        Some(port) if port != default => format!("{host}:{port}"),
        _ => host,
    }
}

/// The host part of a URL, as the activity log names a server. An IPv6 address keeps its
/// brackets.
pub(super) fn url_host(url: &str) -> String {
    parse(url)
        .and_then(|uri| uri.authority().map(|a| a.host().to_ascii_lowercase()))
        .unwrap_or_default()
}

/// A SHA-256 fingerprint compared on its hex digits only, in any case.
fn same_fingerprint(a: Option<&str>, b: Option<&str>) -> bool {
    let digits = |f: Option<&str>| -> String {
        f.unwrap_or_default()
            .chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    digits(a) == digits(b)
}

/// The same server and credentials as `previous`: the same client ID and secret, and a
/// certificate that is either the one pinned before or the one the server said it would switch to.
pub(super) fn continues(now: &ClientConnection, previous: &ClientConnection) -> bool {
    now.client_id == previous.client_id
        && now.client_secret == previous.client_secret
        && now.server_fingerprint.is_some()
        && (same_fingerprint(
            now.server_fingerprint.as_deref(),
            previous.server_fingerprint.as_deref(),
        ) || same_fingerprint(
            now.server_fingerprint.as_deref(),
            previous.next_server_fingerprint.as_deref(),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_server_and_credentials_continue_across_a_certificate_switch() {
        let before = ClientConnection {
            server_url: Some("https://server:5240".into()),
            server_fingerprint: Some("AA:BB".into()),
            client_id: Some("c1".into()),
            client_secret: Some("s".into()),
            next_server_fingerprint: Some("CC:DD".into()),
        };
        let switched = ClientConnection {
            server_fingerprint: Some("ccdd".into()),
            next_server_fingerprint: None,
            ..before.clone()
        };
        let other = ClientConnection {
            client_secret: Some("t".into()),
            ..before.clone()
        };

        assert!(continues(&switched, &before));
        assert!(!continues(&other, &before));
    }

    #[test]
    fn addresses_compare_without_case_or_the_default_port() {
        assert_eq!(authority("https://Server:443/x"), "server");
        assert_eq!(authority("https://server:5240"), "server:5240");
        assert_eq!(authority("http://server:80"), "server");
        assert_eq!(authority("server:5240"), "server:5240");
        assert_eq!(authority("https://user@Server:5240/a?b#c"), "server:5240");
        assert_eq!(url_host("https://10.0.0.5:5240"), "10.0.0.5");
        assert_eq!(url_host("https://[fe80::1]:5240"), "[fe80::1]");
    }
}

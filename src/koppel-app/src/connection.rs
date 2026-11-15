//! The checks and spellings of a saved server's address and fingerprint, shared by the servers the
//! client keeps and what they save.

/// What is wrong with the address of a server.
pub const URL_PROBLEM: &str = "Use the server's HTTPS address, such as https://lab-rack-3:5240.";

/// The scheme, host and port of an HTTPS address, as .NET's `GetLeftPart(UriPartial.Authority)`
/// gives it: lower case, and without the port when it is 443. `None` when it is not one.
pub fn https_authority(url: &str) -> Option<String> {
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
pub fn digits(fingerprint: Option<&str>) -> String {
    fingerprint
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect()
}

/// A SHA-256 fingerprint: 64 hex digits, with or without separators.
pub fn is_fingerprint(fingerprint: Option<&str>) -> bool {
    digits(fingerprint).len() == 64
}

pub fn same_fingerprint(a: Option<&str>, b: Option<&str>) -> bool {
    digits(a).eq_ignore_ascii_case(&digits(b))
}

/// The spelling Koppel stores: upper case hex in pairs, separated by colons.
pub fn normalize(fingerprint: &str) -> String {
    let hex = digits(Some(fingerprint)).to_ascii_uppercase();
    hex.as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).into_owned())
        .collect::<Vec<_>>()
        .join(":")
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
}

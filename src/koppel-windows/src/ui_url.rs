//! The UI's address, as the service publishes it for the desktop: `HKLM\SOFTWARE\Matumi\Koppel`,
//! value `UiUrl`, which every user can read and only the service writes. The Start menu entry opens
//! it, so a moved UI port is followed. Uninstall deletes the key.

use std::net::IpAddr;

use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE};

const KEY: &str = r"SOFTWARE\Matumi\Koppel";
const VALUE: &str = "UiUrl";

/// What the Start menu entry opens when the service has not said.
pub const DEFAULT: &str = "https://localhost:5240/";

/// The address the UI answers on: `localhost` unless `Ports:Address` names one address.
pub fn for_listener(address: Option<IpAddr>, port: u16) -> String {
    match address {
        Some(IpAddr::V4(a)) if !a.is_unspecified() => format!("https://{a}:{port}/"),
        Some(IpAddr::V6(a)) if !a.is_unspecified() => format!("https://[{a}]:{port}/"),
        _ => format!("https://localhost:{port}/"),
    }
}

/// Writes the address for the Start menu entry. The service runs as LocalSystem, which may; if it
/// cannot, the entry opens [`DEFAULT`], which is right unless the UI port was moved.
pub fn publish(url: &str) {
    let written = RegKey::predef(HKEY_LOCAL_MACHINE)
        .create_subkey_with_flags(KEY, KEY_SET_VALUE)
        .and_then(|(key, _)| key.set_value(VALUE, &url));
    if let Err(e) = written {
        tracing::warn!(
            "Could not write the UI's address for the Start menu entry ({e}); it opens {DEFAULT}"
        );
    }
}

/// The address the service published, when it is an HTTPS address, or [`DEFAULT`].
pub fn read() -> String {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(VALUE))
        .ok()
        .and_then(|value| checked(&value))
        .unwrap_or_else(|| DEFAULT.to_owned())
}

/// `value` when it is an absolute HTTPS address with a host, ending in a slash.
pub fn checked(value: &str) -> Option<String> {
    let rest = value.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    // A quote would end the quoted argument the launcher builds.
    if host.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return None;
    }
    Some(if value.ends_with('/') {
        value.to_owned()
    } else {
        format!("{value}/")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_localhost_unless_one_is_named() {
        assert_eq!(for_listener(None, 5240), "https://localhost:5240/");
        assert_eq!(
            for_listener(Some("0.0.0.0".parse().unwrap()), 1),
            "https://localhost:1/"
        );
        assert_eq!(
            for_listener(Some("::".parse().unwrap()), 1),
            "https://localhost:1/"
        );
        assert_eq!(
            for_listener(Some("10.0.0.2".parse().unwrap()), 5240),
            "https://10.0.0.2:5240/"
        );
        assert_eq!(
            for_listener(Some("fd00::1".parse().unwrap()), 5240),
            "https://[fd00::1]:5240/"
        );
    }

    #[test]
    fn only_an_https_address_is_taken() {
        assert_eq!(
            checked("https://localhost:5250").as_deref(),
            Some("https://localhost:5250/")
        );
        assert_eq!(checked("http://localhost:5240/"), None);
        assert_eq!(checked("https:///"), None);
        assert_eq!(checked("https://a b/"), None);
        assert_eq!(checked("https://a\"b/"), None);
        assert_eq!(checked("file:///c:/x"), None);
    }
}

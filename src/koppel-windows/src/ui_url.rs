//! The UI's address, as the service publishes it for the desktop: `HKLM\SOFTWARE\Matumi\Koppel`,
//! value `UiUrl`, which every user can read and only the service writes. The Start menu entry opens
//! it, so a moved UI port is followed. Uninstall deletes the key.

use std::net::IpAddr;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW,
};
use windows::core::w;

const KEY: windows::core::PCWSTR = w!(r"SOFTWARE\Matumi\Koppel");
const VALUE: windows::core::PCWSTR = w!("UiUrl");

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
    let mut key = HKEY::default();
    // SAFETY: the out parameter is ours; the key is closed below.
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            KEY,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if created != ERROR_SUCCESS {
        tracing::warn!(
            "Could not write the UI's address for the Start menu entry ({created:?}); it opens {DEFAULT}"
        );
        return;
    }
    let wide: Vec<u8> = url
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: the key is open and the data is a terminated UTF-16 string.
    let set = unsafe { RegSetValueExW(key, VALUE, None, REG_SZ, Some(&wide)) };
    if set != ERROR_SUCCESS {
        tracing::warn!(
            "Could not write the UI's address for the Start menu entry ({set:?}); it opens {DEFAULT}"
        );
    }
    // SAFETY: opened above, closed once.
    let _ = unsafe { RegCloseKey(key) };
}

/// The address the service published, when it is an HTTPS address, or [`DEFAULT`].
pub fn read() -> String {
    let mut buffer = [0u16; 512];
    let mut size = size_of_val(&buffer) as u32;
    // SAFETY: the buffer and its size in bytes are ours.
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if read != ERROR_SUCCESS {
        return DEFAULT.to_owned();
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..length]);
    checked(&value).unwrap_or_else(|| DEFAULT.to_owned())
}

/// `value` when it is an absolute HTTPS address with a host, ending in a slash.
pub fn checked(value: &str) -> Option<String> {
    let rest = value.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    if host.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
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
        assert_eq!(checked("file:///c:/x"), None);
    }
}

//! JSON shapes: which fields an object must have and what kind of value each holds.
//!
//! A shape lists the required fields. Extra fields are allowed, so the program can add one without
//! breaking a test, and a field of an `Opt` kind may be missing or `null`. Values are checked by kind
//! only: times, ids and names differ from run to run.

use serde_json::Value;

/// What a field holds. The `Opt` kinds may also be `null`, or missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Str,
    OptStr,
    Bool,
    Int,
    OptInt,
    /// An RFC 3339 time with an offset, such as `2026-11-09T07:58:00.7359782+00:00`.
    Time,
    OptTime,
    Array,
    Object,
    /// Anything, `null` included. For a field whose own test checks it.
    Any,
}

impl Kind {
    fn optional(self) -> bool {
        matches!(self, Kind::OptStr | Kind::OptInt | Kind::OptTime)
    }

    fn admits(self, value: &Value) -> bool {
        match self {
            Kind::Str => value.is_string(),
            Kind::OptStr => value.is_string() || value.is_null(),
            Kind::Bool => value.is_boolean(),
            Kind::Int => value.is_i64() || value.is_u64(),
            Kind::OptInt => value.is_i64() || value.is_u64() || value.is_null(),
            Kind::Time => value.as_str().is_some_and(is_time),
            Kind::OptTime => value.is_null() || value.as_str().is_some_and(is_time),
            Kind::Array => value.is_array(),
            Kind::Object => value.is_object(),
            Kind::Any => true,
        }
    }
}

/// Fails the test unless `value` is an object with these fields, each of its kind. Other fields are
/// fine, and an `Opt` field may be missing.
#[track_caller]
pub fn fields(value: &Value, fields: &[(&str, Kind)], context: &str) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{context}: not an object: {value}"));
    for (name, kind) in fields {
        match object.get(*name) {
            None if kind.optional() => {}
            None => panic!("{context}: {name} is missing in {value}"),
            Some(field) => assert!(
                kind.admits(field),
                "{context}: {name} should be {kind:?}, is {field}"
            ),
        }
    }
}

/// Fails the test unless `value` is an array, and checks each item with `item`.
#[track_caller]
pub fn each(value: &Value, context: &str, mut item: impl FnMut(&Value)) {
    let items = value
        .as_array()
        .unwrap_or_else(|| panic!("{context}: not an array: {value}"));
    for entry in items {
        item(entry);
    }
}

/// True for an RFC 3339 date and time with seconds and an offset (`Z` or `+hh:mm`).
pub fn is_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    if !(digits(0..4)
        && bytes.get(4) == Some(&b'-')
        && digits(5..7)
        && bytes.get(7) == Some(&b'-')
        && digits(8..10)
        && matches!(bytes.get(10), Some(b'T' | b't'))
        && digits(11..13)
        && bytes.get(13) == Some(&b':')
        && digits(14..16)
        && bytes.get(16) == Some(&b':')
        && digits(17..19))
    {
        return false;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let end = fraction
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(fraction.len());
        if end == 0 {
            return false;
        }
        rest = &fraction[end..];
    }
    if rest.eq_ignore_ascii_case("z") {
        return true;
    }
    let offset = rest.as_bytes();
    offset.len() == 6
        && matches!(offset[0], b'+' | b'-')
        && offset[1..3].iter().all(u8::is_ascii_digit)
        && offset[3] == b':'
        && offset[4..6].iter().all(u8::is_ascii_digit)
}

/// The fields of an activity entry, as `GET /api/activity` and the push carry it.
pub const ACTIVITY: &[(&str, Kind)] = &[
    ("id", Kind::Str),
    ("timestampUtc", Kind::Time),
    ("entity", Kind::Object),
    ("reason", Kind::Str),
    ("outcome", Kind::Str),
    ("summary", Kind::Str),
    ("detail", Kind::OptStr),
];

/// What an activity entry is about.
pub const ENTITY: &[(&str, Kind)] = &[("id", Kind::Str), ("name", Kind::Str), ("type", Kind::Str)];

/// The settings, as `GET /api/settings` and the push carry them.
pub const SETTINGS: &[(&str, Kind)] = &[
    ("role", Kind::Str),
    ("shareOnStartup", Kind::Bool),
    ("setupComplete", Kind::Bool),
    ("notifyPairing", Kind::Bool),
    ("notifyLink", Kind::Bool),
    ("notifyAttachFailures", Kind::Bool),
];

/// One registered client on the server.
pub const CLIENT: &[(&str, Kind)] = &[
    ("id", Kind::Str),
    ("name", Kind::Str),
    ("createdAt", Kind::Time),
    ("lastSeen", Kind::OptTime),
    ("hasNextCertificate", Kind::Bool),
    ("hasSecret", Kind::Bool),
];

/// The server's certificate and the next one while a handover is under way. 0.1.x also had the token
/// signing key here; Skakel 0.3.0's tokens are not signed.
pub const KEYS: &[(&str, Kind)] = &[
    ("fingerprint", Kind::Str),
    ("nextFingerprint", Kind::OptStr),
    ("nextPreparedAt", Kind::OptTime),
];

/// The pairing window and the pairing requests waiting on the server. 0.1.x had no window: it took
/// requests at any time.
pub const PAIRING_OVERVIEW: &[(&str, Kind)] = &[
    ("requests", Kind::Array),
    ("budgetExhausted", Kind::Bool),
    ("window", Kind::Object),
];

/// Whether the server takes new pairing requests, and until when.
pub const PAIRING_WINDOW: &[(&str, Kind)] = &[
    ("open", Kind::Bool),
    ("alwaysOpen", Kind::Bool),
    ("until", Kind::OptTime),
];

/// One pairing request on the server.
pub const PAIRING_REQUEST: &[(&str, Kind)] = &[
    ("id", Kind::Str),
    ("clientName", Kind::Str),
    ("from", Kind::Str),
    ("requestedAt", Kind::Time),
    ("expiresAt", Kind::Time),
    ("state", Kind::Str),
    ("message", Kind::OptStr),
];

/// Where pairing stands on the client.
pub const PAIRING_STATUS: &[(&str, Kind)] = &[
    ("state", Kind::Str),
    ("serverName", Kind::OptStr),
    ("serverUrl", Kind::OptStr),
    ("code", Kind::OptStr),
    ("message", Kind::OptStr),
    ("expiresAt", Kind::OptTime),
    ("serverFingerprint", Kind::OptStr),
];

/// The client's stored connection, without the secret.
pub const CONNECTION: &[(&str, Kind)] = &[
    ("serverUrl", Kind::OptStr),
    ("serverFingerprint", Kind::OptStr),
    ("clientId", Kind::OptStr),
    ("hasSecret", Kind::Bool),
    ("nextServerFingerprint", Kind::OptStr),
];

/// The server's devices.
pub const SERVER_DEVICES: &[(&str, Kind)] =
    &[("devices", Kind::Array), ("unavailable", Kind::OptStr)];

/// One USB device as the server's UI sees it.
pub const DEVICE: &[(&str, Kind)] = &[
    ("busId", Kind::Str),
    ("vidPid", Kind::Str),
    ("name", Kind::Str),
    ("manufacturer", Kind::OptStr),
    ("speed", Kind::Str),
    ("state", Kind::Str),
    ("attachedTo", Kind::OptStr),
    ("attachedSince", Kind::OptTime),
    ("fault", Kind::OptStr),
    ("serial", Kind::OptStr),
    ("driver", Kind::OptStr),
    ("interfaces", Kind::Any),
];

/// The client's link and the devices it sees.
pub const CLIENT_DEVICES: &[(&str, Kind)] = &[
    ("link", Kind::Str),
    ("linkMessage", Kind::OptStr),
    ("devices", Kind::Array),
    ("unavailable", Kind::OptStr),
    ("gateway", Kind::Bool),
    ("gatewayFault", Kind::OptStr),
];

/// One server found on the network.
pub const DISCOVERED_SERVER: &[(&str, Kind)] = &[
    ("name", Kind::Str),
    ("address", Kind::Str),
    ("httpsPort", Kind::Int),
    ("fingerprint", Kind::Str),
    ("version", Kind::OptStr),
    ("otherAddresses", Kind::Array),
];

/// The answer to the client's connection test.
pub const CONNECTION_TEST: &[(&str, Kind)] = &[
    ("ok", Kind::Bool),
    ("outcome", Kind::Str),
    ("message", Kind::Str),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_wants_its_fields_and_allows_more() {
        let shape = &[("a", Kind::Str), ("b", Kind::OptInt)];
        fields(&serde_json::json!({ "a": "x", "b": null }), shape, "null");
        fields(&serde_json::json!({ "a": "x" }), shape, "missing");
        fields(&serde_json::json!({ "a": "x", "c": 1 }), shape, "extra");
        let missing = std::panic::catch_unwind(|| {
            fields(&serde_json::json!({ "b": 1 }), shape, "required");
        });
        assert!(missing.is_err());
        let wrong = std::panic::catch_unwind(|| {
            fields(&serde_json::json!({ "a": 1 }), shape, "kind");
        });
        assert!(wrong.is_err());
    }

    #[test]
    fn times_need_seconds_and_an_offset() {
        assert!(is_time("2026-11-09T07:58:00.7359782+00:00"));
        assert!(is_time("2026-11-09T07:58:00Z"));
        assert!(!is_time("2026-11-09T07:58"));
        assert!(!is_time("2026-11-09T07:58:00"));
        assert!(!is_time("2026-11-09 07:58:00Z"));
    }
}

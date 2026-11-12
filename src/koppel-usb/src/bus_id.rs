//! A kernel bus ID such as `1-1.4`: a bus number, a dash, then the port path. A [`BusId`] can
//! only be made from text that is one, so a value from the network or from a local USB/IP driver
//! is checked once, at the edge, and can then carry nothing into a command line, a log line, a
//! sysfs path or a UI row.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The kernel's own limit (`SYSFS_BUS_ID_SIZE` is 32, with the terminator).
pub const MAX_LENGTH: usize = 31;

/// A bus ID that passed the check: `1-1`, `1-1.4`, `3-2.1.7.4`. ASCII digits, one dash, dots
/// between port numbers, at most [`MAX_LENGTH`] bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BusId(String);

/// The text is not a USB bus ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("That is not a USB bus ID.")]
pub struct InvalidBusId;

impl BusId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BusId {
    type Err = InvalidBusId;

    fn from_str(text: &str) -> Result<Self, InvalidBusId> {
        if text.is_empty() || text.len() > MAX_LENGTH {
            return Err(InvalidBusId);
        }
        let (bus, ports) = text.split_once('-').ok_or(InvalidBusId)?;
        if digits(bus) && ports.split('.').all(digits) {
            Ok(Self(text.to_owned()))
        } else {
            Err(InvalidBusId)
        }
    }
}

impl TryFrom<String> for BusId {
    type Error = InvalidBusId;

    fn try_from(text: String) -> Result<Self, InvalidBusId> {
        text.parse()
    }
}

impl fmt::Display for BusId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for BusId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for BusId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for BusId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for BusId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Serialize for BusId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BusId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn valid(text: &str) -> bool {
        text.parse::<BusId>().is_ok()
    }

    #[test]
    fn valid_bus_ids_are_accepted() {
        for bus_id in ["1-1", "1-1.4", "3-2.1.7.4", "12-10"] {
            assert!(valid(bus_id), "{bus_id}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for bus_id in [
            "",
            "1",
            "1-",
            "-1",
            "1-1.",
            "1-1.4 --stop",
            "--bus-id=1-1",
            "1-1.4\n",
            "a-b",
            "1-1.4;calc",
            "\u{661}-\u{661}",
            "1-1-1",
            "1-1..4",
        ] {
            assert!(!valid(bus_id), "{bus_id:?}");
        }
    }

    #[test]
    fn a_bus_id_longer_than_the_kernel_allows_is_refused() {
        let longest = format!("1-{}", vec!["1"; 15].join("."));
        assert!(longest.len() <= MAX_LENGTH);
        assert!(valid(&longest));
        assert!(!valid(&format!("{longest}.1")));
    }

    #[test]
    fn the_error_is_a_sentence_for_the_ui() {
        let error = "../../etc".parse::<BusId>().unwrap_err();
        assert!(error.to_string().contains("not a USB bus ID"));
    }

    #[test]
    fn deserialising_goes_through_the_check() {
        let ok: BusId = serde_json::from_str("\"1-1.4\"").unwrap();
        assert_eq!(ok.as_str(), "1-1.4");
        assert_eq!(serde_json::to_string(&ok).unwrap(), "\"1-1.4\"");
        assert!(serde_json::from_str::<BusId>("\"1-1.4 --stop\"").is_err());
    }

    proptest! {
        /// Whatever text comes in, a bus ID that parses has the shape the kernel's does and
        /// survives a round trip; nothing else gets through.
        #[test]
        fn a_parsed_bus_id_always_has_the_kernel_shape(text in ".*") {
            if let Ok(id) = text.parse::<BusId>() {
                prop_assert!(id.as_str().len() <= MAX_LENGTH);
                prop_assert!(id.as_str().bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'.'));
                prop_assert_eq!(id.as_str().matches('-').count(), 1);
                prop_assert_eq!(id.as_str().parse::<BusId>().unwrap(), id);
            }
        }

        #[test]
        fn every_well_formed_bus_id_parses(bus in 1u32..1000, ports in prop::collection::vec(1u32..100, 1..5)) {
            let text = format!("{bus}-{}", ports.iter().map(u32::to_string).collect::<Vec<_>>().join("."));
            prop_assert_eq!(text.len() <= MAX_LENGTH, text.parse::<BusId>().is_ok());
        }
    }
}

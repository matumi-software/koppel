//! A kernel bus ID such as `1-1.4`: a bus number, a dash, then the port path. Bus IDs from the
//! network and from a local USB/IP driver are checked with this before they are used, so one
//! cannot carry anything into a command line, a log line, a sysfs path or a UI row.

use crate::UsbError;

/// The kernel's own limit (`SYSFS_BUS_ID_SIZE` is 32, with the terminator).
pub const MAX_LENGTH: usize = 31;

/// True for `1-1`, `1-1.4`, `3-2.1.7.4`: ASCII digits, one dash, dots between port numbers.
pub fn is_valid(bus_id: &str) -> bool {
    if bus_id.is_empty() || bus_id.len() > MAX_LENGTH {
        return false;
    }
    let Some((bus, ports)) = bus_id.split_once('-') else {
        return false;
    };
    digits(bus) && ports.split('.').all(digits)
}

/// The bus ID, or an error the UI can show when it is not one.
pub fn require(bus_id: &str) -> Result<&str, UsbError> {
    if is_valid(bus_id) {
        Ok(bus_id)
    } else {
        Err(UsbError::new("That is not a USB bus ID."))
    }
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_bus_ids_are_accepted() {
        for bus_id in ["1-1", "1-1.4", "3-2.1.7.4", "12-10"] {
            assert!(is_valid(bus_id), "{bus_id}");
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
            assert!(!is_valid(bus_id), "{bus_id:?}");
        }
    }

    #[test]
    fn a_bus_id_longer_than_the_kernel_allows_is_refused() {
        let longest = format!("1-{}", vec!["1"; 15].join("."));
        assert!(longest.len() <= MAX_LENGTH);
        assert!(is_valid(&longest));
        assert!(!is_valid(&format!("{longest}.1")));
    }

    #[test]
    fn require_says_what_is_wrong() {
        assert_eq!(require("1-1.4"), Ok("1-1.4"));
        assert!(
            require("../../etc")
                .unwrap_err()
                .message()
                .contains("not a USB bus ID")
        );
    }
}

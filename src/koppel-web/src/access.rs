//! Who may use the UI, by the caller's address (`Ui:AllowFrom`), and what a caller is for the limits.

use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::sync::Unpoisoned;

/// Who may use the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiReach {
    /// This machine only.
    Local,
    /// This machine and the local network.
    Lan,
    /// Anyone who can reach the port.
    Any,
}

impl UiReach {
    /// The setting's name.
    pub const SETTING: &str = "Ui:AllowFrom";

    /// `local`, `lan` or `any`, in any case. Unset, `lan`, or `local` in Development. Anything else
    /// stops the start, naming the setting.
    pub fn resolve(setting: Option<&str>, development: bool) -> Result<Self, String> {
        match setting.map(str::trim).filter(|s| !s.is_empty()) {
            None if development => Ok(Self::Local),
            None => Ok(Self::Lan),
            Some(value) => match value.to_ascii_lowercase().as_str() {
                "local" => Ok(Self::Local),
                "lan" => Ok(Self::Lan),
                "any" => Ok(Self::Any),
                _ => Err(format!(
                    "{} is \"{value}\". Use local (this machine), lan (this machine and the local network) or any.",
                    Self::SETTING
                )),
            },
        }
    }
}

/// The private and link-local ranges, which count as the local network whatever this machine's own
/// addresses are. Carrier-grade NAT (100.64.0.0/10) is not among them.
const PRIVATE_RANGES: &[(IpAddr, u8)] = &[
    (IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 0)), 8),
    (IpAddr::V4(std::net::Ipv4Addr::new(172, 16, 0, 0)), 12),
    (IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 0, 0)), 16),
    (IpAddr::V4(std::net::Ipv4Addr::new(169, 254, 0, 0)), 16),
    (
        IpAddr::V6(std::net::Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0)),
        7,
    ),
    (
        IpAddr::V6(std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)),
        10,
    ),
];

/// How long the list of this machine's networks is kept before it is read again. Addresses change
/// with DHCP and Wi-Fi.
const NETWORKS_FOR: Duration = Duration::from_secs(30);

/// Networks as an address and a prefix length.
type Networks = Vec<(IpAddr, u8)>;

/// Decides by the caller's address whether it may use the UI, as Sunshine's
/// `origin_web_ui_allowed` does. The network surface is not affected: it has tokens.
pub(crate) struct UiAccess {
    reach: UiReach,
    networks: Mutex<Option<(Instant, Networks)>>,
}

impl UiAccess {
    pub(crate) fn new(reach: UiReach) -> Self {
        Self {
            reach,
            networks: Mutex::new(None),
        }
    }

    /// True when a caller at `remote` may use the UI. The local network is loopback, the private
    /// ranges, link-local, IPv6 unique local addresses, and any network this machine has an address
    /// on, since most home IPv6 is global and the private ranges alone would turn a neighbour away.
    pub(crate) fn allows(&self, remote: IpAddr) -> bool {
        if self.reach == UiReach::Any {
            return true;
        }
        let remote = canonical(remote);
        if remote.is_loopback() {
            return true;
        }
        self.reach == UiReach::Lan
            && (PRIVATE_RANGES
                .iter()
                .any(|(network, prefix)| in_network(remote, *network, *prefix))
                || self
                    .local_networks()
                    .iter()
                    .any(|(network, prefix)| in_network(remote, *network, *prefix)))
    }

    fn local_networks(&self) -> Networks {
        let mut cached = self.networks.locked();
        match &*cached {
            Some((read, networks)) if read.elapsed() < NETWORKS_FOR => networks.clone(),
            _ => {
                let networks = interface_networks();
                *cached = Some((Instant::now(), networks.clone()));
                networks
            }
        }
    }
}

/// The networks this machine has an address on, loopback left out. A very short prefix would make
/// half the internet local, so only /8 to /32 on IPv4 and /48 to /128 on IPv6 count.
fn interface_networks() -> Networks {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter(|interface| !interface.is_loopback())
        .filter_map(|interface| {
            let (address, prefix) = match interface.addr {
                if_addrs::IfAddr::V4(v4) => (IpAddr::V4(v4.ip), v4.prefixlen),
                if_addrs::IfAddr::V6(v6) => (IpAddr::V6(v6.ip), v6.prefixlen),
            };
            let fits = match address {
                IpAddr::V4(_) => (8..=32).contains(&prefix),
                IpAddr::V6(_) => (48..=128).contains(&prefix),
            };
            fits.then_some((address, prefix))
        })
        .collect()
}

/// An IPv4-mapped IPv6 address as the IPv4 address it carries.
pub(crate) fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

/// What a caller's rate limits count: one whole address, an IPv4-mapped IPv6 address counting as its
/// IPv4 address, as Skakel's limits do. The sign-in lock counts by [`throttle_key`] instead.
pub(crate) fn address_key(address: IpAddr) -> String {
    canonical(address).to_string()
}

/// What the sign-in lock counts: an IPv4 address whole, an IPv6 address as its /64. A host on a /64
/// owns the whole prefix and can take any address in it, so counting whole addresses would let one
/// neighbour rotate past the lock. The price is that a shared /64 shares its lock.
pub(crate) fn throttle_key(address: IpAddr) -> String {
    match canonical(address) {
        IpAddr::V6(v6) => {
            let [a, b, c, d, ..] = v6.segments();
            format!("{}/64", std::net::Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
        }
        v4 => v4.to_string(),
    }
}

pub(crate) fn in_network(address: IpAddr, network: IpAddr, prefix: u8) -> bool {
    let (address, network): (Vec<u8>, Vec<u8>) = match (address, network) {
        (IpAddr::V4(a), IpAddr::V4(n)) => (a.octets().to_vec(), n.octets().to_vec()),
        (IpAddr::V6(a), IpAddr::V6(n)) => (a.octets().to_vec(), n.octets().to_vec()),
        _ => return false,
    };
    let whole = usize::from(prefix / 8);
    if address[..whole] != network[..whole] {
        return false;
    }
    let rest = prefix % 8;
    if rest == 0 {
        return true;
    }
    let mask = 0xFFu8 << (8 - rest);
    address[whole] & mask == network[whole] & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_setting_takes_three_words_and_defaults_by_environment() {
        assert_eq!(UiReach::resolve(None, false), Ok(UiReach::Lan));
        assert_eq!(UiReach::resolve(Some(" "), true), Ok(UiReach::Local));
        assert_eq!(UiReach::resolve(Some("ANY"), false), Ok(UiReach::Any));
        assert!(
            UiReach::resolve(Some("everyone"), false)
                .unwrap_err()
                .contains("Ui:AllowFrom")
        );
    }

    #[test]
    fn local_takes_loopback_only_and_lan_adds_the_private_ranges() {
        let local = UiAccess::new(UiReach::Local);
        assert!(local.allows(ip("127.0.0.1")));
        assert!(local.allows(ip("::1")));
        assert!(local.allows(ip("::ffff:127.0.0.1")));
        assert!(!local.allows(ip("192.168.1.5")));

        let lan = UiAccess::new(UiReach::Lan);
        assert!(lan.allows(ip("192.168.1.5")));
        assert!(lan.allows(ip("::ffff:10.1.2.3")));
        assert!(lan.allows(ip("172.31.255.1")));
        assert!(!lan.allows(ip("172.32.0.1")));
        assert!(lan.allows(ip("fe80::1")));
        assert!(lan.allows(ip("fd12::1")));
        assert!(!lan.allows(ip("100.64.0.1")));
        assert!(UiAccess::new(UiReach::Any).allows(ip("8.8.8.8")));
    }

    #[test]
    fn a_caller_is_its_whole_address() {
        assert_eq!(address_key(ip("::ffff:192.168.1.5")), "192.168.1.5");
        assert_eq!(address_key(ip("2001:db8::1")), "2001:db8::1");
        assert_eq!(throttle_key(ip("::ffff:192.168.1.5")), "192.168.1.5");
        assert_eq!(
            throttle_key(ip("2001:db8:1:2:aaaa::1")),
            "2001:db8:1:2::/64"
        );
        assert_eq!(
            throttle_key(ip("2001:db8:1:2::1")),
            throttle_key(ip("2001:db8:1:2:ffff:ffff:ffff:ffff"))
        );
        assert_ne!(
            throttle_key(ip("2001:db8:1:2::1")),
            throttle_key(ip("2001:db8:1:3::1"))
        );
        assert!(in_network(ip("10.0.0.255"), ip("10.0.0.0"), 24));
        assert!(!in_network(ip("10.0.1.0"), ip("10.0.0.0"), 24));
    }
}

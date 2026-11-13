//! Random bytes and hex, for ids and tokens.

use ring::rand::{SecureRandom, SystemRandom};

/// `N` random bytes.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system has random bytes");
    bytes
}

/// `bytes` as lower case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

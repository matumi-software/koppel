//! The UI password: what is allowed, and how it is hashed for `ui-auth.json`, the same as 0.1.x so
//! a 0.1.x hash still signs in.

use std::num::NonZeroU32;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::pbkdf2;

pub(crate) const MIN_LENGTH: usize = 10;
pub(crate) const MAX_LENGTH: usize = 128;

/// OWASP's figure for PBKDF2-HMAC-SHA256. A stored hash with fewer is not accepted.
const ITERATIONS: u32 = 600_000;
const PREFIX: &str = "pbkdf2-sha256";
const SALT_BYTES: usize = 16;
const HASH_BYTES: usize = 32;
const OBVIOUS: [&str; 2] = ["koppel", "password"];

/// The length as .NET counts it, in UTF-16 code units, so the limits are the same as 0.1.x's.
pub(crate) fn length(password: &str) -> usize {
    password.encode_utf16().count()
}

/// Why a password is not acceptable, worded for the person choosing it, or `None`.
pub(crate) fn validate(password: Option<&str>) -> Option<String> {
    let password = password.unwrap_or_default();
    if length(password) < MIN_LENGTH {
        return Some(format!("Use at least {MIN_LENGTH} characters."));
    }
    if length(password) > MAX_LENGTH {
        return Some(format!("Use at most {MAX_LENGTH} characters."));
    }
    OBVIOUS
        .iter()
        .any(|word| password.eq_ignore_ascii_case(word))
        .then(|| "That password is too easy to guess.".to_owned())
}

/// `pbkdf2-sha256$iterations$salt$hash`, so the parameters travel with the hash. Takes a moment on
/// purpose: call it off the async threads.
pub(crate) fn hash(password: &str) -> String {
    let salt = super::random_bytes::<SALT_BYTES>();
    let mut out = [0u8; HASH_BYTES];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(ITERATIONS).expect("not zero"),
        &salt,
        password.as_bytes(),
        &mut out,
    );
    format!(
        "{PREFIX}${ITERATIONS}${}${}",
        STANDARD.encode(salt),
        STANDARD.encode(out)
    )
}

/// True when `password` is the one `stored` was made from. Takes a moment, as [`hash`] does.
pub(crate) fn verify(password: &str, stored: &str) -> bool {
    let Some((iterations, salt, expected)) = parse(stored) else {
        return false;
    };
    pbkdf2::verify(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        &salt,
        password.as_bytes(),
        &expected,
    )
    .is_ok()
}

fn parse(stored: &str) -> Option<(NonZeroU32, Vec<u8>, Vec<u8>)> {
    if !koppel_config::files::password_hash_is_well_formed(stored) {
        return None;
    }
    let mut parts = stored.split('$').skip(1);
    let iterations = parts.next()?.parse::<u32>().ok()?;
    let salt = STANDARD.decode(parts.next()?).ok()?;
    let hash = STANDARD.decode(parts.next()?).ok()?;
    Some((NonZeroU32::new(iterations)?, salt, hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_are_0_1s() {
        assert!(validate(None).is_some());
        assert!(validate(Some("123456789")).is_some());
        assert!(validate(Some("1234567890")).is_none());
        assert!(validate(Some(&"x".repeat(129))).is_some());
        assert!(validate(Some(&"x".repeat(128))).is_none());
        assert!(validate(Some("PassWord")).is_some());
        // An emoji is two UTF-16 units, as .NET counts it.
        assert!(validate(Some("😀😀😀😀😀")).is_none());
    }

    #[test]
    fn a_hash_verifies_its_password_only() {
        let made = hash("correct horse");
        assert!(koppel_config::files::password_hash_is_well_formed(&made));
        assert!(verify("correct horse", &made));
        assert!(!verify("wrong horse", &made));
        assert!(!verify("anything", "pbkdf2-sha256$1$c2FsdA==$aGFzaA=="));
    }

    #[test]
    fn a_hash_made_elsewhere_with_the_same_parameters_verifies() {
        // Python's hashlib.pbkdf2_hmac("sha256", ..., salt 00..0F, 600000, 32), the standard
        // PBKDF2 that .NET's Rfc2898DeriveBytes.Pbkdf2 in 0.1.x is too.
        let stored = "pbkdf2-sha256$600000$AAECAwQFBgcICQoLDA0ODw==$uwbIwLHdW/1OQPTil6LQ5k2n75S0uOwgmJAhyLQVNq0=";
        assert!(verify("correct horse battery", stored));
    }
}

//! Properties of the secret file format, checked with proptest on many generated inputs.
//! A failing case is shrunk and saved in `proptest-regressions/`, so it runs again first next time.
//!
//! Nothing here may panic, whatever the bytes: the files are read from a disk anyone with access
//! can change.

use std::fs;
use std::sync::OnceLock;

use koppel_config::SecretBox;
use proptest::prelude::*;

/// One box for every case: making one makes a key file, which is slow next to a seal.
fn secrets() -> &'static SecretBox {
    static BOX: OnceLock<SecretBox> = OnceLock::new();
    BOX.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("koppel-props-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        SecretBox::with_key_file(&dir).unwrap()
    })
}

proptest! {
    #[test]
    fn what_is_sealed_opens_to_the_same_bytes(plain in prop::collection::vec(any::<u8>(), 0..8192)) {
        let sealed = secrets().protect(&plain).unwrap();
        prop_assert_eq!(secrets().unprotect(&sealed).unwrap(), plain);
    }

    #[test]
    fn a_sealed_file_is_never_the_same_twice(plain in prop::collection::vec(any::<u8>(), 0..64)) {
        let first = secrets().protect(&plain).unwrap();
        let second = secrets().protect(&plain).unwrap();
        prop_assert_ne!(first, second);
    }

    /// Every byte of the file, with any change to it, makes it unreadable.
    #[test]
    fn any_flipped_byte_is_an_error(
        plain in prop::collection::vec(any::<u8>(), 0..300),
        change in 1u8..=255,
    ) {
        let sealed = secrets().protect(&plain).unwrap();
        for position in 0..sealed.len() {
            let mut damaged = sealed.clone();
            damaged[position] ^= change;
            prop_assert!(
                secrets().unprotect(&damaged).is_err(),
                "byte {} of {} ^ {:#x} opened",
                position,
                sealed.len(),
                change
            );
        }
    }

    #[test]
    fn any_truncation_or_extension_is_an_error(
        plain in prop::collection::vec(any::<u8>(), 0..300),
        extra in prop::collection::vec(any::<u8>(), 1..16),
    ) {
        let sealed = secrets().protect(&plain).unwrap();
        for length in 0..sealed.len() {
            prop_assert!(
                secrets().unprotect(&sealed[..length]).is_err(),
                "{} of {} bytes opened",
                length,
                sealed.len()
            );
        }
        let mut longer = sealed;
        longer.extend_from_slice(&extra);
        prop_assert!(secrets().unprotect(&longer).is_err());
    }

    #[test]
    fn random_bytes_are_an_error(bytes in prop::collection::vec(any::<u8>(), 0..200)) {
        prop_assert!(secrets().unprotect(&bytes).is_err());
    }

    /// The header and key id of a real file, so it gets as far as the cipher.
    #[test]
    fn random_bytes_with_a_good_header_are_an_error(tail in prop::collection::vec(any::<u8>(), 38..200)) {
        let sealed = secrets().protect(b"x").unwrap();
        let mut bytes = sealed[..22].to_vec();
        bytes.extend_from_slice(&tail);
        prop_assert!(secrets().unprotect(&bytes).is_err());
    }
}

//! Properties of the secret file format, checked on many random inputs.
//! The generator is fixed, so a failure repeats; the case that failed is in the message.
//!
//! Nothing here may panic, whatever the bytes: the files are read from a disk anyone with access
//! can change.

use std::fs;
use std::path::PathBuf;

use koppel_config::SecretBox;

/// xorshift64*: enough for test inputs, and the same on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| self.next() as u8).collect()
    }
}

fn secret_box(name: &str) -> (SecretBox, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "koppel-props-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    (SecretBox::with_key_file(&dir).unwrap(), dir)
}

#[test]
fn what_is_sealed_opens_to_the_same_bytes() {
    let (secrets, dir) = secret_box("roundtrip");
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for case in 0..400 {
        let length = if case < 40 { case } else { random.below(8192) };
        let plain = random.bytes(length);
        let sealed = secrets.protect(&plain).unwrap();
        assert_eq!(
            secrets.unprotect(&sealed).unwrap(),
            plain,
            "case {case}, {length} bytes"
        );
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_sealed_file_is_never_the_same_twice() {
    let (secrets, dir) = secret_box("nonce");
    let first = secrets.protect(b"same").unwrap();
    let second = secrets.protect(b"same").unwrap();
    assert_ne!(first, second);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn any_flipped_byte_is_an_error() {
    let (secrets, dir) = secret_box("flip");
    let mut random = Random(42);
    for length in [0, 1, 17, 200] {
        let sealed = secrets.protect(&random.bytes(length)).unwrap();
        for position in 0..sealed.len() {
            for bit in [0x01u8, 0x80] {
                let mut damaged = sealed.clone();
                damaged[position] ^= bit;
                assert!(
                    secrets.unprotect(&damaged).is_err(),
                    "byte {position} of {} ^ {bit:#x} opened",
                    sealed.len()
                );
            }
        }
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn any_truncation_or_extension_is_an_error() {
    let (secrets, dir) = secret_box("truncate");
    let sealed = secrets.protect(b"{\"some\":\"json\"}").unwrap();
    for length in 0..sealed.len() {
        assert!(
            secrets.unprotect(&sealed[..length]).is_err(),
            "{length} of {} bytes opened",
            sealed.len()
        );
    }
    let mut longer = sealed.clone();
    longer.push(0);
    assert!(secrets.unprotect(&longer).is_err());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn random_bytes_and_random_bytes_with_a_good_header_are_errors() {
    let (secrets, dir) = secret_box("garbage");
    let sealed = secrets.protect(b"x").unwrap();
    let mut random = Random(7);
    for case in 0..2000 {
        let length = random.below(200);
        let mut bytes = random.bytes(length);
        assert!(secrets.unprotect(&bytes).is_err(), "case {case}");
        // The header and key id of a real file, so it gets as far as the cipher.
        if bytes.len() < 60 {
            bytes.resize(60, 0);
        }
        bytes[..22].copy_from_slice(&sealed[..22]);
        assert!(
            secrets.unprotect(&bytes).is_err(),
            "case {case} with a header"
        );
    }
    let _ = fs::remove_dir_all(dir);
}

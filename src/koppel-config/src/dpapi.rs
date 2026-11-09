//! Windows DPAPI with machine scope: anything on this machine can decrypt, which is what lets a
//! LocalSystem service and an elevated console read the same files, and what survives a change
//! of service account. No prompt is ever shown.

use std::ptr;
use std::slice;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    CryptUnprotectData,
};
use windows::core::PCWSTR;

fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).expect("a secret under 4 GB"),
        // DPAPI only reads the input; the cast is what its signature asks for.
        pbData: bytes.as_ptr().cast_mut(),
    }
}

/// Copies DPAPI's output and frees it.
fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: DPAPI filled `out` with a buffer of `cbData` bytes from LocalAlloc, freed here once.
    unsafe {
        let bytes = slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(out.pbData.cast())));
        bytes
    }
}

/// Encrypts `plaintext` for this machine, with `entropy` mixed in.
pub fn protect(plaintext: &[u8], entropy: &[u8]) -> windows::core::Result<Vec<u8>> {
    let input = blob(plaintext);
    let entropy = blob(entropy);
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    // SAFETY: every pointer is to a live local for the length of the call.
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN | CRYPTPROTECT_LOCAL_MACHINE,
            &mut out,
        )?;
    }
    Ok(take(out))
}

/// Decrypts what [`protect`], or .NET's `ProtectedData`/Data Protection, made on this machine
/// with the same `entropy`.
pub fn unprotect(protected: &[u8], entropy: &[u8]) -> windows::core::Result<Vec<u8>> {
    let input = blob(protected);
    let entropy = blob(entropy);
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    // SAFETY: every pointer is to a live local for the length of the call.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )?;
    }
    Ok(take(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protect_round_trips_and_the_entropy_matters() {
        let protected = protect(b"hello", b"one").unwrap();
        assert_ne!(protected, b"hello");
        assert_eq!(unprotect(&protected, b"one").unwrap(), b"hello");
        assert!(unprotect(&protected, b"two").is_err());
    }
}

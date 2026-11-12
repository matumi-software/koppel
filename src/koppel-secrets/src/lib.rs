//! A value that must not end up in a log: [`Secret`]. A leaf crate (serde and zeroize only), so the
//! crates that carry a secret around without reading it need nothing else.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroize;

/// A secret value (a client secret, a private key, a password hash, a certificate with its key).
///
/// - `Debug` prints `Secret(<redacted>)`, so a `{:?}` of a config struct cannot leak it.
/// - The bytes are overwritten when the value is dropped, as far as `zeroize` can: a clone is a
///   second copy that is wiped when it is dropped in turn, and the `serde_json` values a file is
///   parsed through are not wiped (see "Threat model" in docs/security.md).
/// - It reads and writes as the value it holds, so the files do not change.
/// - It dereferences to the value, so `&secret` works where a `&str` or `&String` does.
///
/// There is no `Display`: a secret is put into a message only on purpose, through
/// [`Secret::expose`].
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The value. Named so that a use of it is easy to find.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl<T: Zeroize> Deref for Secret<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl AsRef<str> for Secret<String> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<[u8]> for Secret<String> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl<T: Zeroize + Clone> Clone for Secret<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Zeroize + Default> Default for Secret<T> {
    fn default() -> Self {
        Self(T::default())
    }
}

impl<T: Zeroize + PartialEq> PartialEq for Secret<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Zeroize + Eq> Eq for Secret<T> {}

impl<T: Zeroize> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret<String> {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl<T: Zeroize + Serialize> Serialize for Secret<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Zeroize + Deserialize<'de>> Deserialize<'de> for Secret<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value_also_inside_other_types() {
        let secret = Secret::new("hunter2".to_string());
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert!(!format!("{:?}", Some(&secret)).contains("hunter2"));
        assert!(!format!("{:#?}", vec![secret]).contains("hunter2"));
    }

    #[test]
    fn it_reads_and_writes_as_the_value_it_holds() {
        let secret: Secret<String> = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(&*secret, "abc");
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"abc\"");
        let none: Option<Secret<String>> = serde_json::from_str("null").unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn dropping_wipes_the_value() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Counts how often it was wiped.
        struct Probe(Arc<AtomicUsize>);
        impl Zeroize for Probe {
            fn zeroize(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let wiped = Arc::new(AtomicUsize::new(0));
        let secret = Secret::new(Probe(wiped.clone()));
        assert_eq!(wiped.load(Ordering::SeqCst), 0);
        drop(secret);
        assert_eq!(wiped.load(Ordering::SeqCst), 1);

        // And the real thing wipes a string in place.
        let mut text = String::from("secret");
        text.zeroize();
        assert!(text.is_empty());
    }
}

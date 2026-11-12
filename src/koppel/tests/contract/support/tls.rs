//! Trusting a target's self-signed certificate, and only that one.
//!
//! Koppel makes its own certificate on first start, so there is no CA to trust. The first certificate
//! a target presents is pinned by its SHA-256 fingerprint (trust on first use), and every later
//! handshake with that target must present a pinned one. The test starts the instance seconds before
//! it connects and nothing else is on the path, so the first sight is the instance.
//! A test that makes the target switch certificate pins the next one first ([`crate::Target::trust`]).
//! The handshake's signature is still checked against the certificate's key, so only the holder of the
//! pinned certificate passes. The name in the certificate is not checked: it is the target's machine
//! name, while the tests reach it as `localhost`, and the pin is the stronger check.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, ring as ring_provider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};

/// The fingerprints one target may present.
#[derive(Debug, Default)]
pub struct Pins {
    trusted: Mutex<Vec<String>>,
    last: Mutex<Option<String>>,
}

impl Pins {
    /// Adds a fingerprint, in any spelling with 64 hex digits.
    pub fn trust(&self, fingerprint: &str) {
        let fingerprint = normalize(fingerprint);
        let mut trusted = self.trusted.lock().unwrap_or_else(|e| e.into_inner());
        if !trusted.contains(&fingerprint) {
            trusted.push(fingerprint);
        }
    }

    /// The certificate the last handshake presented, pinned or not.
    pub fn last(&self) -> Option<String> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// True when `der` is pinned, or when nothing is pinned yet, which pins it.
    fn admit(&self, der: &[u8]) -> bool {
        let fingerprint = fingerprint_of(der);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(fingerprint.clone());
        let mut trusted = self.trusted.lock().unwrap_or_else(|e| e.into_inner());
        if trusted.is_empty() {
            trusted.push(fingerprint);
            return true;
        }
        trusted.contains(&fingerprint)
    }
}

/// The SHA-256 of a certificate as Koppel shows it: upper case hex pairs separated by colons.
pub fn fingerprint_of(der: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, der);
    let mut out = String::with_capacity(95);
    for (i, byte) in digest.as_ref().iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// A fingerprint in Koppel's spelling, from any spelling: its hex digits, upper case, in pairs.
pub fn normalize(fingerprint: &str) -> String {
    let hex: Vec<char> = fingerprint
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    hex.chunks(2)
        .map(|pair| pair.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(":")
}

struct PinnedVerifier {
    pins: Arc<Pins>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl std::fmt::Debug for PinnedVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinnedVerifier").finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self.pins.admit(end_entity.as_ref()) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// An HTTP client that trusts the target's pins and nothing else. It sends no cookies of its own (the
/// tests carry the session cookie by hand, to check what the target sets) and follows no redirects.
pub fn client(pins: Arc<Pins>) -> reqwest::Client {
    let provider = Arc::new(ring_provider::default_provider());
    let verifier = PinnedVerifier {
        pins,
        algorithms: provider.signature_verification_algorithms,
    };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("the ring provider speaks the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .no_proxy()
        // A target that restarts leaves a pooled connection dead, and Windows answers its reuse with a reset.
        .pool_max_idle_per_host(0)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("a client with this configuration always builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_reads_any_spelling() {
        assert_eq!(normalize("ab:cd-0f"), "AB:CD:0F");
        assert_eq!(normalize("abcd0f"), "AB:CD:0F");
    }

    #[test]
    fn the_first_certificate_is_pinned_and_another_is_refused() {
        let pins = Pins::default();
        assert!(pins.admit(b"first"));
        assert!(pins.admit(b"first"));
        assert!(!pins.admit(b"second"));
        pins.trust(&fingerprint_of(b"second"));
        assert!(pins.admit(b"second"));
        assert_eq!(pins.last(), Some(fingerprint_of(b"second")));
    }
}

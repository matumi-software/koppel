//! The server certificate in `secrets.json`: the certificate as DER and its private key as PKCS#8,
//! both base64, as Skakel's server stores them. Clients pin its SHA-256 fingerprint, and it is
//! loaded into what rustls presents.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::digest;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::CertifiedKey;
use skakel_server::CertificateRecord;

/// Why a certificate could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateError(pub String);

impl fmt::Display for CertificateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CertificateError {}

/// A certificate and its private key, as DER.
#[derive(Clone, PartialEq, Eq)]
pub struct ServerCertificate {
    /// The certificate, DER.
    pub certificate: Vec<u8>,
    /// Its private key, PKCS#8 DER: ECDSA P-256 for every certificate Koppel makes.
    pub private_key: Vec<u8>,
}

impl fmt::Debug for ServerCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerCertificate")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

impl ServerCertificate {
    /// Reads Skakel's record of a certificate: both parts are base64.
    pub fn from_record(record: &CertificateRecord) -> Result<Self, CertificateError> {
        let decode = |name: &str, text: &str| {
            STANDARD
                .decode(text.trim())
                .map_err(|e| CertificateError(format!("the {name} is not base64 ({e})")))
        };
        Ok(Self {
            certificate: decode("certificate", &record.certificate)?,
            private_key: decode("private key", &record.private_key)?,
        })
    }

    /// The SHA-256 of the DER, upper case hex in pairs with colons, as Skakel writes a fingerprint.
    pub fn fingerprint(&self) -> String {
        let hash = digest::digest(&digest::SHA256, &self.certificate);
        hash.as_ref()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    /// What rustls presents: the certificate with a signer for its key, RSA, ECDSA or Ed25519.
    pub fn certified_key(&self) -> Result<CertifiedKey, CertificateError> {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.private_key.clone()));
        let signer = rustls::crypto::ring::sign::any_supported_type(&key)
            .map_err(|e| CertificateError(format!("its private key cannot sign: {e}")))?;
        let certified =
            CertifiedKey::new(vec![CertificateDer::from(self.certificate.clone())], signer);
        certified.keys_match().map_err(|e| {
            CertificateError(format!("the private key is not the certificate's: {e}"))
        })?;
        Ok(certified)
    }
}

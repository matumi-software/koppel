//! The server certificate as 0.1.x keeps it in `secrets.json`: base64 PKCS#12, no password. Clients
//! pin its SHA-256 fingerprint, so it is carried over exactly, and loaded into what rustls presents.

use std::fmt;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::digest;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::CertifiedKey;

use crate::files::SkakelState;
use crate::pkcs12;

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
    /// Its private key, PKCS#8 DER: ECDSA P-256 for every certificate 0.1.x made.
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
    /// Reads a PKCS#12 file without a password. The certificate taken is the one whose
    /// `localKeyId` matches the key's, or the first when none do.
    pub fn from_pfx(pfx: &[u8]) -> Result<Self, CertificateError> {
        let contents = pkcs12::parse(pfx)
            .map_err(|e| CertificateError(format!("the PKCS#12 cannot be read: {e}")))?;
        let key = &contents.keys[0];
        let certificate = contents
            .certificates
            .iter()
            .find(|c| c.local_key_id.is_some() && c.local_key_id == key.local_key_id)
            .unwrap_or(&contents.certificates[0]);
        Ok(Self {
            certificate: certificate.der.clone(),
            private_key: key.der.clone(),
        })
    }

    /// Reads the base64 PKCS#12 that `SkakelState.CertificatePfx` holds.
    pub fn from_pfx_base64(text: &str) -> Result<Self, CertificateError> {
        let pfx = STANDARD
            .decode(text.trim())
            .map_err(|e| CertificateError(format!("it is not base64 ({e})")))?;
        Self::from_pfx(&pfx)
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

impl SkakelState {
    /// The certificate the server presents.
    pub fn certificate(&self) -> Result<ServerCertificate, CertificateError> {
        ServerCertificate::from_pfx_base64(&self.certificate_pfx)
    }

    /// The certificate prepared for a handover, if there is one.
    pub fn next_certificate(&self) -> Option<Result<ServerCertificate, CertificateError>> {
        self.next_certificate
            .as_ref()
            .map(|next| ServerCertificate::from_pfx_base64(&next.pfx))
    }

    /// The certificate as rustls presents it.
    pub fn certified_key(&self) -> Result<Arc<CertifiedKey>, CertificateError> {
        self.certificate()?.certified_key().map(Arc::new)
    }
}

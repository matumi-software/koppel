//! The TLS side: the certificate the server presents, which can be replaced while it runs, and the
//! rustls settings around it.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio_rustls::TlsAcceptor;

use crate::WebError;

/// The certificate the server presents. Every handshake reads the current one, so a replaced
/// certificate is what the next connection sees. Connections already open keep theirs.
pub struct CertificateSlot {
    current: RwLock<Arc<CertifiedKey>>,
}

impl CertificateSlot {
    pub fn new(certified: CertifiedKey) -> Arc<Self> {
        Arc::new(Self {
            current: RwLock::new(Arc::new(certified)),
        })
    }

    /// The certificate new connections get.
    pub fn current(&self) -> Arc<CertifiedKey> {
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Presents `certified` from the next handshake on.
    pub fn replace(&self, certified: CertifiedKey) {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(certified);
    }
}

impl fmt::Debug for CertificateSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CertificateSlot")
    }
}

impl ResolvesServerCert for CertificateSlot {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current())
    }
}

/// The acceptor for the web port, presenting whatever `slot` holds.
pub fn acceptor(slot: Arc<CertificateSlot>) -> Result<TlsAcceptor, WebError> {
    Ok(TlsAcceptor::from(Arc::new(server_config(slot)?)))
}

fn server_config(slot: Arc<CertificateSlot>) -> Result<ServerConfig, WebError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(WebError::Tls)?
        .with_no_client_auth()
        .with_cert_resolver(slot);
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    // No session resumption, as in Skakel's server. A resumed session shows the client no
    // certificate, and Skakel's certificate handover has a client prove the new certificate by the
    // one a new connection presents. rustls would also resume a session made under the old
    // certificate after a switch. A full handshake on a P-256 certificate costs next to nothing.
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    Ok(config)
}

/// A certificate made on the spot, with the DER a client needs to trust it.
pub struct SelfSigned {
    pub certified: CertifiedKey,
    pub der: CertificateDer<'static>,
}

/// A self-signed ECDSA P-256 certificate for `names`. A stand-in until Koppel loads its own
/// certificate from disk (#83), so it keeps rcgen's default validity.
pub fn self_signed(names: &[&str]) -> Result<SelfSigned, WebError> {
    use rcgen::{
        CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
    };

    let failed = |e: rcgen::Error| WebError::Certificate(e.to_string());
    let key = KeyPair::generate().map_err(failed)?;
    let mut params = CertificateParams::new(
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
    )
    .map_err(failed)?;
    params.distinguished_name.push(DnType::CommonName, "Koppel");
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let certificate = params.self_signed(&key).map_err(failed)?;

    let der = certificate.der().clone();
    let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let signing = rustls::crypto::ring::sign::any_ecdsa_type(&private).map_err(WebError::Tls)?;
    Ok(SelfSigned {
        certified: CertifiedKey::new(vec![der.clone()], signing),
        der,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_changes_what_resolve_gives() {
        let first = self_signed(&["localhost"]).unwrap();
        let second = self_signed(&["localhost"]).unwrap();
        let slot = CertificateSlot::new(first.certified);
        assert_eq!(slot.current().cert[0], first.der);

        slot.replace(second.certified);

        assert_eq!(slot.current().cert[0], second.der);
    }

    #[test]
    fn self_signed_makes_a_new_key_each_time() {
        let first = self_signed(&["localhost"]).unwrap();
        let second = self_signed(&["localhost"]).unwrap();
        assert_ne!(first.der, second.der);
    }

    #[test]
    fn server_config_offers_h2_and_http1_without_resumption() {
        let slot = CertificateSlot::new(self_signed(&["localhost"]).unwrap().certified);
        let config = server_config(slot).unwrap();

        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
        assert_eq!(config.send_tls13_tickets, 0);
        assert!(!config.session_storage.can_cache());
        assert!(!config.ticketer.enabled());
    }
}

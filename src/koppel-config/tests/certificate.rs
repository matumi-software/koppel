//! The certificate a 0.1.2 server kept, presented by koppel-web's `CertificateSlot`, so a client that
//! pinned it before the upgrade still finds it.

use std::sync::{Arc, Mutex};

use koppel_config::files::ServerSecrets;
use koppel_config::json;
use koppel_web::CertificateSlot;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Takes any certificate, as a pinning client does before it compares the fingerprint, but checks
/// the handshake signatures, so the server must hold the private key.
#[derive(Debug)]
struct Pinning {
    seen: Mutex<Option<Vec<u8>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinning {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.seen.lock().unwrap() = Some(end_entity.to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[tokio::test]
async fn koppel_web_presents_the_carried_over_certificate() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/linux-0.1.2/plaintext/secrets.json"
    ))
    .unwrap();
    let secrets: ServerSecrets = json::from_slice(&bytes).unwrap();
    let state = secrets.skakel.unwrap();
    let certificate = state.certificate().unwrap();
    let slot = CertificateSlot::new(certificate.certified_key().unwrap());
    let acceptor = koppel_web::tls::acceptor(slot).unwrap();

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(Pinning {
        seen: Mutex::new(None),
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

    let (client, server) = tokio::io::duplex(64 * 1024);
    let accept = tokio::spawn(async move { acceptor.accept(server).await.map(|_| ()) });
    let connected = connector
        .connect(ServerName::try_from("localhost").unwrap(), client)
        .await
        .unwrap();
    accept.await.unwrap().unwrap();
    drop(connected);

    let seen = verifier.seen.lock().unwrap().clone().unwrap();
    assert_eq!(seen, certificate.certificate);
    // What the client pinned in connection.json before the upgrade.
    assert_eq!(
        certificate.fingerprint(),
        "21:91:2B:41:C0:2D:38:79:4B:F9:D6:EA:17:EC:B9:33:20:1E:11:19:DD:62:08:E7:C1:62:F7:3E:14:19:BD:AA"
    );
}

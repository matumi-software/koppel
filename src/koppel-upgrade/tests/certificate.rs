//! The certificate a 0.1.2 server kept, after the upgrade, presented by koppel-web's
//! `CertificateSlot`, so a client that pinned it before the upgrade still finds it.

use std::path::Path;
use std::sync::{Arc, Mutex};

use koppel_config::{KoppelPaths, SecretBox, ServerCertificate, open};
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
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/linux-0.1.2");
    let dir = std::env::temp_dir().join(format!("koppel-upgrade-tls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&fixture.join("config"), &dir.join("config"));
    copy_dir(&fixture.join("keys"), &dir.join("keys"));
    let paths = KoppelPaths::new(dir.join("config"), dir.join("keys"));
    paths.prepare().unwrap();
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());
    koppel_upgrade::upgrade(&paths, &secrets).unwrap();
    let file = open::server_secrets(&paths, &secrets);
    file.start().await.unwrap();
    let state = file.current().skakel.clone().unwrap();
    file.stop();
    let certificate = ServerCertificate::from_record(&state.certificate).unwrap();
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
    let _ = std::fs::remove_dir_all(dir);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

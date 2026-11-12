//! `secrets.json` as the store of Skakel's server. The file holds skakel-server's own state: the
//! registered clients, their secret hashes and generations, the certificate and a prepared next
//! certificate. An older file (0.1.x, 0.2.0) was rewritten into that shape at start by `koppel-upgrade`.
//!
//! The certificate in the file is also what Koppel's own HTTPS ports present in the client role
//! ([`CertificateSlot`]): a save that changes it replaces what the next connection gets. While the
//! role is server or both they present it through Skakel's server's TLS settings instead.

use std::io;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use koppel_config::files::ServerSecrets;
use koppel_config::{ConfigFile, ServerCertificate};
use serde_json::json;
use skakel_server::{CertificateRecord, SkakelStore, StoreFuture};
use time::{Duration, OffsetDateTime};

use crate::tls::CertificateSlot;

/// A certificate lasts ten years, as Skakel makes them.
const CERTIFICATE_LIFETIME: Duration = Duration::days(3650);

/// `secrets.json`, read and written as skakel-server's state.
pub(crate) struct SecretsStore {
    file: ConfigFile<ServerSecrets>,
    slot: Arc<CertificateSlot>,
}

impl SecretsStore {
    /// Takes the started `secrets.json` and makes sure it holds a server state skakel-server can
    /// use: a file with nothing in it gets a new certificate. A file from an older Koppel was
    /// rewritten before this by `koppel-upgrade`. A token signing key that file may still hold
    /// is dropped by Skakel's server on its first save, since Skakel 0.3.0 does not sign tokens.
    pub(crate) async fn open(
        file: ConfigFile<ServerSecrets>,
        host_name: &str,
    ) -> Result<Arc<Self>, String> {
        let state = match file.current().skakel.clone() {
            Some(state) => state,
            None => {
                let state = new_state(host_name, OffsetDateTime::now_utc())?;
                file.update(|_| ServerSecrets::of(state.clone()))
                    .await
                    .map_err(|e| e.to_string())?;
                let certificate = certificate_of(&state.certificate)?;
                tracing::info!(
                    "Made the server certificate, fingerprint {}",
                    certificate.fingerprint()
                );
                state
            }
        };
        let certified = certificate_of(&state.certificate)?
            .certified_key()
            .map_err(|e| format!("the server certificate in secrets.json cannot be used: {e}"))?;
        Ok(Arc::new(Self {
            file,
            slot: CertificateSlot::new(certified),
        }))
    }

    /// What the web ports present. A save that switches the certificate replaces it.
    pub(crate) fn slot(&self) -> Arc<CertificateSlot> {
        self.slot.clone()
    }

    fn read(&self) -> io::Result<Option<skakel_server::SkakelState>> {
        Ok(self.file.current().skakel.clone())
    }

    async fn write(&self, state: &skakel_server::SkakelState) -> io::Result<()> {
        self.file
            .update(|_| ServerSecrets::of(state.clone()))
            .await
            .map_err(io::Error::other)?;
        let certificate = certificate_of(&state.certificate).map_err(io::Error::other)?;
        if self.slot.current().cert[0].as_ref() != certificate.certificate.as_slice() {
            let certified = certificate.certified_key().map_err(io::Error::other)?;
            self.slot.replace(certified);
            tracing::info!(
                "The web ports present the certificate {} from now on",
                certificate.fingerprint()
            );
        }
        Ok(())
    }
}

impl SkakelStore for SecretsStore {
    fn load(&self) -> StoreFuture<'_, Option<skakel_server::SkakelState>> {
        Box::pin(async move { self.read() })
    }

    fn save<'a>(&'a self, state: &'a skakel_server::SkakelState) -> StoreFuture<'a, ()> {
        Box::pin(async move { self.write(state).await })
    }
}

fn certificate_of(record: &CertificateRecord) -> Result<ServerCertificate, String> {
    ServerCertificate::from_record(record).map_err(|e| e.to_string())
}

/// A new server state: a certificate and no clients. Skakel's state cannot be built field by field
/// (its types are `#[non_exhaustive]`), so it is read from the JSON it keeps.
fn new_state(host_name: &str, now: OffsetDateTime) -> Result<skakel_server::SkakelState, String> {
    let certificate = new_certificate(host_name, now)?;
    serde_json::from_value(json!({
        "clients": [],
        "certificate": {
            "certificate": STANDARD.encode(&certificate.certificate),
            "privateKey": STANDARD.encode(&certificate.private_key),
        },
    }))
    .map_err(|e| format!("a new server state could not be made: {e}"))
}

/// A self-signed ECDSA P-256 certificate for this machine's name (and `localhost`), valid from a day
/// ago for ten years, as 0.1.x made them. Clients pin its fingerprint and ignore the names.
fn new_certificate(host_name: &str, now: OffsetDateTime) -> Result<ServerCertificate, String> {
    use rcgen::{
        CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
    };

    let failed = |e: rcgen::Error| format!("a certificate could not be made: {e}");
    let key = KeyPair::generate().map_err(failed)?;
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, host_name);
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Koppel");
    for name in [host_name, "localhost"] {
        if let Ok(name) = name.try_into() {
            params.subject_alt_names.push(SanType::DnsName(name));
        }
    }
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.not_before = now - Duration::days(1);
    params.not_after = now + CERTIFICATE_LIFETIME;
    let certificate = params.self_signed(&key).map_err(failed)?;
    Ok(ServerCertificate {
        certificate: certificate.der().to_vec(),
        private_key: key.serialize_der(),
    })
}

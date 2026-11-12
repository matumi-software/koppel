//! `secrets.json` as the store of Skakel's server. The file keeps the shape 0.1.x wrote (Skakel
//! 0.1's state, the certificates as PKCS#12), so the registered clients, their secret hashes and
//! generations, the certificate and a prepared next certificate carry over as they are; each load
//! and save converts between that and skakel-server's own state.
//!
//! The certificate in the file is also what Koppel's own HTTPS ports present in the client role
//! ([`CertificateSlot`]): a save that changes it replaces what the next connection gets. While the
//! role is server or both they present it through Skakel's server's TLS settings instead.

use std::io;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use koppel_config::files::{
    ServerSecrets, SkakelClientRecord, SkakelNextCertificateRecord, SkakelState,
};
use koppel_config::{ConfigFile, ServerCertificate};
use serde_json::{Value, json};
use skakel_server::{CertificateRecord, SkakelStore, StoreFuture};
use time::format_description::well_known::Rfc3339;
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
    /// use, as 0.1.x did on its first start: a file from Koppel 0.1.0 is carried over, and a file
    /// with nothing in it gets a new certificate. A 0.1.0 file that cannot be
    /// carried over is copied aside and the server starts afresh, which the log says. A token
    /// signing key from an earlier Koppel is dropped, since Skakel 0.3.0 no longer signs tokens.
    pub(crate) async fn open(
        file: ConfigFile<ServerSecrets>,
        host_name: &str,
    ) -> Result<Arc<Self>, String> {
        let current = file.current();
        let now = OffsetDateTime::now_utc();
        let mut state = match current.skakel_state(now) {
            Ok(Some(carried)) if current.skakel.is_some() => carried.state,
            Ok(Some(carried)) => {
                for dropped in &carried.dropped {
                    tracing::warn!(
                        "Client {} ({}) was added before Koppel kept secrets in their current form, so it was not carried over and has to pair again",
                        dropped.name,
                        dropped.id
                    );
                }
                tracing::info!("secrets.json from Koppel 0.1.0 was carried over");
                carried.state
            }
            Ok(None) => {
                let state = new_state(host_name, now)?;
                tracing::info!(
                    "Made the server certificate, fingerprint {}",
                    state
                        .certificate()
                        .map(|c| c.fingerprint())
                        .unwrap_or_default()
                );
                state
            }
            Err(reason) => {
                let aside = file
                    .path()
                    .with_file_name(format!("secrets.json.0.1.0-{}", compact_utc(now)));
                std::fs::copy(file.path(), &aside)
                    .map_err(|e| format!("{} could not be copied aside: {e}", aside.display()))?;
                tracing::warn!(
                    "secrets.json from Koppel 0.1.0 could not be carried over ({reason}). It was copied to {} and the server starts with a new certificate, a new signing key and no clients",
                    aside.display()
                );
                new_state(host_name, now)?
            }
        };
        // Skakel 0.3.0's tokens are random strings the server remembers, so a signing key an older
        // file holds is dropped here and does not stay on disk as a private key nobody uses.
        if state.signing_key.take().is_some()
            | !std::mem::take(&mut state.retired_signing_keys).is_empty()
        {
            tracing::info!(
                "The token signing key of an earlier Koppel was dropped: Skakel 0.3.0 does not sign tokens"
            );
        }
        if current.skakel.as_ref() != Some(&state) {
            file.update(|_| ServerSecrets::of(state.clone()))
                .await
                .map_err(|e| e.to_string())?;
        }
        let certified = state
            .certificate()
            .and_then(|c| c.certified_key())
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

    /// The state as skakel-server keeps it.
    fn read(&self) -> io::Result<Option<skakel_server::SkakelState>> {
        self.file
            .current()
            .skakel
            .as_ref()
            .map(to_skakel)
            .transpose()
            .map_err(io::Error::other)
    }

    async fn write(&self, state: &skakel_server::SkakelState) -> io::Result<()> {
        let mut written = None;
        let mut failed = None;
        self.file
            .update(
                |current| match from_skakel(state, current.skakel.as_ref()) {
                    Ok(next) => {
                        written = Some(next.clone());
                        ServerSecrets::of(next)
                    }
                    Err(e) => {
                        failed = Some(e);
                        current.clone()
                    }
                },
            )
            .await
            .map_err(io::Error::other)?;
        if let Some(e) = failed {
            return Err(io::Error::other(e));
        }
        let Some(written) = written else {
            return Ok(());
        };
        let certificate = written.certificate().map_err(io::Error::other)?;
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

/// 0.1.x's state as skakel-server's: the same clients, the certificates as DER. Skakel's state
/// cannot be built field by field (its types are `#[non_exhaustive]`), so it is read from the JSON
/// it keeps.
fn to_skakel(state: &SkakelState) -> Result<skakel_server::SkakelState, String> {
    let certificate = state
        .certificate()
        .map_err(|e| format!("the certificate in secrets.json cannot be read: {e}"))?;
    let next_certificate = match &state.next_certificate {
        Some(next) => {
            let prepared = ServerCertificate::from_pfx_base64(&next.pfx).map_err(|e| {
                format!("the prepared certificate in secrets.json cannot be read: {e}")
            })?;
            json!({
                "certificate": record_of(&prepared),
                "preparedAt": rfc3339(next.prepared_at)?,
                "receivedBy": next.received_by,
            })
        }
        None => Value::Null,
    };
    let mut clients = Vec::with_capacity(state.clients.len());
    for c in &state.clients {
        clients.push(json!({
            "id": c.id,
            "name": c.name,
            "secretHash": c.secret_hash,
            "createdAt": rfc3339(c.created_at)?,
            "generation": u32::try_from(c.generation).unwrap_or(0),
            "secret": c.secret,
        }));
    }
    serde_json::from_value(json!({
        "clients": clients,
        "certificate": record_of(&certificate),
        "nextCertificate": next_certificate,
    }))
    .map_err(|e| format!("the state in secrets.json is not one Skakel's server can read: {e}"))
}

fn rfc3339(at: OffsetDateTime) -> Result<String, String> {
    at.format(&Rfc3339).map_err(|e| e.to_string())
}

/// skakel-server's state in 0.1.x's shape. A certificate that did not change keeps the PKCS#12 the
/// file already holds, so a save does not rewrite it for nothing.
fn from_skakel(
    state: &skakel_server::SkakelState,
    current: Option<&SkakelState>,
) -> Result<SkakelState, String> {
    let certificate = certificate_of(&state.certificate)?;
    let certificate_pfx = match current {
        Some(current)
            if current
                .certificate()
                .is_ok_and(|c| c.certificate == certificate.certificate) =>
        {
            current.certificate_pfx.clone()
        }
        _ => certificate.to_pfx_base64().map_err(|e| e.0)?,
    };
    let next_certificate = match &state.next_certificate {
        Some(next) => {
            let next_certificate = certificate_of(&next.certificate)?;
            let pfx = match current.and_then(|c| c.next_certificate.as_ref()) {
                Some(kept)
                    if ServerCertificate::from_pfx_base64(&kept.pfx)
                        .is_ok_and(|c| c.certificate == next_certificate.certificate) =>
                {
                    kept.pfx.clone()
                }
                _ => next_certificate.to_pfx_base64().map_err(|e| e.0)?,
            };
            Some(SkakelNextCertificateRecord {
                pfx,
                prepared_at: next.prepared_at,
                received_by: next.received_by.clone(),
            })
        }
        None => None,
    };
    Ok(SkakelState {
        clients: state
            .clients
            .iter()
            .map(|c| SkakelClientRecord {
                id: c.id.clone(),
                name: c.name.clone(),
                secret_hash: c.secret_hash.clone(),
                created_at: c.created_at,
                generation: i32::try_from(c.generation).unwrap_or(i32::MAX),
                secret: c.secret.clone(),
            })
            .collect(),
        signing_key: None,
        retired_signing_keys: Vec::new(),
        certificate_pfx,
        next_certificate,
    })
}

fn record_of(certificate: &ServerCertificate) -> Value {
    json!({
        "certificate": STANDARD.encode(&certificate.certificate),
        "privateKey": STANDARD.encode(&certificate.private_key),
    })
}

fn certificate_of(record: &CertificateRecord) -> Result<ServerCertificate, String> {
    let decode = |text: &str| {
        STANDARD
            .decode(text)
            .map_err(|e| format!("a certificate from Skakel's server is not base64: {e}"))
    };
    Ok(ServerCertificate {
        certificate: decode(&record.certificate)?,
        private_key: decode(&record.private_key)?,
    })
}

/// A new server state: a certificate and no clients.
fn new_state(host_name: &str, now: OffsetDateTime) -> Result<SkakelState, String> {
    let certificate = new_certificate(host_name, now)?;
    Ok(SkakelState {
        clients: Vec::new(),
        signing_key: None,
        retired_signing_keys: Vec::new(),
        certificate_pfx: certificate.to_pfx_base64().map_err(|e| e.0)?,
        next_certificate: None,
    })
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

/// `yyyyMMddHHmmss` in UTC, for a file copied aside.
fn compact_utc(now: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_converts_to_skakels_and_back_unchanged() {
        let now = OffsetDateTime::now_utc();
        let mut state = new_state("bench", now).unwrap();
        state.clients.push(SkakelClientRecord {
            id: "client-1".into(),
            name: "lab pc".into(),
            secret_hash: "sha256$x".into(),
            created_at: now,
            generation: 3,
            secret: Some("s".into()),
        });
        let next = new_certificate("bench", now).unwrap();
        state.next_certificate = Some(SkakelNextCertificateRecord {
            pfx: next.to_pfx_base64().unwrap(),
            prepared_at: now,
            received_by: vec!["client-1".into()],
        });

        let skakel = to_skakel(&state).unwrap();
        let back = from_skakel(&skakel, Some(&state)).unwrap();

        assert_eq!(skakel.clients[0].generation, 3);
        assert_eq!(back, state, "unchanged certificates keep their PKCS#12");
        let fresh = from_skakel(&skakel, None).unwrap();
        assert_eq!(
            fresh.certificate().unwrap().certificate,
            state.certificate().unwrap().certificate
        );
    }
}

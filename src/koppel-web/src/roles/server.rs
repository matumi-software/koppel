//! The server role's state as 0.1.x kept it in `secrets.json`: the registered clients, the token
//! signing key, the certificate and the one prepared for a handover. It stands in for Skakel's server
//! until #85: everything the server's UI does with clients and keys works and is saved, but there is
//! no pairing, no token endpoint and no remote hub, so a client cannot use what it is given yet.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use koppel_config::files::{
    ServerSecrets, SkakelClientRecord, SkakelKeyRecord, SkakelNextCertificateRecord, SkakelState,
};
use koppel_config::{ConfigFile, ServerCertificate};
use ring::digest;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
use time::{Duration, OffsetDateTime};

use super::{
    Approval, ClientView, Fut, KeysView, NewClient, PairingOverview, RegisterError, RoleError,
    ServerRole, Switched,
};
use crate::activity::hex;
use crate::auth::random_bytes;
use crate::push::{self, Push};
use crate::tls::CertificateSlot;

/// A certificate lasts ten years, as Skakel makes them.
const CERTIFICATE_LIFETIME: Duration = Duration::days(3650);
/// How long a replaced signing key's tokens stay good: a token's lifetime.
const TOKEN_LIFETIME: Duration = Duration::minutes(30);

/// The server role over `secrets.json`. See the module docs.
pub struct StoredServer {
    file: ConfigFile<ServerSecrets>,
    slot: Arc<CertificateSlot>,
    push: Push,
    host_name: String,
}

impl StoredServer {
    /// Takes the started `secrets.json` and makes sure it holds a server state, as 0.1.x did on its
    /// first start: a file from Koppel 0.1.0 is carried over, and a file with nothing in it gets a new
    /// certificate and signing key. A 0.1.0 file that cannot be carried over is copied aside and the
    /// server starts afresh, which the log says.
    pub async fn open(
        file: ConfigFile<ServerSecrets>,
        host_name: &str,
        push: Push,
    ) -> Result<Arc<Self>, String> {
        let current = file.current();
        let now = OffsetDateTime::now_utc();
        let state = match current.skakel_state(now) {
            Ok(Some(carried)) if current.skakel.is_some() => carried.state,
            Ok(Some(carried)) => {
                for dropped in &carried.dropped {
                    tracing::warn!(
                        "Client {} ({}) was added before Koppel kept secrets in their current form, so it was not carried over and has to pair again",
                        dropped.name,
                        dropped.id
                    );
                }
                save(&file, carried.state.clone()).await?;
                tracing::info!("secrets.json from Koppel 0.1.0 was carried over");
                carried.state
            }
            Ok(None) => {
                let state = new_state(host_name, now)?;
                save(&file, state.clone()).await?;
                tracing::info!(
                    "Made the server certificate, fingerprint {}, and a token signing key",
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
                let state = new_state(host_name, now)?;
                save(&file, state.clone()).await?;
                state
            }
        };
        let certified = state
            .certificate()
            .and_then(|c| c.certified_key())
            .map_err(|e| format!("the server certificate in secrets.json cannot be used: {e}"))?;
        Ok(Arc::new(Self {
            file,
            slot: CertificateSlot::new(certified),
            push,
            host_name: host_name.to_owned(),
        }))
    }

    /// What the web ports present. A switch replaces it for the next connection.
    pub fn slot(&self) -> Arc<CertificateSlot> {
        self.slot.clone()
    }

    fn state(&self) -> SkakelState {
        self.file
            .current()
            .skakel
            .clone()
            .expect("open made sure secrets.json has a server state")
    }

    /// Changes the state and writes it, then gives back what `change` said.
    async fn change<R: Send>(
        &self,
        change: impl FnOnce(&mut SkakelState) -> R + Send,
    ) -> Result<R, RoleError> {
        let mut result = None;
        self.file
            .update(|current| {
                let mut state = current
                    .skakel
                    .clone()
                    .expect("open made sure secrets.json has a server state");
                result = Some(change(&mut state));
                ServerSecrets::of(state)
            })
            .await
            .map_err(|e| RoleError(e.to_string()))?;
        Ok(result.expect("the change ran"))
    }

    fn push_clients(&self) {
        self.push.send(push::CLIENTS_CHANGED, &self.clients());
    }

    fn push_keys(&self) {
        self.push.send(push::SERVER_KEYS_CHANGED, &self.keys());
    }
}

impl ServerRole for StoredServer {
    fn fingerprint(&self) -> String {
        digest::digest(&digest::SHA256, &self.slot.current().cert[0])
            .as_ref()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    fn clients(&self) -> Vec<ClientView> {
        let state = self.state();
        let received: &[String] = state
            .next_certificate
            .as_ref()
            .map_or(&[], |next| next.received_by.as_slice());
        state
            .clients
            .iter()
            .map(|c| ClientView {
                id: c.id.clone(),
                name: c.name.clone(),
                created_at: c.created_at,
                last_seen: None,
                has_next_certificate: received.contains(&c.id),
                has_secret: c.secret.is_some(),
            })
            .collect()
    }

    fn name_of(&self, id: &str) -> Option<String> {
        self.state()
            .clients
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.name)
    }

    fn secret_of(&self, id: &str) -> Option<Option<String>> {
        self.state()
            .clients
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.secret)
    }

    fn register<'a>(&'a self, name: &'a str) -> Fut<'a, Result<NewClient, RegisterError>> {
        Box::pin(async move {
            let name = clean_name(name);
            if name.is_empty() {
                return Err(RegisterError::EmptyName);
            }
            let id = format!("client-{}", hex(&random_bytes::<5>()));
            let secret = URL_SAFE_NO_PAD.encode(random_bytes::<32>());
            let record = SkakelClientRecord {
                id: id.clone(),
                name: name.clone(),
                secret_hash: secret_hash(&secret),
                created_at: OffsetDateTime::now_utc(),
                generation: 1,
                secret: Some(secret.clone()),
            };
            self.change(|state| state.clients.push(record))
                .await
                .map_err(RegisterError::Failed)?;
            self.push_clients();
            Ok(NewClient {
                client_id: id,
                client_secret: secret,
                name,
            })
        })
    }

    fn revoke<'a>(&'a self, id: &'a str) -> Fut<'a, Result<bool, RoleError>> {
        Box::pin(async move {
            if self.name_of(id).is_none() {
                return Ok(false);
            }
            let removed = self
                .change(|state| {
                    let before = state.clients.len();
                    state.clients.retain(|c| c.id != id);
                    if let Some(next) = &mut state.next_certificate {
                        next.received_by.retain(|r| r != id);
                    }
                    state.clients.len() != before
                })
                .await?;
            if removed {
                self.push_clients();
            }
            Ok(removed)
        })
    }

    fn rotate_secret<'a>(&'a self, id: &'a str) -> Fut<'a, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            if self.name_of(id).is_none() {
                return Ok(None);
            }
            let secret = URL_SAFE_NO_PAD.encode(random_bytes::<32>());
            let rotated = self
                .change(|state| {
                    let client = state.clients.iter_mut().find(|c| c.id == id)?;
                    client.secret_hash = secret_hash(&secret);
                    client.secret = Some(secret.clone());
                    client.generation += 1;
                    Some(secret.clone())
                })
                .await?;
            if rotated.is_some() {
                self.push_clients();
            }
            Ok(rotated)
        })
    }

    fn keys(&self) -> KeysView {
        let state = self.state();
        KeysView {
            fingerprint: self.fingerprint(),
            next_fingerprint: state
                .next_certificate()
                .and_then(Result::ok)
                .map(|c| c.fingerprint()),
            next_prepared_at: state.next_certificate.as_ref().map(|n| n.prepared_at),
            signing_key_id: state.signing_key.key_id.clone(),
            signing_key_since: state.signing_key.created_at,
            retired_keys_until: state
                .retired_signing_keys
                .iter()
                .filter_map(|k| k.accept_until)
                .max(),
        }
    }

    fn rotate_signing_key(&self) -> Fut<'_, Result<String, RoleError>> {
        Box::pin(async move {
            let now = OffsetDateTime::now_utc();
            let key = new_signing_key(now)?;
            let id = key.key_id.clone();
            self.change(|state| {
                let mut old = std::mem::replace(&mut state.signing_key, key);
                old.accept_until = Some(now + TOKEN_LIFETIME);
                state
                    .retired_signing_keys
                    .retain(|k| k.accept_until.is_some_and(|until| until > now));
                state.retired_signing_keys.push(old);
            })
            .await?;
            self.push_keys();
            Ok(id)
        })
    }

    fn prepare_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            if self.state().next_certificate.is_some() {
                return Ok(None);
            }
            let now = OffsetDateTime::now_utc();
            let certificate = new_certificate(&self.host_name, now).map_err(RoleError)?;
            let fingerprint = certificate.fingerprint();
            let pfx = certificate.to_pfx_base64().map_err(|e| RoleError(e.0))?;
            let prepared = self
                .change(|state| {
                    if state.next_certificate.is_some() {
                        return false;
                    }
                    state.next_certificate = Some(SkakelNextCertificateRecord {
                        pfx,
                        prepared_at: now,
                        received_by: Vec::new(),
                    });
                    true
                })
                .await?;
            if !prepared {
                return Ok(None);
            }
            self.push_keys();
            Ok(Some(fingerprint))
        })
    }

    fn cancel_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            let Some(next) = self.keys().next_fingerprint else {
                return Ok(None);
            };
            let cancelled = self
                .change(|state| state.next_certificate.take().is_some())
                .await?;
            if !cancelled {
                return Ok(None);
            }
            self.push_keys();
            self.push_clients();
            Ok(Some(next))
        })
    }

    fn switch_certificate(&self) -> Fut<'_, Result<Option<Switched>, RoleError>> {
        Box::pin(async move {
            let Some(next) = self.state().next_certificate else {
                return Ok(None);
            };
            let certificate = ServerCertificate::from_pfx_base64(&next.pfx)
                .map_err(|e| RoleError(format!("the prepared certificate cannot be read: {e}")))?;
            let certified = certificate
                .certified_key()
                .map_err(|e| RoleError(format!("the prepared certificate cannot be used: {e}")))?;
            let not_received = self
                .change(|state| {
                    let next = state.next_certificate.take()?;
                    let not_received = state
                        .clients
                        .iter()
                        .filter(|c| !next.received_by.contains(&c.id))
                        .map(|c| c.name.clone())
                        .collect::<Vec<_>>();
                    state.certificate_pfx = next.pfx;
                    Some(not_received)
                })
                .await?;
            let Some(not_received) = not_received else {
                return Ok(None);
            };
            self.slot.replace(certified);
            self.push_keys();
            self.push_clients();
            Ok(Some(Switched {
                to: certificate.fingerprint(),
                not_received,
            }))
        })
    }

    fn pairing(&self) -> PairingOverview {
        PairingOverview {
            requests: Vec::new(),
            budget_exhausted: false,
        }
    }

    fn approve<'a>(&'a self, _id: &'a str, _code: &'a str) -> Fut<'a, Approval> {
        Box::pin(async { Approval::NotFound })
    }

    fn deny<'a>(&'a self, _id: &'a str) -> Fut<'a, bool> {
        Box::pin(async { false })
    }
}

async fn save(file: &ConfigFile<ServerSecrets>, state: SkakelState) -> Result<(), String> {
    file.update(|_| ServerSecrets::of(state))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A new server state: a certificate, a signing key and no clients.
fn new_state(host_name: &str, now: OffsetDateTime) -> Result<SkakelState, String> {
    let certificate = new_certificate(host_name, now)?;
    Ok(SkakelState {
        clients: Vec::new(),
        signing_key: new_signing_key(now).map_err(|e| e.0)?,
        retired_signing_keys: Vec::new(),
        certificate_pfx: certificate.to_pfx_base64().map_err(|e| e.0)?,
        next_certificate: None,
    })
}

/// A self-signed ECDSA P-256 certificate for this machine's name (and `localhost`), valid from a day
/// ago for ten years, as Skakel makes them. Clients pin its fingerprint and ignore the names.
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

/// A token signing key. 0.1.x signed with RSA; Skakel's Rust server signs with ECDSA P-256 (#85),
/// so a key made now is one.
fn new_signing_key(now: OffsetDateTime) -> Result<SkakelKeyRecord, RoleError> {
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &SystemRandom::new())
        .map_err(|_| RoleError("a signing key could not be made".into()))?;
    Ok(SkakelKeyRecord {
        key_id: format!("skakel-{}", hex(&random_bytes::<5>())),
        private_key: STANDARD.encode(pkcs8.as_ref()),
        created_at: now,
        accept_until: None,
    })
}

/// `sha256$` and the base64 SHA-256 of the secret, as Skakel keeps it.
fn secret_hash(secret: &str) -> String {
    format!(
        "sha256${}",
        STANDARD.encode(digest::digest(&digest::SHA256, secret.as_bytes()))
    )
}

/// The name with control and format characters taken out, as Skakel cleans it, and trimmed.
pub(crate) fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && !is_format(*c))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Unicode's format characters (category Cf), which the standard library cannot tell: soft hyphens,
/// zero-width and direction marks, the byte order mark and the like.
fn is_format(c: char) -> bool {
    const RANGES: &[(u32, u32)] = &[
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    let code = u32::from(c);
    RANGES
        .iter()
        .any(|(low, high)| (*low..=*high).contains(&code))
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
    fn names_lose_control_and_format_characters() {
        assert_eq!(clean_name("  lab pc  "), "lab pc");
        assert_eq!(clean_name("\u{200B}\u{200B}"), "");
        assert_eq!(clean_name("a\u{0007}b\u{FEFF}c"), "abc");
        assert_eq!(clean_name("é 😀"), "é 😀");
    }

    #[test]
    fn a_new_state_has_a_certificate_that_reads_back() {
        let now = OffsetDateTime::now_utc();
        let state = new_state("bench", now).unwrap();
        let certificate = state.certificate().unwrap();
        assert!(certificate.certified_key().is_ok());
        assert_eq!(certificate.fingerprint().len(), 95);
        assert!(state.signing_key.key_id.starts_with("skakel-"));
        assert!(secret_hash("x").starts_with("sha256$"));
    }
}

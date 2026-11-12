//! The one-time rewrite of `secrets.json` into the shape Koppel 0.2.1 stores: Skakel's own server
//! state, with each certificate as `{ certificate, privateKey }` (DER and PKCS#8, base64).
//!
//! Three older shapes are read, each only here:
//!
//! - Koppel 0.2.0 and 0.1.1/0.1.2: `skakel` holds Skakel 0.1's state, with `certificatePfx` and
//!   `nextCertificate.pfx` as base64 PKCS#12 (the 3DES/SHA-1 files .NET exports, and the ones 0.2.0
//!   wrote in the same shape), a token signing key that nothing uses any more, and clients whose
//!   `generation` is a signed number. The old fields are read once and the file is written again
//!   without them.
//! - Koppel 0.1.0: no `skakel`; `signingKey`, `certificatePfx`, `clients` and `nextCertificate` sit at
//!   the top. Clients with a 0.1.0 PBKDF2 secret hash cannot be checked by Skakel and pair again. A file
//!   whose key or certificate cannot be read is moved aside and Koppel starts as a new install, as
//!   0.1.1 did.
//!
//! A file already in the new shape, and a file with no state in it, are left alone. The certificate
//! itself does not change, so the fingerprint clients pinned stays.

use std::fs;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use koppel_config::files::ServerSecrets;
use koppel_config::{KoppelPaths, SecretBox, SecretFormat, encode_json, format_of, replace_file};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;

use crate::migrate::ConversionError;
use crate::pkcs12;

/// What the rewrite did to `secrets.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reshaped {
    /// The file is missing, empty of state, or already in the current shape.
    Nothing,
    /// The file was written again in the current shape.
    Rewritten {
        /// Clients of a 0.1.0 file that could not be carried over, by name and id.
        dropped: Vec<(String, String)>,
    },
    /// A 0.1.0 file that could not be carried over was moved aside, unchanged, as this.
    StartedFresh(PathBuf),
}

/// Rewrites `secrets.json` if it is in an older shape. Run after [`crate::convert_0_1_secrets`], so the
/// file is already sealed. Logs what it did.
pub fn reshape_secrets(
    paths: &KoppelPaths,
    secrets: &SecretBox,
) -> Result<Reshaped, ConversionError> {
    let file = paths.server_secrets_file();
    let result = reshape(&file, secrets);
    match &result {
        Ok(Reshaped::Nothing) => {}
        Ok(Reshaped::Rewritten { dropped }) => {
            for (name, id) in dropped {
                tracing::warn!(
                    "Client {name} ({id}) was added before Koppel kept secrets in their current form, so it was not carried over and has to pair again"
                );
            }
            tracing::info!(
                "{} was rewritten in the format Koppel 0.2.1 stores: the certificate as DER and a key, not PKCS#12",
                file.display()
            );
        }
        Ok(Reshaped::StartedFresh(aside)) => tracing::warn!(
            "secrets.json from Koppel 0.1.0 could not be carried over. It was kept, unchanged, as {}, and the server starts with a new certificate and no clients",
            aside.display()
        ),
        Err(e) => tracing::error!("{e}. The file was left as it was"),
    }
    result
}

fn reshape(file: &std::path::Path, secrets: &SecretBox) -> Result<Reshaped, ConversionError> {
    let failed = |reason: String| ConversionError {
        file: Some(file.to_path_buf()),
        reason,
    };
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Reshaped::Nothing),
        Err(e) => return Err(failed(format!("it could not be read ({e})"))),
    };
    if format_of(&bytes) != SecretFormat::Current {
        return Ok(Reshaped::Nothing);
    }
    let plain = secrets
        .unprotect(&bytes)
        .map_err(|e| failed(format!("it could not be opened ({e})")))?;
    let Ok(Value::Object(root)) =
        serde_json::from_slice(plain.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&plain))
    else {
        return Ok(Reshaped::Nothing);
    };

    let (state, dropped) = match root.get("skakel") {
        Some(Value::Object(state)) if state.contains_key("certificatePfx") => {
            (state_of(state).map_err(failed)?, Vec::new())
        }
        Some(Value::Object(_)) => return Ok(Reshaped::Nothing),
        _ if root.get("certificatePfx").is_some_and(Value::is_string) => match carry_over(&root) {
            Ok(carried) => carried,
            Err(reason) => {
                tracing::warn!("secrets.json from Koppel 0.1.0 cannot be read: {reason}");
                return aside(file).map(Reshaped::StartedFresh).map_err(failed);
            }
        },
        _ => return Ok(Reshaped::Nothing),
    };

    // The daemon's own type has to read what is written.
    let typed: ServerSecrets = serde_json::from_value(json!({ "skakel": state })).map_err(|e| {
        failed(format!(
            "the state is not one Skakel's server can read ({e})"
        ))
    })?;
    let sealed = secrets
        .protect(&encode_json(&typed))
        .map_err(|e| failed(format!("it could not be encrypted again ({e})")))?;
    replace_file(file, &sealed).map_err(|e| failed(format!("it could not be written ({e})")))?;
    Ok(Reshaped::Rewritten { dropped })
}

/// Skakel 0.1's state, the shape 0.1.1 to 0.2.0 wrote, as the state Koppel stores now.
fn state_of(old: &Map<String, Value>) -> Result<Value, String> {
    let pfx = old
        .get("certificatePfx")
        .and_then(Value::as_str)
        .ok_or("certificatePfx is not text")?;
    let certificate =
        record_of_pfx(pfx).map_err(|e| format!("the certificate cannot be read ({e})"))?;
    let next_certificate = match old.get("nextCertificate") {
        Some(Value::Object(next)) => {
            let pfx = next
                .get("pfx")
                .and_then(Value::as_str)
                .ok_or("the prepared certificate has no pfx")?;
            json!({
                "certificate": record_of_pfx(pfx)
                    .map_err(|e| format!("the prepared certificate cannot be read ({e})"))?,
                "preparedAt": next.get("preparedAt").cloned().unwrap_or(Value::Null),
                "receivedBy": next.get("receivedBy").cloned().unwrap_or_else(|| json!([])),
            })
        }
        _ => Value::Null,
    };
    let mut clients = Vec::new();
    for client in old
        .get("clients")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let client = client.as_object().ok_or("a client is not an object")?;
        let mut kept = Map::new();
        for name in ["id", "name", "secretHash", "createdAt"] {
            let value = client
                .get(name)
                .ok_or_else(|| format!("a client has no {name}"))?;
            kept.insert(name.to_owned(), value.clone());
        }
        // Skakel counts generations from zero upward in a `u32`; 0.1.x wrote an `int`.
        let generation = client
            .get("generation")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .clamp(0, i64::from(u32::MAX));
        kept.insert("generation".to_owned(), json!(generation));
        if let Some(secret) = client.get("secret").filter(|v| !v.is_null()) {
            kept.insert("secret".to_owned(), secret.clone());
        }
        clients.push(Value::Object(kept));
    }
    Ok(json!({
        "clients": clients,
        "certificate": certificate,
        "nextCertificate": next_certificate,
    }))
}

/// Koppel 0.1.0's `secrets.json`, carried over the way 0.1.1 did: the clients whose secret hash Skakel
/// can check, the certificate, and a prepared certificate with the clients that stayed. The token
/// signing key must still be a readable RSA key, as 0.1.1 asked, though nothing uses it now.
fn carry_over(root: &Map<String, Value>) -> Result<(Value, Vec<(String, String)>), String> {
    let signing_key = root
        .get("signingKey")
        .and_then(Value::as_str)
        .ok_or("the signing key is missing")?;
    let key = STANDARD
        .decode(signing_key)
        .map_err(|e| format!("the signing key is not base64 ({e})"))?;
    ring::rsa::KeyPair::from_pkcs8(&key)
        .map_err(|e| format!("the signing key is not an RSA PKCS#8 key ({e})"))?;

    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for client in root
        .get("clients")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let hash = client.get("secretHash").and_then(Value::as_str);
        if hash.is_some_and(|h| h.starts_with("sha256$")) {
            kept.push(client.clone());
        } else {
            let text = |name| {
                client
                    .get(name)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            dropped.push((text("name"), text("id")));
        }
    }
    let kept_ids: Vec<&str> = kept
        .iter()
        .filter_map(|c| c.get("id").and_then(Value::as_str))
        .collect();
    let next_certificate = match root.get("nextCertificate") {
        Some(Value::Object(next)) => {
            let mut next = next.clone();
            let received: Vec<Value> = next
                .get("receivedBy")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter(|id| id.as_str().is_some_and(|id| kept_ids.contains(&id)))
                .cloned()
                .collect();
            next.insert("receivedBy".to_owned(), Value::Array(received));
            Value::Object(next)
        }
        _ => Value::Null,
    };
    let old = json!({
        "clients": kept,
        "certificatePfx": root.get("certificatePfx"),
        "nextCertificate": next_certificate,
    });
    let old = old.as_object().expect("an object");
    Ok((state_of(old)?, dropped))
}

/// `{ certificate, privateKey }` of a PKCS#12 with no password: base64 DER and base64 PKCS#8. The
/// certificate taken is the one whose `localKeyId` matches the key's, or the first when none do.
fn record_of_pfx(text: &str) -> Result<Value, String> {
    let pfx = STANDARD
        .decode(text.trim())
        .map_err(|e| format!("it is not base64 ({e})"))?;
    let contents = pkcs12::parse(&pfx).map_err(|e| format!("the PKCS#12 cannot be read: {e}"))?;
    let key = &contents.keys[0];
    let certificate = contents
        .certificates
        .iter()
        .find(|c| c.local_key_id.is_some() && c.local_key_id == key.local_key_id)
        .unwrap_or(&contents.certificates[0]);
    Ok(json!({
        "certificate": STANDARD.encode(&certificate.der),
        "privateKey": STANDARD.encode(&key.der),
    }))
}

/// Moves a file that cannot be carried over aside, unchanged, as `secrets.json.0.1.0-yyyyMMddHHmmss`.
fn aside(file: &std::path::Path) -> Result<PathBuf, String> {
    let now = OffsetDateTime::now_utc();
    let mut name = file.as_os_str().to_owned();
    name.push(format!(
        ".0.1.0-{:04}{:02}{:02}{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    ));
    let aside = PathBuf::from(name);
    fs::rename(file, &aside).map_err(|e| format!("it could not be moved aside ({e})"))?;
    Ok(aside)
}

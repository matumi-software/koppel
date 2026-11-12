//! The one-time conversion of a 0.1.x install's secret files from Data Protection to Koppel's own
//! sealed format.
//!
//! `secrets.json`, `connection.json` and `ui-auth.json` that are still Data Protection payloads are
//! decrypted with the 0.1.x key ring and written again with the [`SecretBox`], with the same JSON
//! inside, byte for byte. Every file is decrypted before any is written, so a failure leaves all of
//! them as they were and says why; each file is then replaced on its own through a temp file, so a
//! file is only ever the old one or the whole new one, and a start after a crash part way converts
//! what is left. The key ring is never changed.
//!
//! A key ring protected with a key certificate (`Paths:KeyCertificate`, deprecated in 0.1.2) is not
//! read. Its files are renamed aside, unchanged, and Koppel starts as a new install would: a new
//! certificate and signing key, no clients, no connection and no UI password. Every client pairs
//! again, as the 0.1.2 changelog announced.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use koppel_config::{KoppelPaths, SecretBox, SecretFormat, format_of, replace_file, utc_stamp};
use time::OffsetDateTime;

use crate::dataprotection::{DataProtectionError, KOPPEL_PURPOSES, KeyRing};

/// What the conversion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversion {
    /// No file was in 0.1.x's format.
    NothingToDo,
    /// These files were written again in the new format.
    Converted(Vec<PathBuf>),
    /// The key ring is protected with a key certificate. These files were renamed (from, to) and
    /// Koppel starts without them.
    PairAgain(Vec<(PathBuf, PathBuf)>),
}

/// Why the conversion did not happen. The 0.1.x files are as they were.
#[derive(Debug)]
pub struct ConversionError {
    pub file: Option<PathBuf>,
    pub reason: String,
}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.file {
            Some(file) => write!(
                f,
                "{} could not be converted from Koppel 0.1.x: {}",
                file.display(),
                self.reason
            ),
            None => write!(
                f,
                "the Koppel 0.1.x secret files could not be converted: {}",
                self.reason
            ),
        }
    }
}

impl std::error::Error for ConversionError {}

/// Converts what is left of a 0.1.x install. Run once at start, after [`KoppelPaths::prepare`] and
/// before the secret files are opened ([`crate::upgrade`] does that). Logs what it did, and why when
/// it could not.
pub fn convert_0_1_secrets(
    paths: &KoppelPaths,
    secrets: &SecretBox,
) -> Result<Conversion, ConversionError> {
    let result = convert(paths, secrets);
    match &result {
        Ok(Conversion::NothingToDo) => {}
        Ok(Conversion::Converted(files)) => {
            for file in files {
                tracing::info!(
                    "{} was converted from Koppel 0.1.x's format",
                    file.display()
                );
            }
            tracing::info!(
                "The Koppel 0.1.x key ring in {} is no longer used and was left in place, as Koppel never deletes it.                  It holds the key to Koppel 0.1.x's old files, in the clear on Linux: once those files are backed up, or no downgrade is wanted, delete the key-*.xml files there",
                paths.keys.display()
            );
        }
        Ok(Conversion::PairAgain(moved)) => {
            tracing::error!(
                "The Koppel 0.1.x key ring in {} is protected with a key certificate (Paths:KeyCertificate), which Koppel no longer reads. \
                 Starting as a new install: a new certificate and signing key, no clients, no connection and no UI password. \
                 Pair every client again and set the password in the UI",
                paths.keys.display()
            );
            for (from, to) in moved {
                tracing::warn!(
                    "{} was kept, unchanged, as {}",
                    from.display(),
                    to.display()
                );
            }
        }
        Err(e) => tracing::error!("{e}. The 0.1.x files were left as they were"),
    }
    if paths.key_certificate.is_some() {
        tracing::warn!("Paths:KeyCertificate is set. Koppel no longer uses it: remove the setting");
    }
    result
}

fn convert(paths: &KoppelPaths, secrets: &SecretBox) -> Result<Conversion, ConversionError> {
    let mut old = Vec::new();
    for file in paths.secret_files() {
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(ConversionError {
                    file: Some(file),
                    reason: format!("it could not be read ({e})"),
                });
            }
        };
        if format_of(&bytes) == SecretFormat::DataProtection {
            old.push((file, bytes));
        }
    }
    if old.is_empty() {
        return Ok(Conversion::NothingToDo);
    }

    let ring = KeyRing::load(&paths.keys).map_err(|e| ConversionError {
        file: None,
        reason: format!(
            "the key ring in {} cannot be read: {e}",
            paths.keys.display()
        ),
    })?;

    let mut plain = Vec::with_capacity(old.len());
    for (file, bytes) in &old {
        match ring.unprotect(&KOPPEL_PURPOSES, bytes) {
            Ok(json) => {
                if let Err(e) = serde_json::from_slice::<serde_json::Value>(
                    json.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&json),
                ) {
                    return Err(ConversionError {
                        file: Some(file.clone()),
                        reason: format!("it decrypted to something that is not JSON ({e})"),
                    });
                }
                plain.push((file.clone(), json));
            }
            Err(DataProtectionError::KeyCertificate(_)) => return pair_again(&old),
            Err(e) => {
                return Err(ConversionError {
                    file: Some(file.clone()),
                    reason: e.to_string(),
                });
            }
        }
    }

    let mut sealed = Vec::with_capacity(plain.len());
    for (file, json) in plain {
        let bytes = secrets.protect(&json).map_err(|e| ConversionError {
            file: Some(file.clone()),
            reason: format!("it could not be encrypted again ({e})"),
        })?;
        sealed.push((file, bytes));
    }
    let mut converted = Vec::with_capacity(sealed.len());
    for (file, bytes) in sealed {
        replace_file(&file, &bytes).map_err(|e| ConversionError {
            file: Some(file.clone()),
            reason: format!("it could not be written ({e})"),
        })?;
        converted.push(file);
    }
    Ok(Conversion::Converted(converted))
}

/// Renames the 0.1.x files aside, unchanged, so Koppel starts without them.
fn pair_again(old: &[(PathBuf, Vec<u8>)]) -> Result<Conversion, ConversionError> {
    let stamp = utc_stamp(OffsetDateTime::now_utc());
    let mut moved = Vec::with_capacity(old.len());
    for (file, _) in old {
        let mut aside = file.clone().into_os_string();
        aside.push(format!(".0.1-key-certificate-{stamp}"));
        let aside = PathBuf::from(aside);
        fs::rename(file, &aside).map_err(|e| ConversionError {
            file: Some(file.clone()),
            reason: format!(
                "its key ring uses a key certificate, and it could not be moved aside ({e})"
            ),
        })?;
        moved.push((file.clone(), aside));
    }
    Ok(Conversion::PairAgain(moved))
}

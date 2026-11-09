//! Reads what ASP.NET Core Data Protection wrote for Koppel 0.1.x: the key ring and the protected
//! payloads. Only reading; nothing here writes that format, except the tests.
//!
//! The key ring is one `key-{guid}.xml` per key in the keys directory. A key's master key is
//! plain base64 on Linux, and on Windows the `masterKey` element is protected with machine DPAPI
//! by `DpapiXmlEncryptor`. A key ring protected with a key certificate (`Paths:KeyCertificate`)
//! is recognised and not read.
//!
//! A payload is `09 F0 C9 F0`, the key's id (a .NET `Guid` in its own byte order), then what the
//! CBC authenticated encryptor wrote: a 16 byte key modifier, the IV, the AES-256-CBC ciphertext
//! and an HMAC-SHA256 over the IV and the ciphertext. The two keys come from the master key through
//! SP800-108 in counter mode with HMAC-SHA512, with the additional authenticated data as the label
//! and the encryptor's context header and the key modifier as the context. The additional data is
//! the magic header, the key id, and the purposes: their count as a big-endian 32 bit number, then
//! each as a 7 bit encoded length and UTF-8. `SetApplicationName("Koppel")` makes "Koppel" the first
//! purpose, ahead of the one the protector was made with.
//!
//! Sources: Microsoft's "Authenticated encryption details", "Subkey derivation and authenticated
//! encryption" and "Context headers" pages, and in dotnet/aspnetcore `KeyRingBasedDataProtector`,
//! `ManagedAuthenticatedEncryptor`, `ManagedSP800_108_CTR_HMACSHA512`, `DpapiXmlEncryptor` and
//! `DpapiSecretSerializerHelper`. Checked against key rings and files 0.1.2 wrote, on Linux and on
//! Windows.

use std::fmt;
use std::fs;
use std::path::Path;

use aes::Aes256;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use ring::hmac;
use time::OffsetDateTime;

/// What every Data Protection payload starts with (`MAGIC_HEADER_V0`).
pub const MAGIC: [u8; 4] = [0x09, 0xF0, 0xC9, 0xF0];

/// The purposes Koppel 0.1.x protected its secret files with: the application name, then
/// `SecretBoxFactory`'s purpose.
pub const KOPPEL_PURPOSES: [&str; 2] = ["Koppel", "Koppel.Secrets.v1"];

/// The entropy `DpapiSecretSerializerHelper` passes to DPAPI.
#[cfg_attr(not(windows), allow(dead_code))]
pub const DPAPI_ENTROPY: &[u8] = b"DPAPI-Protected Secret";

const KEY_MODIFIER_LEN: usize = 16;
const BLOCK_LEN: usize = 16;
const AES_KEY_LEN: usize = 32;
const MAC_LEN: usize = 32;

/// True when `bytes` is a Data Protection payload.
pub fn is_payload(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

/// Why a key ring or a payload could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataProtectionError {
    /// The keys directory holds no key.
    NoKeys,
    /// A key file could not be read or parsed.
    KeyFile { file: String, reason: String },
    /// The payload names a key the ring does not have. The key files that could not be read are
    /// listed, since the key may be one of them.
    KeyNotFound {
        key: String,
        unreadable: Vec<String>,
    },
    /// The key is protected with a key certificate, which this version does not read.
    KeyCertificate(String),
    /// The key was revoked, so 0.1.x refused it too.
    Revoked(String),
    /// The key's master key could not be unwrapped (DPAPI from another machine, or DPAPI on Linux).
    KeyProtection { key: String, reason: String },
    /// The payload is damaged or was not made with this key and these purposes.
    Payload(String),
}

impl fmt::Display for DataProtectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKeys => f.write_str("the keys directory holds no key-*.xml"),
            Self::KeyFile { file, reason } => write!(f, "{file} could not be read ({reason})"),
            Self::KeyNotFound { key, unreadable } if unreadable.is_empty() => {
                write!(f, "key {key} is not in the key ring")
            }
            Self::KeyNotFound { key, unreadable } => write!(
                f,
                "key {key} is not among the key files that could be read; these could not: {}",
                unreadable.join("; ")
            ),
            Self::KeyCertificate(key) => write!(
                f,
                "key {key} is protected with a key certificate (Paths:KeyCertificate), which Koppel 0.2 does not read"
            ),
            Self::Revoked(key) => write!(f, "key {key} was revoked"),
            Self::KeyProtection { key, reason } => {
                write!(f, "key {key} could not be unwrapped ({reason})")
            }
            Self::Payload(reason) => write!(f, "the payload could not be decrypted ({reason})"),
        }
    }
}

impl std::error::Error for DataProtectionError {}

/// How a key's master key is kept.
#[derive(Clone, PartialEq, Eq)]
enum Secret {
    Plain(Vec<u8>),
    /// `DpapiXmlEncryptor`'s blob of the `masterKey` element.
    Dpapi(Vec<u8>),
    Certificate,
}

#[derive(Clone)]
struct Key {
    id: [u8; 16],
    name: String,
    created: Option<OffsetDateTime>,
    algorithms_ok: Result<(), String>,
    secret: Secret,
}

/// The keys in a Data Protection keys directory.
pub struct KeyRing {
    keys: Vec<Key>,
    revoked_ids: Vec<[u8; 16]>,
    revoked_before: Option<OffsetDateTime>,
    /// Key and revocation files that could not be read, with why. Data Protection skips those too.
    unreadable: Vec<String>,
}

impl fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyRing")
            .field(
                "keys",
                &self.keys.iter().map(|k| &k.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl KeyRing {
    /// Reads every `key-*.xml` and `revocation-*.xml` in `directory`. Subdirectories, such as the
    /// `aspnet` one the UI cookies' keys sit in, are not looked at.
    pub fn load(directory: &Path) -> Result<Self, DataProtectionError> {
        let mut ring = Self {
            keys: Vec::new(),
            revoked_ids: Vec::new(),
            revoked_before: None,
            unreadable: Vec::new(),
        };
        let entries = fs::read_dir(directory).map_err(|e| DataProtectionError::KeyFile {
            file: directory.display().to_string(),
            reason: e.to_string(),
        })?;
        let mut names: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.path())
            .collect();
        names.sort();
        for path in names {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if !name.ends_with(".xml") {
                continue;
            }
            let read = fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| xml_text(&bytes));
            let result = read.and_then(|text| {
                if name.starts_with("key-") {
                    parse_key(&text).map(|key| ring.keys.push(key))
                } else if name.starts_with("revocation-") {
                    ring.add_revocation(&text)
                } else {
                    Ok(())
                }
            });
            if let Err(reason) = result {
                tracing::warn!("{name} in the Koppel 0.1.x key ring could not be read: {reason}");
                ring.unreadable.push(format!("{name} ({reason})"));
            }
        }
        if ring.keys.is_empty() {
            return Err(match ring.unreadable.first() {
                Some(first) => DataProtectionError::KeyFile {
                    file: directory.display().to_string(),
                    reason: format!("no key file could be read, the first: {first}"),
                },
                None => DataProtectionError::NoKeys,
            });
        }
        Ok(ring)
    }

    fn add_revocation(&mut self, text: &str) -> Result<(), String> {
        let document = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
        let root = document.root_element();
        let date = child_text(root, "revocationDate").and_then(crate::json::parse_date);
        let id = root
            .children()
            .find(|n| n.has_tag_name("key"))
            .and_then(|n| n.attribute("id"));
        match id {
            Some("*") => {
                let date = date.ok_or("a revocation of every key has no revocationDate")?;
                self.revoked_before = Some(self.revoked_before.map_or(date, |d| d.max(date)));
            }
            Some(id) => self.revoked_ids.push(guid_bytes(id)?),
            None => return Err("a revocation names no key".into()),
        }
        Ok(())
    }

    /// True when the ring has a key protected with a key certificate.
    pub fn uses_key_certificate(&self) -> bool {
        self.keys.iter().any(|k| k.secret == Secret::Certificate)
    }

    /// The key id `payload` was made with, as .NET writes a `Guid`, or `None` for anything that is
    /// not a payload.
    pub fn payload_key(payload: &[u8]) -> Option<String> {
        is_payload(payload)
            .then(|| payload.get(4..20))
            .flatten()
            .map(|id| guid_text(id.try_into().expect("16 bytes")))
    }

    /// Decrypts `payload`, made by a protector with `purposes`.
    pub fn unprotect(
        &self,
        purposes: &[&str],
        payload: &[u8],
    ) -> Result<Vec<u8>, DataProtectionError> {
        if payload.len() < 20 || !is_payload(payload) {
            return Err(DataProtectionError::Payload(
                "it does not start with Data Protection's header".into(),
            ));
        }
        let id: [u8; 16] = payload[4..20].try_into().expect("16 bytes");
        let key = self.keys.iter().find(|k| k.id == id).ok_or_else(|| {
            DataProtectionError::KeyNotFound {
                key: guid_text(&id),
                unreadable: self.unreadable.clone(),
            }
        })?;
        if self.revoked_ids.contains(&id)
            || matches!((self.revoked_before, key.created), (Some(before), Some(created)) if created <= before)
        {
            return Err(DataProtectionError::Revoked(key.name.clone()));
        }
        key.algorithms_ok
            .clone()
            .map_err(|reason| DataProtectionError::KeyFile {
                file: format!("key-{}.xml", key.name),
                reason,
            })?;
        let master = master_key(key)?;
        let aad = additional_data(&id, purposes);
        decrypt_cbc_hmac(&master, &aad, &payload[20..]).map_err(DataProtectionError::Payload)
    }
}

/// The key's master key, unwrapping it with DPAPI where it is protected so.
fn master_key(key: &Key) -> Result<Vec<u8>, DataProtectionError> {
    match &key.secret {
        Secret::Plain(master) => Ok(master.clone()),
        Secret::Certificate => Err(DataProtectionError::KeyCertificate(key.name.clone())),
        Secret::Dpapi(blob) => {
            let unwrap_failed = |reason: String| DataProtectionError::KeyProtection {
                key: key.name.clone(),
                reason,
            };
            let element = unprotect_dpapi(blob).map_err(unwrap_failed)?;
            let text = xml_text(&element).map_err(unwrap_failed)?;
            let document =
                roxmltree::Document::parse(&text).map_err(|e| unwrap_failed(e.to_string()))?;
            let root = document.root_element();
            if !root.has_tag_name("masterKey") {
                return Err(unwrap_failed(
                    "DPAPI gave something other than a masterKey element".into(),
                ));
            }
            let value = child_text(root, "value")
                .ok_or_else(|| unwrap_failed("the masterKey element has no value".into()))?;
            STANDARD
                .decode(value.trim())
                .map_err(|e| unwrap_failed(e.to_string()))
        }
    }
}

#[cfg(windows)]
fn unprotect_dpapi(blob: &[u8]) -> Result<Vec<u8>, String> {
    crate::dpapi::unprotect(blob, DPAPI_ENTROPY).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn unprotect_dpapi(_blob: &[u8]) -> Result<Vec<u8>, String> {
    Err(
        "it is protected with Windows DPAPI, which only the Windows machine that wrote it can read"
            .into(),
    )
}

/// An XML document's text, after a byte order mark if there is one.
fn xml_text(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
}

fn child_text<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(name))
        .and_then(|n| n.text())
}

fn parse_key(text: &str) -> Result<Key, String> {
    let document = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name("key") {
        return Err("it is not a key".into());
    }
    let name = root.attribute("id").ok_or("the key has no id")?.to_string();
    let id = guid_bytes(&name)?;
    let created = child_text(root, "creationDate").and_then(crate::json::parse_date);
    let outer = root
        .children()
        .find(|n| n.has_tag_name("descriptor"))
        .ok_or("the key has no descriptor")?;
    let descriptor = outer
        .children()
        .find(|n| n.has_tag_name("descriptor"))
        .ok_or("the key has no inner descriptor")?;
    let algorithm = |element: &str| {
        descriptor
            .children()
            .find(|n| n.has_tag_name(element))
            .and_then(|n| n.attribute("algorithm"))
            .unwrap_or_default()
            .to_string()
    };
    let (encryption, validation) = (algorithm("encryption"), algorithm("validation"));
    let algorithms_ok = if encryption == "AES_256_CBC" && validation == "HMACSHA256" {
        Ok(())
    } else {
        Err(format!(
            "it uses {encryption} with {validation}; Koppel 0.1.x only ever used AES_256_CBC with HMACSHA256"
        ))
    };
    let secret = if let Some(master) = descriptor.children().find(|n| n.has_tag_name("masterKey")) {
        let value = child_text(master, "value").ok_or("the masterKey has no value")?;
        Secret::Plain(STANDARD.decode(value.trim()).map_err(|e| e.to_string())?)
    } else if let Some(encrypted) = descriptor
        .children()
        .find(|n| n.has_tag_name("encryptedSecret"))
    {
        let decryptor = encrypted.attribute("decryptorType").unwrap_or_default();
        if decryptor.contains("DpapiXmlDecryptor") {
            let value = encrypted
                .children()
                .find(|n| n.has_tag_name("encryptedKey"))
                .and_then(|n| child_text(n, "value"))
                .ok_or("the DPAPI key has no value")?;
            Secret::Dpapi(STANDARD.decode(value.trim()).map_err(|e| e.to_string())?)
        } else if decryptor.contains("EncryptedXmlDecryptor")
            || decryptor.contains("CertificateXmlEncryptor")
        {
            Secret::Certificate
        } else {
            return Err(format!(
                "its master key is protected by {decryptor}, which Koppel never used"
            ));
        }
    } else {
        return Err("the key has no master key".into());
    };
    Ok(Key {
        id,
        name,
        created,
        algorithms_ok,
        secret,
    })
}

/// A `Guid` in the byte order .NET writes it: the first three groups little-endian.
pub(crate) fn guid_bytes(text: &str) -> Result<[u8; 16], String> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || text.len() != 36 {
        return Err(format!("{text} is not a GUID"));
    }
    let mut plain = [0u8; 16];
    for (i, byte) in plain.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex digits");
    }
    let mut net = plain;
    net[0..4].reverse();
    net[4..6].reverse();
    net[6..8].reverse();
    Ok(net)
}

fn guid_text(net: &[u8; 16]) -> String {
    let mut plain = *net;
    plain[0..4].reverse();
    plain[4..6].reverse();
    plain[6..8].reverse();
    let hex: String = plain.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `magicHeader || keyId || purposeCount || (7 bit length || UTF-8)*`.
fn additional_data(key_id: &[u8; 16], purposes: &[&str]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(64);
    aad.extend_from_slice(&MAGIC);
    aad.extend_from_slice(key_id);
    aad.extend_from_slice(
        &i32::try_from(purposes.len())
            .expect("a few purposes")
            .to_be_bytes(),
    );
    for purpose in purposes {
        let mut length = purpose.len();
        while length >= 0x80 {
            aad.push((length as u8) | 0x80);
            length >>= 7;
        }
        aad.push(length as u8);
        aad.extend_from_slice(purpose.as_bytes());
    }
    aad
}

/// NIST SP800-108 in counter mode with HMAC-SHA512: `K(i) = PRF(kdk, [i]_32 || label || 0x00 ||
/// context || [L]_32)`, L in bits.
fn sp800_108(kdk: &[u8], label: &[u8], context: &[&[u8]], length: usize) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA512, kdk);
    let bits = u32::try_from(length * 8)
        .expect("a short output")
        .to_be_bytes();
    let mut out = Vec::with_capacity(length + 64);
    let mut i: u32 = 1;
    while out.len() < length {
        let mut context_hmac = hmac::Context::with_key(&key);
        context_hmac.update(&i.to_be_bytes());
        context_hmac.update(label);
        context_hmac.update(&[0]);
        for part in context {
            context_hmac.update(part);
        }
        context_hmac.update(&bits);
        out.extend_from_slice(context_hmac.sign().as_ref());
        i += 1;
    }
    out.truncate(length);
    out
}

/// The CBC encryptor's context header for AES-256-CBC with HMAC-SHA256.
fn context_header() -> Vec<u8> {
    let keys = sp800_108(&[], &[], &[], AES_KEY_LEN + MAC_LEN);
    let (encryption, validation) = keys.split_at(AES_KEY_LEN);
    let mut header = vec![0u8, 0u8];
    for value in [AES_KEY_LEN, BLOCK_LEN, MAC_LEN, MAC_LEN] {
        header.extend_from_slice(&(value as u32).to_be_bytes());
    }
    let mut block = [0u8; BLOCK_LEN];
    let empty = cbc::Encryptor::<Aes256>::new_from_slices(encryption, &[0u8; BLOCK_LEN])
        .expect("key and IV lengths")
        .encrypt_padded::<Pkcs7>(&mut block, 0)
        .expect("one block holds the padding");
    header.extend_from_slice(empty);
    header.extend_from_slice(
        hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, validation), &[]).as_ref(),
    );
    header
}

/// `keyModifier || IV || ciphertext || MAC(IV || ciphertext)` back to the plaintext.
fn decrypt_cbc_hmac(master: &[u8], aad: &[u8], body: &[u8]) -> Result<Vec<u8>, String> {
    if body.len() < KEY_MODIFIER_LEN + BLOCK_LEN + MAC_LEN {
        return Err("it is too short".into());
    }
    let (key_modifier, rest) = body.split_at(KEY_MODIFIER_LEN);
    let (iv_and_ciphertext, mac) = rest.split_at(rest.len() - MAC_LEN);
    let (iv, ciphertext) = iv_and_ciphertext.split_at(BLOCK_LEN);
    if ciphertext.is_empty() || ciphertext.len() % BLOCK_LEN != 0 {
        return Err("the ciphertext is not whole blocks".into());
    }
    let header = context_header();
    let keys = sp800_108(master, aad, &[&header, key_modifier], AES_KEY_LEN + MAC_LEN);
    let (encryption, validation) = keys.split_at(AES_KEY_LEN);
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, validation),
        iv_and_ciphertext,
        mac,
    )
    .map_err(|_| {
        "its MAC does not match: another key, other purposes, or a damaged file".to_string()
    })?;
    let mut buffer = ciphertext.to_vec();
    let plain = cbc::Decryptor::<Aes256>::new_from_slices(encryption, iv)
        .expect("key and IV lengths")
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .map_err(|_| "its padding is wrong".to_string())?;
    Ok(plain.to_vec())
}

/// Protects `plaintext` as .NET would with this master key, for tests that need a payload no
/// C# code wrote.
#[cfg(test)]
pub(crate) fn protect_for_test(
    master: &[u8],
    key_id: &[u8; 16],
    purposes: &[&str],
    plaintext: &[u8],
) -> Vec<u8> {
    let aad = additional_data(key_id, purposes);
    let key_modifier = [7u8; KEY_MODIFIER_LEN];
    let iv = [9u8; BLOCK_LEN];
    let header = context_header();
    let keys = sp800_108(
        master,
        &aad,
        &[&header, &key_modifier],
        AES_KEY_LEN + MAC_LEN,
    );
    let (encryption, validation) = keys.split_at(AES_KEY_LEN);
    let mut buffer = vec![0u8; plaintext.len() + BLOCK_LEN];
    buffer[..plaintext.len()].copy_from_slice(plaintext);
    let ciphertext = cbc::Encryptor::<Aes256>::new_from_slices(encryption, &iv)
        .unwrap()
        .encrypt_padded::<Pkcs7>(&mut buffer, plaintext.len())
        .unwrap()
        .to_vec();
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(key_id);
    out.extend_from_slice(&key_modifier);
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ciphertext);
    let mut signed = iv.to_vec();
    signed.extend_from_slice(&ciphertext);
    out.extend_from_slice(
        hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, validation), &signed).as_ref(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_use_dotnets_byte_order() {
        // The key id of a real 0.1.2 payload, and the key file it belongs to.
        let bytes = guid_bytes("6411acdb-8611-4073-a5e7-111a0405a81e").unwrap();
        assert_eq!(
            bytes,
            [
                0xdb, 0xac, 0x11, 0x64, 0x11, 0x86, 0x73, 0x40, 0xa5, 0xe7, 0x11, 0x1a, 0x04, 0x05,
                0xa8, 0x1e
            ]
        );
        assert_eq!(guid_text(&bytes), "6411acdb-8611-4073-a5e7-111a0405a81e");
        assert!(guid_bytes("6411acdb86114073a5e7111a0405a81e").is_err());
    }

    #[test]
    fn purposes_are_laid_out_as_dotnet_lays_them_out() {
        let aad = additional_data(&[0xAB; 16], &["Koppel", "Koppel.Secrets.v1"]);
        let mut expected = vec![0x09, 0xF0, 0xC9, 0xF0];
        expected.extend_from_slice(&[0xAB; 16]);
        expected.extend_from_slice(&[0, 0, 0, 2, 6]);
        expected.extend_from_slice(b"Koppel");
        expected.push(17);
        expected.extend_from_slice(b"Koppel.Secrets.v1");
        assert_eq!(aad, expected);

        let long = "x".repeat(200);
        let aad = additional_data(&[0; 16], &[&long]);
        assert_eq!(&aad[24..26], &[0xC8, 0x01]);
    }

    /// A Windows key ring cannot be committed: machine DPAPI only opens on the machine that made
    /// it. So this wraps the committed Linux 0.1.2 master key with DPAPI here, into the XML
    /// `DpapiXmlEncryptor` writes (the `masterKey` element as `XElement.Save` writes it: a byte
    /// order mark, the declaration, CRLF and two space indents), and reads the 0.1.2 files with it.
    #[cfg(windows)]
    #[test]
    fn a_key_ring_wrapped_with_machine_dpapi_as_dotnet_wraps_it_is_read() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/linux-0.1.2");
        let key_path = fs::read_dir(fixtures.join("keys"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        let plain_xml = xml_text(&fs::read(&key_path).unwrap()).unwrap();
        let start = plain_xml.find("<masterKey").unwrap();
        let end = plain_xml.find("</masterKey>").unwrap() + "</masterKey>".len();
        let value_start = plain_xml.find("<value>").unwrap() + 7;
        let value = &plain_xml[value_start..plain_xml.find("</value>").unwrap()];

        let element = format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<masterKey p4:requiresEncryption=\"true\" xmlns:p4=\"http://schemas.asp.net/2015/03/dataProtection\">\r\n  <!-- Warning: the key below is in an unencrypted form. -->\r\n  <value>{value}</value>\r\n</masterKey>"
        );
        let blob = crate::dpapi::protect(element.as_bytes(), DPAPI_ENTROPY).unwrap();
        let encrypted = format!(
            "<encryptedSecret decryptorType=\"Microsoft.AspNetCore.DataProtection.XmlEncryption.DpapiXmlDecryptor, Microsoft.AspNetCore.DataProtection, Version=10.0.0.0, Culture=neutral, PublicKeyToken=adb9793829ddae60\" xmlns=\"http://schemas.asp.net/2015/03/dataProtection\">\n          <encryptedKey xmlns=\"\">\n            <!-- This key is encrypted with Windows DPAPI. -->\n            <value>{}</value>\n          </encryptedKey>\n        </encryptedSecret>",
            STANDARD.encode(blob)
        );
        let windows_xml = format!("{}{}{}", &plain_xml[..start], encrypted, &plain_xml[end..]);
        assert!(
            !windows_xml.contains(value),
            "the master key is no longer in the clear"
        );

        let dir = std::env::temp_dir().join(format!("koppel-dpapi-ring-{}", crate::random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(windows_xml.as_bytes());
        fs::write(dir.join(key_path.file_name().unwrap()), bytes).unwrap();
        let ring = KeyRing::load(&dir).unwrap();

        for name in ["secrets.json", "connection.json", "ui-auth.json"] {
            let payload = fs::read(fixtures.join("config").join(name)).unwrap();
            let plain = ring.unprotect(&KOPPEL_PURPOSES, &payload).unwrap();
            assert_eq!(
                plain,
                fs::read(fixtures.join("plaintext").join(name)).unwrap(),
                "{name}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_payload_round_trips_and_a_wrong_purpose_is_refused() {
        let master = [5u8; 64];
        let id = guid_bytes("9130dec0-c26c-4c1a-9248-6f9905a5d9d7").unwrap();
        let ring = KeyRing {
            keys: vec![Key {
                id,
                name: "9130dec0-c26c-4c1a-9248-6f9905a5d9d7".into(),
                created: None,
                algorithms_ok: Ok(()),
                secret: Secret::Plain(master.to_vec()),
            }],
            revoked_ids: Vec::new(),
            revoked_before: None,
            unreadable: Vec::new(),
        };
        let payload = protect_for_test(&master, &id, &KOPPEL_PURPOSES, b"{\"a\":1}");
        assert_eq!(
            ring.unprotect(&KOPPEL_PURPOSES, &payload).unwrap(),
            b"{\"a\":1}"
        );
        assert!(matches!(
            ring.unprotect(&["Koppel"], &payload),
            Err(DataProtectionError::Payload(_))
        ));
        let mut damaged = payload.clone();
        let last = damaged.len() - 40;
        damaged[last] ^= 1;
        assert!(matches!(
            ring.unprotect(&KOPPEL_PURPOSES, &damaged),
            Err(DataProtectionError::Payload(_))
        ));
    }
}

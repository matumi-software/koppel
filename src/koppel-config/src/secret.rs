//! The format Koppel 0.2 keeps its secret files in.
//!
//! Every file starts with a six byte header: `KPSF` (Koppel protected secret file), a format
//! version (1), and the scheme:
//!
//! - **1, Windows:** the rest is a DPAPI blob made with machine scope, with
//!   [`DPAPI_ENTROPY`] mixed in. Any process on the machine can read it, as with 0.1.x's key ring.
//! - **2, a key file:** a 16 byte key id, a 12 byte nonce, then AES-256-GCM ciphertext and tag.
//!   The header and the key id are the additional data. The key is 32 random bytes in
//!   `koppel-secrets.key` in the keys directory, mode `0600` on Linux. That file is `KPSK`, its
//!   version (1), the key id and the key.
//!
//! Windows uses scheme 1 and everything else scheme 2. A reader takes whichever scheme a file has
//! where it can, so a new scheme or version can be added beside these. The key id says which key a
//! file needs, so a file from another machine's key is named as such instead of failing as damaged.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use crate::file::{CodecError, FileCodec, create_private, sync_directory};

/// What every secret file starts with.
pub const MAGIC: [u8; 4] = *b"KPSF";
/// What an ASP.NET Core Data Protection payload, as Koppel 0.1.x wrote its secret files, starts with.
const DATA_PROTECTION_MAGIC: [u8; 4] = [0x09, 0xF0, 0xC9, 0xF0];
/// The format version this code writes.
pub const VERSION: u8 = 1;
/// Machine-scope DPAPI.
pub const SCHEME_DPAPI: u8 = 1;
/// AES-256-GCM with the key in the keys directory.
pub const SCHEME_KEY_FILE: u8 = 2;
/// What DPAPI mixes in, so a blob made for something else does not open as a secret file.
#[cfg_attr(not(windows), allow(dead_code))]
pub const DPAPI_ENTROPY: &[u8] = b"Koppel secret file";
/// The key file's name in the keys directory.
pub const KEY_FILE: &str = "koppel-secrets.key";
/// The environment variable that names a key file outside the keys directory.
pub const KEY_FILE_VARIABLE: &str = "KOPPEL_KEY_FILE";
/// The name of the systemd credential and of the Docker secret that hold a key.
pub const KEY_SOURCE_NAME: &str = "koppel-key";

const KEY_MAGIC: [u8; 4] = *b"KPSK";
const KEY_ID_LEN: usize = 16;
const KEY_LEN: usize = 32;
const HEADER_LEN: usize = 6;
const KEY_FILE_LEN: usize = 5 + KEY_ID_LEN + KEY_LEN;
const KEY_FILE_WAITS: u32 = 30;
const KEY_FILE_WAIT: Duration = Duration::from_millis(100);
const MIN_SOURCE_LEN: usize = 32;
const MAX_SOURCE_LEN: usize = 4096;

/// Why a secret could not be sealed, opened, or its key loaded.
#[derive(Debug)]
pub enum SecretError {
    /// The key file could not be read, made or used.
    KeyFile { path: PathBuf, reason: String },
    /// The bytes are not a secret file this version can open.
    Format(String),
    /// The secret did not open: another key, or damaged.
    Open(String),
    /// Sealing failed.
    Seal(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyFile { path, reason } => write!(
                f,
                "the key file {} cannot be used ({reason})",
                path.display()
            ),
            Self::Format(reason) | Self::Open(reason) | Self::Seal(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for SecretError {}

/// What a file's first bytes say it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretFormat {
    /// This format, any scheme.
    Current,
    /// ASP.NET Core Data Protection, as Koppel 0.1.x wrote it.
    DataProtection,
    /// Neither.
    Unknown,
}

/// What `bytes` is, by its header.
pub fn format_of(bytes: &[u8]) -> SecretFormat {
    if bytes.starts_with(&MAGIC) {
        SecretFormat::Current
    } else if bytes.starts_with(&DATA_PROTECTION_MAGIC) {
        SecretFormat::DataProtection
    } else {
        SecretFormat::Unknown
    }
}

/// One AES-256-GCM key and the id its files carry.
struct KeyEntry {
    id: [u8; KEY_ID_LEN],
    key: LessSafeKey,
}

/// The keys of the key file scheme. A file is opened with the key its id names. New files are
/// sealed with `entries[sealing]`.
struct KeyRing {
    sealing: usize,
    entries: Vec<KeyEntry>,
}

enum Scheme {
    #[cfg(windows)]
    Dpapi,
    KeyFile(KeyRing),
}

/// Where the key that seals new files comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// Windows: machine DPAPI, no key to keep.
    Dpapi,
    /// `koppel-secrets.key` in the keys directory, beside the files it protects.
    KeysDirectory,
    /// The file `KOPPEL_KEY_FILE` names.
    Environment(PathBuf),
    /// A systemd credential: `koppel-key` in `$CREDENTIALS_DIRECTORY`.
    Credential(PathBuf),
    /// A Docker secret: `/run/secrets/koppel-key`.
    DockerSecret(PathBuf),
}

impl KeySource {
    /// True when the key is kept apart from the config and keys volume.
    pub fn is_outside_the_volume(&self) -> bool {
        !matches!(self, Self::KeysDirectory)
    }
}

impl fmt::Display for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dpapi => f.write_str("Windows DPAPI"),
            Self::KeysDirectory => write!(f, "{KEY_FILE} in the keys directory"),
            Self::Environment(path) => write!(f, "{KEY_FILE_VARIABLE} ({})", path.display()),
            Self::Credential(path) => write!(f, "a systemd credential ({})", path.display()),
            Self::DockerSecret(path) => write!(f, "a Docker secret ({})", path.display()),
        }
    }
}

/// The places a key can come from other than the keys directory, in the order they are tried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeySources {
    /// `KOPPEL_KEY_FILE`. When it is set the file must exist.
    pub environment: Option<PathBuf>,
    /// `$CREDENTIALS_DIRECTORY/koppel-key`, used when it exists.
    pub credential: Option<PathBuf>,
    /// `/run/secrets/koppel-key`, used when it exists.
    pub docker_secret: Option<PathBuf>,
}

impl KeySources {
    /// None of them: the key lives in the keys directory.
    pub fn none() -> Self {
        Self::default()
    }

    /// From `KOPPEL_KEY_FILE`, `CREDENTIALS_DIRECTORY` and the standard Docker secrets folder.
    pub fn from_environment() -> Self {
        let set = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            environment: set(KEY_FILE_VARIABLE).map(PathBuf::from),
            credential: set("CREDENTIALS_DIRECTORY")
                .map(|directory| PathBuf::from(directory).join(KEY_SOURCE_NAME)),
            docker_secret: Some(PathBuf::from("/run/secrets").join(KEY_SOURCE_NAME)),
        }
    }

    /// The first source that applies.
    fn chosen(&self) -> Result<Option<KeySource>, SecretError> {
        if let Some(path) = &self.environment {
            if !path.is_file() {
                return Err(SecretError::KeyFile {
                    path: path.clone(),
                    reason: format!("{KEY_FILE_VARIABLE} names it and it is not a file"),
                });
            }
            return Ok(Some(KeySource::Environment(path.clone())));
        }
        if let Some(path) = self.credential.as_ref().filter(|p| p.is_file()) {
            return Ok(Some(KeySource::Credential(path.clone())));
        }
        if let Some(path) = self.docker_secret.as_ref().filter(|p| p.is_file()) {
            return Ok(Some(KeySource::DockerSecret(path.clone())));
        }
        Ok(None)
    }
}

/// Seals and opens Koppel's secret files.
pub struct SecretBox {
    scheme: Scheme,
    source: KeySource,
    random: SystemRandom,
}

impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scheme {
            #[cfg(windows)]
            Scheme::Dpapi => f.write_str("SecretBox(dpapi)"),
            Scheme::KeyFile(ring) => write!(
                f,
                "SecretBox(key file, {} key(s), sealing from {})",
                ring.entries.len(),
                self.source
            ),
        }
    }
}

impl SecretBox {
    /// The box for this platform: machine DPAPI on Windows, and elsewhere a key from
    /// [`KeySources::from_environment`] or else the key file in `keys_directory`, made on first
    /// use. The directory must exist; [`crate::KoppelPaths::prepare`] makes and restricts it.
    pub fn open(keys_directory: &Path) -> Result<Self, SecretError> {
        #[cfg(windows)]
        {
            let _ = keys_directory;
            if std::env::var_os(KEY_FILE_VARIABLE).is_some() {
                tracing::warn!(
                    "{KEY_FILE_VARIABLE} is set, but Windows seals the secret files with DPAPI and does not read it"
                );
            }
            Ok(Self {
                scheme: Scheme::Dpapi,
                source: KeySource::Dpapi,
                random: SystemRandom::new(),
            })
        }
        #[cfg(not(windows))]
        {
            Self::with_sources(keys_directory, &KeySources::from_environment())
        }
    }

    /// The key file scheme on any platform, with the key in the keys directory. Makes the key file
    /// when there is none.
    pub fn with_key_file(keys_directory: &Path) -> Result<Self, SecretError> {
        Self::with_sources(keys_directory, &KeySources::none())
    }

    /// The key file scheme on any platform, with the key from `sources` when one applies.
    ///
    /// The ring holds every key it can read, and seals with one:
    ///
    /// - A key from `sources` seals. The key file in the keys directory, when there is one, is
    ///   then only read, so files sealed before the move still open; [`Self::reseal`] moves them
    ///   over, after which that file can be deleted. None is made, since a key made beside the
    ///   files would defeat the point.
    /// - Without a source, `koppel-secrets.key` seals, and is made when missing.
    /// - `koppel-secrets.<anything>.key` files are older keys, only read. To rotate the key, rename
    ///   `koppel-secrets.key` to such a name and start Koppel: it makes a new one and reseals.
    pub fn with_sources(keys_directory: &Path, sources: &KeySources) -> Result<Self, SecretError> {
        let random = SystemRandom::new();
        let mut entries: Vec<KeyEntry> = Vec::new();
        let mut keep = |entry: KeyEntry| {
            if !entries.iter().any(|known| known.id == entry.id) {
                entries.push(entry);
            }
        };
        let source = match sources.chosen()? {
            Some(
                source @ (KeySource::Environment(_)
                | KeySource::Credential(_)
                | KeySource::DockerSecret(_)),
            ) => {
                let (KeySource::Environment(path)
                | KeySource::Credential(path)
                | KeySource::DockerSecret(path)) = &source
                else {
                    unreachable!("matched above");
                };
                keep(derive_key(path)?);
                let primary = keys_directory.join(KEY_FILE);
                if primary.exists() {
                    keep(load_key_file(&primary, &random, false)?);
                }
                source
            }
            _ => {
                keep(load_key_file(
                    &keys_directory.join(KEY_FILE),
                    &random,
                    true,
                )?);
                KeySource::KeysDirectory
            }
        };
        for retired in retired_key_files(keys_directory) {
            keep(load_retired_key(&retired, &random)?);
        }
        Ok(Self {
            scheme: Scheme::KeyFile(KeyRing {
                sealing: 0,
                entries,
            }),
            source,
            random,
        })
    }

    /// Where the key that seals new files comes from.
    pub fn source(&self) -> &KeySource {
        &self.source
    }

    /// How many keys can open files, the one that seals included.
    pub fn keys_read(&self) -> usize {
        match &self.scheme {
            #[cfg(windows)]
            Scheme::Dpapi => 1,
            Scheme::KeyFile(ring) => ring.entries.len(),
        }
    }

    /// The scheme this box seals with.
    pub fn scheme(&self) -> u8 {
        match self.scheme {
            #[cfg(windows)]
            Scheme::Dpapi => SCHEME_DPAPI,
            Scheme::KeyFile(_) => SCHEME_KEY_FILE,
        }
    }

    /// Seals `plaintext` into a secret file's bytes.
    pub fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, SecretError> {
        let mut out = MAGIC.to_vec();
        out.push(VERSION);
        out.push(self.scheme());
        match &self.scheme {
            #[cfg(windows)]
            Scheme::Dpapi => {
                let blob = crate::dpapi::protect(plaintext, DPAPI_ENTROPY)
                    .map_err(|e| SecretError::Seal(format!("DPAPI failed ({e})")))?;
                out.extend_from_slice(&blob);
            }
            Scheme::KeyFile(ring) => {
                let KeyEntry { id, key } = &ring.entries[ring.sealing];
                out.extend_from_slice(id);
                let mut nonce = [0u8; NONCE_LEN];
                self.random
                    .fill(&mut nonce)
                    .map_err(|_| SecretError::Seal("no random bytes".into()))?;
                let aad = out.clone();
                out.extend_from_slice(&nonce);
                // The copy of the plaintext is overwritten by the ciphertext in place.
                let mut sealed = Zeroizing::new(plaintext.to_vec());
                key.seal_in_place_append_tag(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(aad),
                    &mut *sealed,
                )
                .map_err(|_| SecretError::Seal("AES-GCM failed".into()))?;
                out.extend_from_slice(&sealed);
            }
        }
        Ok(out)
    }

    /// Opens a secret file's bytes.
    pub fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>, SecretError> {
        match format_of(bytes) {
            SecretFormat::Current => {}
            SecretFormat::DataProtection => {
                return Err(SecretError::Format(
                    "it is still in Koppel 0.1.x's format: converting it failed, and the log says why".into(),
                ));
            }
            SecretFormat::Unknown => {
                return Err(SecretError::Format("it is not a Koppel secret file".into()));
            }
        }
        if bytes.len() < HEADER_LEN {
            return Err(SecretError::Format("it is too short".into()));
        }
        if bytes[4] != VERSION {
            return Err(SecretError::Format(format!(
                "it is format version {}, written by a newer Koppel; this one reads version {VERSION}",
                bytes[4]
            )));
        }
        match (bytes[5], &self.scheme) {
            #[cfg(windows)]
            (SCHEME_DPAPI, Scheme::Dpapi) => crate::dpapi::unprotect(&bytes[HEADER_LEN..], DPAPI_ENTROPY)
                .map_err(|e| SecretError::Open(format!("DPAPI could not decrypt it, so it was written on another machine or is damaged ({e})"))),
            (SCHEME_KEY_FILE, Scheme::KeyFile(ring)) => {
                let body = &bytes[HEADER_LEN..];
                if body.len() < KEY_ID_LEN + NONCE_LEN + AES_256_GCM.tag_len() {
                    return Err(SecretError::Format("it is too short".into()));
                }
                let Some(entry) = ring.entries.iter().find(|e| body[..KEY_ID_LEN] == e.id[..]) else {
                    return Err(SecretError::Open(format!(
                        "it was encrypted with another key than the ones this Koppel has: the {KEY_FILE} in the keys directory, {KEY_FILE_VARIABLE}, a credential or a Docker secret"
                    )));
                };
                let aad = bytes[..HEADER_LEN + KEY_ID_LEN].to_vec();
                let nonce: [u8; NONCE_LEN] = body[KEY_ID_LEN..KEY_ID_LEN + NONCE_LEN].try_into().expect("checked length");
                // Decrypted in place, so this buffer holds the plaintext until it is wiped.
                let mut sealed = Zeroizing::new(body[KEY_ID_LEN + NONCE_LEN..].to_vec());
                let plain = entry
                    .key
                    .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut sealed)
                    .map_err(|_| SecretError::Open("it does not decrypt with the key file: it is damaged".into()))?;
                Ok(plain.to_vec())
            }
            (SCHEME_DPAPI, _) => Err(SecretError::Open("it was encrypted with Windows DPAPI, which only that Windows machine can read".into())),
            #[cfg(windows)]
            (SCHEME_KEY_FILE, _) => Err(SecretError::Open(format!(
                "it was encrypted with a {KEY_FILE}, and this Koppel uses DPAPI on Windows"
            ))),
            (scheme, _) => Err(SecretError::Format(format!("it uses scheme {scheme}, which this Koppel does not know"))),
        }
    }

    /// Writes again, with the key that seals now, each of `files` that is sealed with another key
    /// of the ring: after a rotation, or after moving the key out of the keys directory. A file
    /// that is missing, or sealed another way, is left alone, and so is one that does not open: it
    /// is named in the log, and its owner stops start-up with the reason when it is read. Returns
    /// the files that were written.
    // Off Windows the key file scheme is the only one, which makes the `let else` always match.
    #[cfg_attr(not(windows), allow(irrefutable_let_patterns))]
    pub fn reseal(&self, files: &[PathBuf]) -> Vec<PathBuf> {
        let Scheme::KeyFile(ring) = &self.scheme else {
            return Vec::new();
        };
        let sealing = ring.entries[ring.sealing].id;
        let mut done = Vec::new();
        for file in files {
            let Ok(bytes) = fs::read(file) else {
                continue;
            };
            let on_other_key = bytes.starts_with(&MAGIC)
                && bytes.len() > HEADER_LEN + KEY_ID_LEN
                && bytes[5] == SCHEME_KEY_FILE
                && bytes[HEADER_LEN..HEADER_LEN + KEY_ID_LEN] != sealing[..];
            if !on_other_key {
                continue;
            }
            let plain = match self.unprotect(&bytes) {
                Ok(plain) => Zeroizing::new(plain),
                Err(e) => {
                    tracing::warn!(
                        "{} was not sealed again with the current key: {e}",
                        file.display()
                    );
                    continue;
                }
            };
            let written = self
                .protect(&plain)
                .map_err(|e| e.to_string())
                .and_then(|sealed| {
                    crate::file::write_atomic(file, &sealed)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
            match written {
                Ok(()) => done.push(file.clone()),
                Err(e) => tracing::warn!("{} could not be sealed again: {e}", file.display()),
            }
        }
        if !done.is_empty() {
            tracing::info!(
                "Sealed {} file(s) again with the key from {}",
                done.len(),
                self.source
            );
        }
        done
    }

    /// A codec that encrypts a [`crate::ConfigFile`] with this box.
    pub fn codec(self: &Arc<Self>) -> Arc<dyn FileCodec> {
        Arc::new(SecretCodec(self.clone()))
    }
}

/// The key a file from outside the volume stands for. The file is any secret of 32 bytes or more
/// (`openssl rand -base64 32` makes a good one); a trailing line break is ignored. The AES key and
/// the key id come from it by HKDF-SHA256, so the same file is always the same key and the id
/// says which files it opened.
fn derive_key(path: &Path) -> Result<KeyEntry, SecretError> {
    let failed = |reason: String| SecretError::KeyFile {
        path: path.to_path_buf(),
        reason,
    };
    let raw = Zeroizing::new(fs::read(path).map_err(|e| failed(e.to_string()))?);
    let trimmed = raw.trim_ascii_end();
    if trimmed.len() < MIN_SOURCE_LEN {
        return Err(failed(format!(
            "it holds {} bytes and needs at least {MIN_SOURCE_LEN}: make it with `openssl rand -base64 32`",
            trimmed.len()
        )));
    }
    if trimmed.len() > MAX_SOURCE_LEN {
        return Err(failed(format!("it holds more than {MAX_SOURCE_LEN} bytes")));
    }
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"Koppel key source v1");
    let mut material = Zeroizing::new([0u8; KEY_LEN + KEY_ID_LEN]);
    salt.extract(trimmed)
        .expand(&[b"Koppel secret file key"], OutputLength(material.len()))
        .and_then(|okm| okm.fill(&mut *material))
        .map_err(|_| failed("a key could not be made from it".into()))?;
    let id: [u8; KEY_ID_LEN] = material[KEY_LEN..].try_into().expect("fixed length");
    let key = UnboundKey::new(&AES_256_GCM, &material[..KEY_LEN])
        .map_err(|_| failed("its key is not usable".into()))?;
    Ok(KeyEntry {
        id,
        key: LessSafeKey::new(key),
    })
}

/// How many bytes HKDF is asked for.
struct OutputLength(usize);

impl hkdf::KeyType for OutputLength {
    fn len(&self) -> usize {
        self.0
    }
}

/// The older keys beside the current one: `koppel-secrets.<anything>.key`, by name.
fn retired_key_files(keys_directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(keys_directory) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("koppel-secrets.")
                && name.ends_with(".key")
                && name.len() > "koppel-secrets..key".len()
        })
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

/// A retired key: a Koppel key file, or a plain secret as a key source takes.
fn load_retired_key(path: &Path, random: &SystemRandom) -> Result<KeyEntry, SecretError> {
    let is_key_file = fs::read(path)
        .map(|bytes| bytes.starts_with(&KEY_MAGIC) && bytes.len() == KEY_FILE_LEN)
        .unwrap_or(false);
    if is_key_file {
        load_key_file(path, random, false)
    } else {
        derive_key(path)
    }
}

/// Reads a `KPSK` key file. `make` makes it when it is missing.
fn load_key_file(path: &Path, random: &SystemRandom, make: bool) -> Result<KeyEntry, SecretError> {
    let failed = |reason: String| SecretError::KeyFile {
        path: path.to_path_buf(),
        reason,
    };
    let bytes = match read_key_file(path) {
        Ok(bytes) => bytes,
        Err(e) if make && e.kind() == io::ErrorKind::NotFound => {
            create_key_file(path, random).map_err(|e| failed(e.to_string()))?
        }
        Err(e) => return Err(failed(e.to_string())),
    };
    let bytes = Zeroizing::new(bytes);
    if bytes.len() != KEY_FILE_LEN || bytes[..4] != KEY_MAGIC {
        return Err(failed("it is not a Koppel key file".into()));
    }
    if bytes[4] != VERSION {
        return Err(failed(format!(
            "it is version {}, and this Koppel reads version {VERSION}",
            bytes[4]
        )));
    }
    let id: [u8; KEY_ID_LEN] = bytes[5..5 + KEY_ID_LEN].try_into().expect("checked length");
    let key = UnboundKey::new(&AES_256_GCM, &bytes[5 + KEY_ID_LEN..])
        .map_err(|_| failed("its key is not usable".into()))?;
    Ok(KeyEntry {
        id,
        key: LessSafeKey::new(key),
    })
}

/// Writes the key file once. The first choice is a temp file with mode `0600`, flushed, then linked
/// into place, so it never exists half written and an existing one is never replaced. A file system
/// without hard links (some network, FUSE and Docker bind mounts) refuses the link; the key file
/// is then made in place with `create_new`, which still never replaces one, and a start that finds
/// it while it is being written waits for it ([`read_key_file`]). The directory is flushed after.
fn create_key_file(path: &Path, random: &SystemRandom) -> io::Result<Vec<u8>> {
    create_key_file_with(path, random, &|from, to| fs::hard_link(from, to))
}

fn create_key_file_with(
    path: &Path,
    random: &SystemRandom,
    link: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    let mut bytes = KEY_MAGIC.to_vec();
    bytes.push(VERSION);
    let mut material = [0u8; KEY_ID_LEN + KEY_LEN];
    random
        .fill(&mut material)
        .map_err(|_| io::Error::other("no random bytes"))?;
    bytes.extend_from_slice(&material);

    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", crate::random_hex(16)));
    let temp = PathBuf::from(temp);
    let linked = (|| {
        let mut file = create_private(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        link(&temp, path)
    })();
    let _ = fs::remove_file(&temp);
    let result = match linked {
        Ok(()) => Ok(()),
        // Another start made it first: use that one.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return read_key_file(path),
        Err(e) => {
            tracing::debug!(
                "{} could not be linked into place ({e}); writing it in place",
                path.display()
            );
            write_in_place(path, &bytes)
        }
    };
    match result {
        Ok(()) => {
            sync_directory(path)?;
            Ok(bytes)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => read_key_file(path),
        Err(e) => Err(e),
    }
}

/// The key file made where it will stay, for a file system that cannot link. A crash in the middle
/// leaves a short file, which a start refuses with the instruction to delete it: nothing was
/// sealed with a key that never finished being written.
fn write_in_place(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_private(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Reads the key file, waiting a moment for one that another start is still writing.
fn read_key_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = fs::read(path)?;
    for _ in 0..KEY_FILE_WAITS {
        if bytes.len() >= KEY_FILE_LEN {
            break;
        }
        std::thread::sleep(KEY_FILE_WAIT);
        bytes = fs::read(path)?;
    }
    Ok(bytes)
}

struct SecretCodec(Arc<SecretBox>);

impl FileCodec for SecretCodec {
    fn encode(&self, json: &[u8]) -> Result<Vec<u8>, CodecError> {
        self.0.protect(json).map_err(|e| CodecError(e.to_string()))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
        self.0
            .unprotect(bytes)
            .map_err(|e| CodecError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("koppel-secret-{name}-{}", crate::random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_key_file_scheme_round_trips_and_keeps_its_key() {
        let dir = temp_dir("keyfile");
        let first = SecretBox::with_key_file(&dir).unwrap();
        let sealed = first.protect(b"{\"a\":1}").unwrap();
        assert_eq!(&sealed[..6], b"KPSF\x01\x02");
        assert_eq!(format_of(&sealed), SecretFormat::Current);

        let again = SecretBox::with_key_file(&dir).unwrap();
        assert_eq!(again.unprotect(&sealed).unwrap(), b"{\"a\":1}");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_for_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        SecretBox::with_key_file(&dir).unwrap();
        let mode = fs::metadata(dir.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn another_key_or_a_damaged_file_is_refused_with_a_reason() {
        let one = temp_dir("one");
        let two = temp_dir("two");
        let sealed = SecretBox::with_key_file(&one)
            .unwrap()
            .protect(b"secret")
            .unwrap();
        let other = SecretBox::with_key_file(&two).unwrap();
        assert!(
            other
                .unprotect(&sealed)
                .unwrap_err()
                .to_string()
                .contains("another key")
        );

        let same = SecretBox::with_key_file(&one).unwrap();
        let mut damaged = sealed.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(
            same.unprotect(&damaged)
                .unwrap_err()
                .to_string()
                .contains("damaged")
        );
        let mut newer = sealed.clone();
        newer[4] = 2;
        assert!(
            same.unprotect(&newer)
                .unwrap_err()
                .to_string()
                .contains("newer Koppel")
        );
        assert!(same.unprotect(b"{}").is_err());
        let mut old = DATA_PROTECTION_MAGIC.to_vec();
        old.extend_from_slice(&[0; 80]);
        assert!(
            same.unprotect(&old)
                .unwrap_err()
                .to_string()
                .contains("0.1.x")
        );
        let _ = (fs::remove_dir_all(one), fs::remove_dir_all(two));
    }

    #[test]
    fn a_damaged_key_file_is_refused() {
        let dir = temp_dir("badkey");
        fs::write(dir.join(KEY_FILE), b"KPSK\x01short").unwrap();
        assert!(SecretBox::with_key_file(&dir).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_system_without_hard_links_still_gets_a_key_file() {
        let dir = temp_dir("nolinks");
        let path = dir.join(KEY_FILE);
        let refuse = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));

        let made = create_key_file_with(&path, &SystemRandom::new(), &refuse).unwrap();

        assert_eq!(fs::read(&path).unwrap(), made);
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "no temp file is left: {names:?}");
        // And it is a usable key file.
        let first = SecretBox::with_key_file(&dir).unwrap();
        let sealed = first.protect(b"x").unwrap();
        assert_eq!(
            SecretBox::with_key_file(&dir)
                .unwrap()
                .unprotect(&sealed)
                .unwrap(),
            b"x"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn without_hard_links_an_existing_key_file_is_used_and_not_replaced() {
        let dir = temp_dir("nolinks-exists");
        let existing = SecretBox::with_key_file(&dir).unwrap();
        let before = fs::read(dir.join(KEY_FILE)).unwrap();
        let refuse = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::PermissionDenied));

        let made =
            create_key_file_with(&dir.join(KEY_FILE), &SystemRandom::new(), &refuse).unwrap();

        assert_eq!(made, before);
        assert_eq!(fs::read(dir.join(KEY_FILE)).unwrap(), before);
        let sealed = existing.protect(b"x").unwrap();
        assert!(
            SecretBox::with_key_file(&dir)
                .unwrap()
                .unprotect(&sealed)
                .is_ok()
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_made_in_place_is_for_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("nolinks-mode");
        let refuse = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        create_key_file_with(&dir.join(KEY_FILE), &SystemRandom::new(), &refuse).unwrap();
        let mode = fs::metadata(dir.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_key_file_that_is_still_being_written_is_waited_for() {
        let dir = temp_dir("slow");
        let path = dir.join(KEY_FILE);
        let source = temp_dir("slow-source");
        let whole = {
            SecretBox::with_key_file(&source).unwrap();
            fs::read(source.join(KEY_FILE)).unwrap()
        };
        fs::write(&path, &whole[..10]).unwrap();
        let finish = {
            let (path, whole) = (path.clone(), whole.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                fs::write(path, whole).unwrap();
            })
        };

        let opened = SecretBox::with_key_file(&dir);

        finish.join().unwrap();
        assert!(opened.is_ok(), "{opened:?}");
        let _ = (fs::remove_dir_all(dir), fs::remove_dir_all(source));
    }

    #[cfg(windows)]
    #[test]
    fn windows_seals_with_machine_dpapi() {
        let secret_box = SecretBox::open(Path::new(".")).unwrap();
        let sealed = secret_box.protect(b"{\"a\":1}").unwrap();
        assert_eq!(&sealed[..6], b"KPSF\x01\x01");
        assert_eq!(secret_box.unprotect(&sealed).unwrap(), b"{\"a\":1}");
        let dir = temp_dir("cross");
        let key_file = SecretBox::with_key_file(&dir).unwrap();
        assert!(
            key_file
                .unprotect(&sealed)
                .unwrap_err()
                .to_string()
                .contains("DPAPI")
        );
        let _ = fs::remove_dir_all(dir);
    }
}

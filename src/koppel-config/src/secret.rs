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
use ring::rand::{SecureRandom, SystemRandom};

use crate::dataprotection;
use crate::file::{CodecError, FileCodec, create_private, sync_directory};

/// What every secret file starts with.
pub const MAGIC: [u8; 4] = *b"KPSF";
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

const KEY_MAGIC: [u8; 4] = *b"KPSK";
const KEY_ID_LEN: usize = 16;
const KEY_LEN: usize = 32;
const HEADER_LEN: usize = 6;
const KEY_FILE_LEN: usize = 5 + KEY_ID_LEN + KEY_LEN;
const KEY_FILE_WAITS: u32 = 30;
const KEY_FILE_WAIT: Duration = Duration::from_millis(100);

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
    } else if dataprotection::is_payload(bytes) {
        SecretFormat::DataProtection
    } else {
        SecretFormat::Unknown
    }
}

enum Scheme {
    #[cfg(windows)]
    Dpapi,
    KeyFile {
        id: [u8; KEY_ID_LEN],
        key: Box<LessSafeKey>,
    },
}

/// Seals and opens Koppel's secret files.
pub struct SecretBox {
    scheme: Scheme,
    random: SystemRandom,
}

impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.scheme {
            #[cfg(windows)]
            Scheme::Dpapi => "SecretBox(dpapi)",
            Scheme::KeyFile { .. } => "SecretBox(key file)",
        })
    }
}

impl SecretBox {
    /// The box for this platform: machine DPAPI on Windows, and elsewhere the key file in
    /// `keys_directory`, made on first use. The directory must exist; [`crate::KoppelPaths::prepare`]
    /// makes and restricts it.
    pub fn open(keys_directory: &Path) -> Result<Self, SecretError> {
        #[cfg(windows)]
        {
            let _ = keys_directory;
            Ok(Self {
                scheme: Scheme::Dpapi,
                random: SystemRandom::new(),
            })
        }
        #[cfg(not(windows))]
        {
            Self::with_key_file(keys_directory)
        }
    }

    /// The key file scheme on any platform. Makes the key file when there is none.
    pub fn with_key_file(keys_directory: &Path) -> Result<Self, SecretError> {
        let random = SystemRandom::new();
        let path = keys_directory.join(KEY_FILE);
        let failed = |reason: String| SecretError::KeyFile {
            path: path.clone(),
            reason,
        };
        let bytes = match read_key_file(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                create_key_file(&path, &random).map_err(|e| failed(e.to_string()))?
            }
            Err(e) => return Err(failed(e.to_string())),
        };
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
        Ok(Self {
            scheme: Scheme::KeyFile {
                id,
                key: Box::new(LessSafeKey::new(key)),
            },
            random,
        })
    }

    /// The scheme this box seals with.
    pub fn scheme(&self) -> u8 {
        match self.scheme {
            #[cfg(windows)]
            Scheme::Dpapi => SCHEME_DPAPI,
            Scheme::KeyFile { .. } => SCHEME_KEY_FILE,
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
            Scheme::KeyFile { id, key } => {
                out.extend_from_slice(id);
                let mut nonce = [0u8; NONCE_LEN];
                self.random
                    .fill(&mut nonce)
                    .map_err(|_| SecretError::Seal("no random bytes".into()))?;
                let aad = out.clone();
                out.extend_from_slice(&nonce);
                let mut sealed = plaintext.to_vec();
                key.seal_in_place_append_tag(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(aad),
                    &mut sealed,
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
            (SCHEME_KEY_FILE, Scheme::KeyFile { id, key }) => {
                let body = &bytes[HEADER_LEN..];
                if body.len() < KEY_ID_LEN + NONCE_LEN + AES_256_GCM.tag_len() {
                    return Err(SecretError::Format("it is too short".into()));
                }
                if body[..KEY_ID_LEN] != id[..] {
                    return Err(SecretError::Open(format!(
                        "it was encrypted with another key file than the {KEY_FILE} in the keys directory"
                    )));
                }
                let aad = bytes[..HEADER_LEN + KEY_ID_LEN].to_vec();
                let nonce: [u8; NONCE_LEN] = body[KEY_ID_LEN..KEY_ID_LEN + NONCE_LEN].try_into().expect("checked length");
                let mut sealed = body[KEY_ID_LEN + NONCE_LEN..].to_vec();
                let plain = key
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

    /// A codec that encrypts a [`crate::ConfigFile`] with this box.
    pub fn codec(self: &Arc<Self>) -> Arc<dyn FileCodec> {
        Arc::new(SecretCodec(self.clone()))
    }
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
                .contains("another key file")
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
        let mut old = dataprotection::MAGIC.to_vec();
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

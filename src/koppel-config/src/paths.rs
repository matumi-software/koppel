//! Where Koppel keeps its files, the same folders as 0.1.x: `%ProgramData%\Koppel\{config,keys}` on
//! Windows, `/var/lib/koppel/{config,keys}` elsewhere, `/koppel/{config,keys}` in the Docker image
//! (set through `Paths__Config` and `Paths__Keys`), unless `Paths:Config` and `Paths:Keys` say
//! otherwise.

use std::io;
use std::path::{Path, PathBuf};

use crate::harden::harden_directory;

/// The config and keys directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KoppelPaths {
    /// The plain `settings.json`, `devices.json` and `attached.json`, and the encrypted
    /// `secrets.json`, `connection.json` and `ui-auth.json`.
    pub config: PathBuf,
    /// What encrypts the secret files, kept apart from them: 0.1.x's Data Protection key ring, and
    /// Koppel 0.2's key file where there is no DPAPI.
    pub keys: PathBuf,
    /// `Paths:KeyCertificate`, deprecated in 0.1.2 and not read by 0.2. Kept only to say so.
    pub key_certificate: Option<PathBuf>,
}

impl KoppelPaths {
    /// The directories for the configured values, each resolved against `content_root` when
    /// relative, or the default for the platform when not set.
    pub fn resolve(
        config: Option<&str>,
        keys: Option<&str>,
        key_certificate: Option<&str>,
        content_root: &Path,
    ) -> Self {
        let root = default_root();
        let pick = |value: Option<&str>| {
            value
                .filter(|v| !v.is_empty())
                .map(|v| content_root.join(v))
        };
        Self {
            config: pick(config).unwrap_or_else(|| root.join("config")),
            keys: pick(keys).unwrap_or_else(|| root.join("keys")),
            key_certificate: pick(key_certificate),
        }
    }

    pub fn new(config: impl Into<PathBuf>, keys: impl Into<PathBuf>) -> Self {
        Self {
            config: config.into(),
            keys: keys.into(),
            key_certificate: None,
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// Skakel's server state: the certificate, the signing keys and the clients. Encrypted.
    pub fn server_secrets_file(&self) -> PathBuf {
        self.config.join("secrets.json")
    }

    /// The client's server and credentials. Encrypted.
    pub fn connection_file(&self) -> PathBuf {
        self.config.join("connection.json")
    }

    /// The UI password's hash and the security stamp. Encrypted.
    pub fn ui_auth_file(&self) -> PathBuf {
        self.config.join("ui-auth.json")
    }

    pub fn devices_file(&self) -> PathBuf {
        self.config.join("devices.json")
    }

    pub fn attached_file(&self) -> PathBuf {
        self.config.join("attached.json")
    }

    /// The encrypted files, the ones a 0.1.x install has in Data Protection's format.
    pub fn secret_files(&self) -> [PathBuf; 3] {
        [
            self.server_secrets_file(),
            self.connection_file(),
            self.ui_auth_file(),
        ]
    }

    /// Makes the config and keys directories and restricts them, before anything is written to
    /// them. On Windows the data root above them (`%ProgramData%\Koppel`, which also holds the
    /// installer's driver record) is restricted too when they are in it, so a standard user cannot
    /// plant files there or swap what is under it.
    pub fn prepare(&self) -> io::Result<()> {
        let mut directories = Vec::new();
        if cfg!(windows) {
            let root = default_root();
            let parent = self.config.parent().map(Path::to_path_buf);
            if parent.is_some_and(|p| {
                p.to_string_lossy()
                    .eq_ignore_ascii_case(&root.to_string_lossy())
            }) {
                directories.push(root);
            }
        }
        directories.push(self.config.clone());
        directories.push(self.keys.clone());
        for directory in directories {
            std::fs::create_dir_all(&directory)?;
            harden_directory(&directory)?;
        }
        Ok(())
    }
}

/// `%ProgramData%\Koppel` on Windows, `/var/lib/koppel` elsewhere.
pub fn default_root() -> PathBuf {
    #[cfg(windows)]
    {
        program_data().join("Koppel")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/koppel")
    }
}

#[cfg(windows)]
fn program_data() -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_ProgramData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

    // SAFETY: the returned string is read once and freed with CoTaskMemFree, as the API asks.
    unsafe {
        match SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) {
            Ok(path) => {
                let wide = path.as_wide().to_vec();
                CoTaskMemFree(Some(path.0 as *const _));
                PathBuf::from(std::ffi::OsString::from_wide(&wide))
            }
            Err(_) => std::env::var_os("ProgramData")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_relative_paths_resolve_as_in_0_1() {
        let root = Path::new("content");
        let paths = KoppelPaths::resolve(None, Some("k"), Some(""), root);
        assert_eq!(paths.config, default_root().join("config"));
        assert_eq!(paths.keys, root.join("k"));
        assert_eq!(paths.key_certificate, None);
        assert_eq!(
            paths.server_secrets_file(),
            default_root().join("config").join("secrets.json")
        );
        #[cfg(not(windows))]
        assert_eq!(default_root(), PathBuf::from("/var/lib/koppel"));
        #[cfg(windows)]
        assert!(default_root().ends_with("Koppel") && default_root().is_absolute());
    }

    #[test]
    fn prepare_makes_both_directories() {
        let base = std::env::temp_dir().join(format!("koppel-paths-{}", crate::random_hex(6)));
        let paths = KoppelPaths::new(base.join("config"), base.join("keys"));
        paths.prepare().unwrap();
        assert!(paths.config.is_dir() && paths.keys.is_dir());
        let _ = std::fs::remove_dir_all(base);
    }
}

//! Koppel's files on disk: the JSON config files, the encrypted secret files and the server
//! certificate, in the same folders and with the same JSON as Koppel 0.1.x, and the one-time
//! conversion of a 0.1.x install's secrets.
//!
//! - [`KoppelPaths`]: where the files are, and [`KoppelPaths::prepare`], which restricts the
//!   folders as 0.1.x does.
//! - [`ConfigFile`]: one file, written through a temp file and a replace, with ordered change
//!   deliveries and hand edits picked up.
//! - [`files`]: what each file holds, and [`open`]: each file opened as 0.1.x opens it.
//! - [`SecretBox`]: the secret file format, machine DPAPI on Windows and AES-256-GCM with a key
//!   file elsewhere.
//! - [`convert_0_1_secrets`]: reads 0.1.x's Data Protection key ring and writes its secret files
//!   again in the new format.
//! - [`ServerCertificate`]: the server certificate from `secrets.json`, as rustls presents it.

mod certificate;
mod dataprotection;
#[cfg(windows)]
mod dpapi;
mod file;
pub mod files;
mod harden;
pub mod json;
mod migrate;
mod paths;
pub mod pkcs12;
mod secret;

use std::sync::Arc;

use ring::rand::{SecureRandom, SystemRandom};

pub use certificate::{CertificateError, ServerCertificate};
pub use dataprotection::{DataProtectionError, KOPPEL_PURPOSES, KeyRing};
pub use file::{
    CodecError, ConfigFile, ConfigFileError, FileCodec, Options, PlainText, Subscription,
    ValidateFn, Validator,
};
pub use harden::harden_directory;
pub use migrate::{Conversion, ConversionError, convert_0_1_secrets};
pub use paths::{KoppelPaths, default_root};
pub use secret::{SecretBox, SecretError, SecretFormat, format_of};

use files::{
    AttachedDevices, ClientConnection, KoppelSettings, ServerSecrets, SharedDevices, UiCredentials,
};

/// Each config file opened as 0.1.x opens it: the same defaults, codec and checks.
pub mod open {
    use super::*;

    /// `settings.json`: plain, hand edits checked.
    pub fn settings(paths: &KoppelPaths) -> ConfigFile<KoppelSettings> {
        ConfigFile::new(
            paths.settings_file(),
            KoppelSettings::default(),
            Options::default(),
        )
    }

    /// `devices.json`: plain, and a bad file starts with nothing shared.
    pub fn devices(paths: &KoppelPaths) -> ConfigFile<SharedDevices> {
        ConfigFile::new(
            paths.devices_file(),
            SharedDevices::empty(),
            Options {
                start_with_defaults_when_unusable: true,
                ..Options::default()
            },
        )
    }

    /// `attached.json`: plain, and a bad file starts with nothing remembered.
    pub fn attached(paths: &KoppelPaths) -> ConfigFile<AttachedDevices> {
        ConfigFile::new(
            paths.attached_file(),
            AttachedDevices::empty(),
            Options {
                start_with_defaults_when_unusable: true,
                ..Options::default()
            },
        )
    }

    /// `secrets.json`: encrypted. Empty until Skakel's state is saved in it.
    pub fn server_secrets(
        paths: &KoppelPaths,
        secrets: &Arc<SecretBox>,
    ) -> ConfigFile<ServerSecrets> {
        ConfigFile::new(
            paths.server_secrets_file(),
            ServerSecrets::default(),
            Options {
                codec: secrets.codec(),
                ..Options::default()
            },
        )
    }

    /// `connection.json`: encrypted.
    pub fn connection(
        paths: &KoppelPaths,
        secrets: &Arc<SecretBox>,
    ) -> ConfigFile<ClientConnection> {
        ConfigFile::new(
            paths.connection_file(),
            ClientConnection::default(),
            Options {
                codec: secrets.codec(),
                ..Options::default()
            },
        )
    }

    /// `ui-auth.json`: encrypted, and a password hash in another format is refused.
    pub fn ui_auth(paths: &KoppelPaths, secrets: &Arc<SecretBox>) -> ConfigFile<UiCredentials> {
        ConfigFile::new(
            paths.ui_auth_file(),
            UiCredentials::empty(),
            Options {
                codec: secrets.codec(),
                validate: Some(Arc::new(UiCredentials::invalid)),
                start_with_defaults_when_unusable: false,
            },
        )
    }
}

/// `bytes` random bytes as lower case hex, for temp file names.
pub(crate) fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    SystemRandom::new()
        .fill(&mut buffer)
        .expect("the system has random bytes");
    buffer.iter().map(|b| format!("{b:02x}")).collect()
}

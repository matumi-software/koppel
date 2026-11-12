//! The one-time upgrade of Koppel's files from 0.1.x and 0.2.0, run by `koppel` at start so that an
//! install converts without a step from the person who owns it.
//!
//! Everything that reads a format only an older Koppel wrote lives here, and nowhere else:
//!
//! - [`convert_0_1_secrets`]: 0.1.x sealed `secrets.json`, `connection.json` and `ui-auth.json` with
//!   ASP.NET Core Data Protection. This reads its key ring ([`KeyRing`]) and writes the files again
//!   with Koppel's [`SecretBox`], with the same JSON inside.
//! - [`reshape_secrets`]: `secrets.json` held its certificates as PKCS#12 ([`pkcs12`]) and carried
//!   fields of Skakel 0.1 that Skakel 0.3 does not read. This writes it again as Skakel's own state, with
//!   each certificate as DER and a PKCS#8 key. Koppel 0.1.0's top-level fields are carried over here too.
//!
//! [`upgrade`] runs both, in that order. Both are safe to run on every start: they find nothing to do
//! once the files are current. Neither deletes anything. The 0.1.x key ring stays where it is, and
//! `docs/config.md` says what it is and when to remove it.
//!
//! The daemon's own crates do not depend on the readers here: `koppel-config` reads and writes only the
//! current shape.

mod dataprotection;
mod migrate;
pub mod pkcs12;
mod reshape;

use koppel_config::{KoppelPaths, SecretBox};

pub use dataprotection::{DataProtectionError, KOPPEL_PURPOSES, KeyRing};
pub use migrate::{Conversion, ConversionError, convert_0_1_secrets};
pub use reshape::{Reshaped, reshape_secrets};

/// What [`upgrade`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgrade {
    /// The conversion from Data Protection.
    pub conversion: Conversion,
    /// The rewrite of `secrets.json`.
    pub secrets: Reshaped,
}

/// Brings the secret files up to date: converts 0.1.x's Data Protection files, then rewrites
/// `secrets.json` if it is in an older shape. Run once at start, after [`KoppelPaths::prepare`] and
/// before any file is opened. Logs what it did and why it could not. On an error the files it did
/// not reach are as they were, and the start then stops on the first one it cannot read.
pub fn upgrade(paths: &KoppelPaths, secrets: &SecretBox) -> Result<Upgrade, ConversionError> {
    let conversion = convert_0_1_secrets(paths, secrets)?;
    let reshaped = reshape_secrets(paths, secrets)?;
    Ok(Upgrade {
        conversion,
        secrets: reshaped,
    })
}

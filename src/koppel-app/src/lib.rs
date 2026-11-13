//! Koppel's roles, below the web server and above the USB layer.
//!
//! - [`App`]: the config files opened and the roles started over them: Skakel's server and client,
//!   the USB layer, the activity log and the push bus. [`AppOptions`], [`NetworkOptions`] and
//!   [`UsbOptions`] say how.
//! - [`roles`]: the server and client roles on Skakel, and the USB layer as the UI sees it.
//! - [`connection`]: the saved connection as the UI sees and changes it.
//! - [`activity`]: the activity log. [`push`]: the live push's bus.
//! - [`tray`]: what the Windows service tells its tray icons, in rules that run on every platform.
//! - [`CertificateSlot`]: the server certificate, replaceable while it runs.

pub mod activity;
mod app;
pub mod certificate;
pub mod connection;
pub mod host;
pub mod push;
pub mod random;
pub mod roles;
pub mod sync;
pub mod tasks;
pub mod tray;

pub use activity::{ActivityEvent, ActivityLog, ActivityOutcome, ActivityReason, EntityRef};
pub use app::{App, AppOptions, NetworkOptions, StartError, UsbOptions};
pub use certificate::CertificateSlot;
pub use host::{Host, machine_name, os_label};
pub use tasks::Tasks;

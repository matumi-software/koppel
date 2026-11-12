//! Koppel's Windows app, ported from 0.1.x's `Tray/*`, its event log logging and its Windows service
//! hosting:
//!
//! - [`service`]: Koppel.exe as the LocalSystem service, with the tray's controls to shut it down
//!   (200) and restart it (201), and `--restart-service`, the helper that starts it again.
//! - [`event_log`]: what `tracing` logs, written to the Application event log.
//! - [`launcher`]: the service starts a tray in each signed-in session, as the user, and writes it
//!   what to show over its standard input ([`wire`]).
//! - [`tray`]: the icon in the notification area, in KoppelDesktop.exe (`--tray <url>`).
//! - [`opener`]: Koppel in the Start menu (`KoppelDesktop.exe --open`), which starts the service
//!   when it is stopped and opens the UI.
//!
//! Everything here is Windows only except [`wire`], which is plain data, so its tests run on Linux
//! too; on other systems the rest of the crate is empty.

pub mod wire;

#[cfg(windows)]
pub mod event_log;
#[cfg(windows)]
mod health;
#[cfg(windows)]
pub mod launcher;
#[cfg(windows)]
pub mod opener;
#[cfg(windows)]
pub mod service;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod ui_url;

/// The Windows service's name, which the installer registers.
#[cfg(windows)]
pub const SERVICE_NAME: &str = "Koppel";

/// The user-defined service control that shuts Koppel down. They run from 128 to 255.
#[cfg(windows)]
pub const SHUT_DOWN_CONTROL: u32 = 200;

/// The user-defined service control that restarts Koppel.
#[cfg(windows)]
pub const RESTART_CONTROL: u32 = 201;

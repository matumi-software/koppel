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
//! Everything here is Windows only; on other systems the crate is empty.

#![cfg(windows)]

pub mod event_log;
mod health;
pub mod launcher;
pub mod opener;
pub mod service;
mod shell;
pub mod tray;
pub mod ui_url;
pub mod wire;

/// The Windows service's name, which the installer registers.
pub const SERVICE_NAME: &str = "Koppel";

/// The user-defined service control that shuts Koppel down. They run from 128 to 255.
pub const SHUT_DOWN_CONTROL: u32 = 200;

/// The user-defined service control that restarts Koppel.
pub const RESTART_CONTROL: u32 = 201;

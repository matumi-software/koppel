//! KoppelDesktop.exe: Koppel's tray icon and its Start menu entry, a Windows program, so neither
//! opens a console window.
//!
//! - `--tray <ui url>`: the tray icon, which the service starts in each signed-in session.
//! - `--open [url]`: the Start menu entry and the installer's Launch button. Starts the service if it
//!   is stopped (asking for an administrator), waits for the UI and opens it in the browser.
//! - `--start-service`: what `--open` runs elevated to start the service.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use koppel_windows::{opener, tray};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some(tray::ARGUMENT) => match args.get(1) {
            Some(url) => tray::run(url),
            None => 2,
        },
        Some(opener::START_ARGUMENT) => opener::start_service(),
        Some(opener::OPEN_ARGUMENT) | None => opener::open(args.get(1).map(String::as_str)),
        Some(_) => 2,
    };
    std::process::ExitCode::from(code)
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "KoppelDesktop is the Windows app's tray icon and Start menu entry; it does nothing here."
    );
}

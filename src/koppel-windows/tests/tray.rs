//! The launcher's pipe and process start, with the real KoppelDesktop.exe, in this session as this
//! user (the service uses the signed-in user's token instead, which only LocalSystem may ask for).

#![cfg(windows)]

use std::path::Path;
use std::time::Duration;

use koppel_app::tray::TrayState;
use koppel_windows::launcher::TrayProcess;
use koppel_windows::wire;

fn desktop() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_koppel-desktop"))
}

#[test]
fn a_tray_given_an_address_that_is_not_https_ends_at_once() {
    let tray = TrayProcess::start(desktop(), "http://localhost:5240/", None).unwrap();
    assert_eq!(tray.wait(Duration::from_secs(10)), Some(2));
}

/// Shows an icon in the notification area for a moment, so it is not run by default:
/// `cargo test -p koppel-windows -- --ignored`.
#[test]
#[ignore = "shows a tray icon"]
fn a_tray_reads_its_lines_and_ends_when_the_pipe_closes() {
    let mut tray = TrayProcess::start(desktop(), "https://localhost:5999/", None).unwrap();
    let state = TrayState {
        tip: "Koppel: test tray from cargo test".into(),
        attention: true,
    };
    tray.send(&wire::state_line(&state), false);
    tray.send("a line it cannot read", true);
    assert_eq!(
        tray.wait(Duration::from_millis(500)),
        None,
        "it keeps running"
    );
    tray.close_pipe();
    assert_eq!(tray.wait(Duration::from_secs(10)), Some(0));
}

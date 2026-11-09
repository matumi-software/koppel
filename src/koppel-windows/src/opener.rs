//! `KoppelDesktop.exe --open`, behind Koppel in the Start menu and the installer's Launch button:
//! opens the UI in the user's browser, and first starts the service if it is not running. Starting a
//! service takes an administrator, so that part runs again elevated as `--start-service`, which asks
//! through UAC. Ported from 0.1.x's `KoppelOpener`.
//!
//! `--open <url>` opens that address instead of the one the service published, for trying it
//! against a Koppel on another port.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::shell::{self, Icon, describe, wait_for};
use crate::{SERVICE_NAME, health, ui_url};

/// Opens the UI, starting the service first when it is stopped.
pub const OPEN_ARGUMENT: &str = "--open";
/// What the elevated copy is run with: starts the service and waits until it runs.
pub const START_ARGUMENT: &str = "--start-service";

const START_WAIT: Duration = Duration::from_secs(30);
const UI_WAIT: Duration = Duration::from_secs(60);

const NOT_RUNNING: &str = "Koppel is not running. Start it from Koppel in the Start menu, or start the Koppel service in Services.";

/// `--open [url]`. 0 when the UI was opened.
pub fn open(url: Option<&str>) -> u8 {
    let status = match query() {
        Ok(state) => state,
        Err(Query::NotInstalled) => {
            shell::show(
                None,
                "The Koppel service is not installed. Run Koppel-Setup.exe again to repair it.",
                Icon::Error,
            );
            return 1;
        }
        Err(Query::Failed(e)) => {
            shell::show(
                None,
                &format!("Koppel could not ask Windows whether it runs: {e}"),
                Icon::Error,
            );
            return 1;
        }
    };
    if matches!(status, ServiceState::Stopped | ServiceState::StopPending) {
        if !shell::ask(
            None,
            "Koppel is not running. Start it now?\n\nStarting it takes an administrator.",
            false,
        ) {
            return 1;
        }
        if !start_elevated() {
            return 1;
        }
    }

    // Read after a start, because the service writes it as it starts.
    let url = url.and_then(ui_url::checked).unwrap_or_else(ui_url::read);
    if !wait_for_ui(&url) {
        shell::show(
            None,
            &format!(
                "Koppel is running, but its UI at {url} does not answer yet. Try again in a minute."
            ),
            Icon::Error,
        );
        return 1;
    }
    match shell::open(None, &url) {
        Ok(()) => 0,
        Err(e) => {
            shell::show(
                None,
                &format!("Koppel could not open {url}: {e}"),
                Icon::Error,
            );
            1
        }
    }
}

enum Query {
    NotInstalled,
    Failed(String),
}

fn query() -> Result<ServiceState, Query> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| Query::Failed(describe(e)))?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .map_err(|e| match e {
            windows_service::Error::Winapi(ref io)
                if io.raw_os_error()
                    == Some(windows::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST.0 as i32) =>
            {
                Query::NotInstalled
            }
            other => Query::Failed(describe(other)),
        })?;
    service
        .query_status()
        .map(|s| s.current_state)
        .map_err(|e| Query::Failed(describe(e)))
}

/// Runs this program again as an administrator to start the service, and says why if that did not
/// work.
fn start_elevated() -> bool {
    let exe = match std::env::current_exe() {
        Ok(exe) => HSTRING::from(exe.as_os_str()),
        Err(_) => {
            shell::show(None, NOT_RUNNING, Icon::Error);
            return false;
        }
    };
    let arguments = HSTRING::from(START_ARGUMENT);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: the structure and its strings live through the call.
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            shell::show(None, NOT_RUNNING, Icon::Information);
        } else {
            shell::show(
                None,
                &format!("Koppel could not be started: {}", e.message()),
                Icon::Error,
            );
        }
        return false;
    }
    if info.hProcess.is_invalid() {
        shell::show(None, NOT_RUNNING, Icon::Error);
        return false;
    }
    let mut code = 1u32;
    // SAFETY: the process handle is ours (SEE_MASK_NOCLOSEPROCESS) and closed once, here.
    unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0 {
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
        }
        let _ = CloseHandle(info.hProcess);
    }
    // The elevated copy showed its own error otherwise.
    code == 0
}

/// `--start-service`, elevated: starts the service and waits until Windows says it runs. 0 when it
/// does.
pub fn start_service() -> u8 {
    match start_and_wait() {
        Ok(()) => 0,
        Err(e) => {
            shell::show(
                None,
                &format!(
                    "Koppel could not be started: {e}\n\nThe Application log in Event Viewer (source Koppel) says more."
                ),
                Icon::Error,
            );
            1
        }
    }
}

fn start_and_wait() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(describe)?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::START,
        )
        .map_err(describe)?;
    let state = || {
        service
            .query_status()
            .map(|s| s.current_state)
            .map_err(describe)
    };
    if state()? == ServiceState::StopPending {
        wait_for(&state, ServiceState::Stopped, START_WAIT)?;
    }
    if state()? == ServiceState::Stopped {
        service.start::<&str>(&[]).map_err(describe)?;
    }
    wait_for(&state, ServiceState::Running, START_WAIT)
}

/// Asks the UI's health endpoint until it answers, for up to a minute.
fn wait_for_ui(url: &str) -> bool {
    let Some((host, port)) = host_and_port(url) else {
        return false;
    };
    let deadline = Instant::now() + UI_WAIT;
    loop {
        if health::answers(&host, port, Duration::from_secs(5)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The host and port of an `https://` address; 443 when it names none.
fn host_and_port(url: &str) -> Option<(String, u16)> {
    let authority = url.strip_prefix("https://")?.split('/').next()?;
    let (host, port) = match authority.rfind(':') {
        Some(i) if !authority[i..].contains(']') => {
            (&authority[..i], authority[i + 1..].parse().ok()?)
        }
        _ => (authority, 443),
    };
    Some((host.to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_and_port_come_from_the_address() {
        assert_eq!(
            host_and_port("https://localhost:5240/"),
            Some(("localhost".into(), 5240))
        );
        assert_eq!(
            host_and_port("https://[::1]:5250/"),
            Some(("[::1]".into(), 5250))
        );
        assert_eq!(host_and_port("https://[::1]/"), Some(("[::1]".into(), 443)));
        assert_eq!(host_and_port("https://pc/"), Some(("pc".into(), 443)));
        assert_eq!(host_and_port("http://pc/"), None);
    }
}

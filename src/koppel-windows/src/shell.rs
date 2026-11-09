//! Message boxes, opening an address in the user's browser, and the Koppel service's controls, for
//! the tray and the Start menu entry.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK,
    MB_SETFOREGROUND, MB_YESNO, MESSAGEBOX_STYLE, MessageBoxW, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, w};
use windows_service::service::{ServiceAccess, ServiceState, UserEventCode};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::SERVICE_NAME;

/// What a message box shows besides its text.
#[derive(Debug, Clone, Copy)]
pub enum Icon {
    Error,
    Information,
}

/// Shows `text` with an OK button.
pub fn show(owner: Option<HWND>, text: &str, icon: Icon) {
    let icon = match icon {
        Icon::Error => MB_ICONERROR,
        Icon::Information => MB_ICONINFORMATION,
    };
    message_box(owner, text, MB_OK | icon);
}

/// Asks `question` with Yes and No, No the default when `no_first`. True for Yes.
pub fn ask(owner: Option<HWND>, question: &str, no_first: bool) -> bool {
    let default = if no_first {
        MB_DEFBUTTON2
    } else {
        MESSAGEBOX_STYLE(0)
    };
    message_box(owner, question, MB_YESNO | MB_ICONQUESTION | default) == IDYES.0
}

fn message_box(owner: Option<HWND>, text: &str, style: MESSAGEBOX_STYLE) -> i32 {
    // SAFETY: both strings live through the call.
    unsafe {
        MessageBoxW(
            owner,
            &HSTRING::from(text),
            w!("Koppel"),
            style | MB_SETFOREGROUND,
        )
    }
    .0
}

/// Opens `url` in the user's browser, as Explorer opens an address. The error says why it did not.
pub fn open(owner: Option<HWND>, url: &str) -> Result<(), String> {
    // SAFETY: the strings live through the call.
    let result = unsafe {
        ShellExecuteW(
            owner,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute answers a number above 32 when it worked.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

/// Sends the service a user-defined control. Interactive users may, though they may not stop or
/// start it.
pub fn send_control(code: u32) -> Result<(), String> {
    let code = UserEventCode::from_raw(code).map_err(|e| e.to_string())?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(describe)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::USER_DEFINED_CONTROL)
        .map_err(describe)?;
    service.notify(code).map(|_| ()).map_err(describe)
}

/// A service manager error in words, the system's own message for a Windows error.
pub fn describe(e: windows_service::Error) -> String {
    match e {
        windows_service::Error::Winapi(e) => e.to_string(),
        other => other.to_string(),
    }
}

/// Waits until `state` says `wanted`, or says why not.
pub fn wait_for(
    state: &dyn Fn() -> Result<ServiceState, String>,
    wanted: ServiceState,
    within: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + within;
    loop {
        let now = state()?;
        if now == wanted {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "it is still {} after {} seconds",
                words(now),
                within.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn words(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Stopped => "stopped",
        ServiceState::StartPending => "starting",
        ServiceState::StopPending => "stopping",
        ServiceState::Running => "running",
        _ => "paused",
    }
}

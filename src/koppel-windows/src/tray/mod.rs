//! `KoppelDesktop.exe --tray <ui url>`: the icon in the notification area of a signed-in user's
//! session, ported from 0.1.x's `TrayIcon`. The service starts it there ([`crate::launcher`]) with
//! the user's own token. Clicking it opens the UI in the user's browser; its menu also restarts and
//! shuts down the service. Its standard input is a pipe from the service, which says what the icon
//! shows ([`crate::wire`]): when that closes, because the service stopped or died, the icon is removed
//! and the process ends.
//!
//! Plain Win32: `Shell_NotifyIcon` with `NOTIFYICON_VERSION_4`, a hidden top-level window (only
//! those hear `TaskbarCreated`, so the icon comes back after Explorer restarts), and notifications
//! with `NIIF_RESPECT_QUIET_TIME`, so Windows holds them back during Do not disturb.

mod attention;

use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use koppel_web::tray::{TrayNotice, TrayState};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_RESPECT_QUIET_TIME, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONUSERCLICK, NIN_SELECT, NINF_KEY,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, GetSystemMetrics, HICON, HMENU, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow, SetMenuDefaultItem,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_NULL,
    WNDCLASSEXW,
};
use windows::core::{PCWSTR, w};

use crate::shell::{self, Icon};
use crate::wire::{self, Message};
use crate::{RESTART_CONTROL, SHUT_DOWN_CONTROL, ui_url};

/// Starts the tray: `--tray <ui url>`.
pub const ARGUMENT: &str = "--tray";

const CALLBACK_MESSAGE: u32 = WM_APP + 1;
const STATE_MESSAGE: u32 = WM_APP + 2;
const NOTICE_MESSAGE: u32 = WM_APP + 3;
const ICON_ID: u32 = 1;
/// The icon picked with the keyboard: `NIN_SELECT | NINF_KEY`.
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const OPEN_COMMAND: usize = 1;
const SHUT_DOWN_COMMAND: usize = 2;
const RESTART_COMMAND: usize = 3;
/// The icon resource build.rs embeds.
const ICON_RESOURCE: u16 = 1;

/// What the window procedure needs. Set once before the window exists.
struct Tray {
    url: String,
    icon: isize,
    menu: isize,
    taskbar_created: u32,
}

static TRAY: OnceLock<Tray> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ATTENTION_ICON: AtomicIsize = AtomicIsize::new(0);
static ICON_SIZE: AtomicU32 = AtomicU32::new(16);
/// What the service said last, and the notices not shown yet. Written by the pipe reader.
static STATE: Mutex<Option<TrayState>> = Mutex::new(None);
static NOTICES: Mutex<VecDeque<TrayNotice>> = Mutex::new(VecDeque::new());
/// The page the last notification opens when clicked.
static NOTICE_PAGE: Mutex<String> = Mutex::new(String::new());

/// Runs the tray until the service closes the pipe. 0 then, 2 for an address that is not HTTPS, 1
/// when the window cannot be made.
pub fn run(url: &str) -> u8 {
    let Some(url) = ui_url::checked(url) else {
        return 2;
    };
    // SAFETY: plain Win32 calls with arguments that live through them. The window procedure only
    // reads the statics above.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return 1;
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("KoppelTray"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return 1;
        }
        let size = GetSystemMetrics(SM_CXSMICON);
        ICON_SIZE.store(size.max(1) as u32, Ordering::Relaxed);
        let icon = LoadImageW(
            Some(instance.into()),
            PCWSTR(ICON_RESOURCE as usize as *const u16),
            IMAGE_ICON,
            size,
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
        .map(|h| h.0 as isize)
        .unwrap_or(0);
        let Ok(menu) = CreatePopupMenu() else {
            return 1;
        };
        let _ = AppendMenuW(menu, MF_STRING, OPEN_COMMAND, w!("Open Koppel"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, RESTART_COMMAND, w!("Restart Koppel"));
        let _ = AppendMenuW(menu, MF_STRING, SHUT_DOWN_COMMAND, w!("Shut down Koppel"));
        let _ = SetMenuDefaultItem(menu, OPEN_COMMAND as u32, 0);
        let _ = TRAY.set(Tray {
            url,
            icon,
            menu: menu.0 as isize,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        });

        // A hidden top-level window, not a message-only one: only top-level windows hear
        // TaskbarCreated.
        let Ok(window) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("KoppelTray"),
            w!("Koppel"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return 1;
        };
        WINDOW.store(window.0 as isize, Ordering::SeqCst);
        add_icon(window);

        let reader = window.0 as isize;
        std::thread::Builder::new()
            .name("Koppel service pipe".into())
            .spawn(move || read_service(HWND(reader as *mut _)))
            .ok();

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    0
}

/// Reads what the service sends on standard input until it closes its end of the pipe, then closes
/// the window. A line this tray cannot read is skipped.
fn read_service(window: HWND) {
    let stdin = std::io::stdin();
    let mut line = String::new();
    let mut input = stdin.lock();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        match wire::read(line.trim_end()) {
            Some(Message::State(state)) => {
                *STATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(state);
                post(window, STATE_MESSAGE);
            }
            Some(Message::Notice(notice)) => {
                NOTICES
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back(notice);
                post(window, NOTICE_MESSAGE);
            }
            None => {}
        }
    }
    post(window, WM_CLOSE);
}

fn post(window: HWND, message: u32) {
    // SAFETY: posting to our own window, from any thread, is allowed.
    let _ = unsafe { PostMessageW(Some(window), message, WPARAM(0), LPARAM(0)) };
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(tray) = TRAY.get() else {
        // SAFETY: the default handling for a message this window does not handle.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    match message {
        CALLBACK_MESSAGE => {
            // NOTIFYICON_VERSION_4: the event is in the low word of lParam, the anchor point in
            // wParam.
            match (lparam.0 & 0xFFFF) as u32 {
                NIN_SELECT | NIN_KEYSELECT => open_ui(hwnd, ""),
                NIN_BALLOONUSERCLICK => {
                    let page = NOTICE_PAGE
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    open_ui(hwnd, &page);
                }
                WM_CONTEXTMENU => {
                    let x = (wparam.0 & 0xFFFF) as u16 as i16 as i32;
                    let y = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                    show_menu(hwnd, tray, x, y);
                }
                _ => {}
            }
            LRESULT(0)
        }
        STATE_MESSAGE => {
            show_state(hwnd);
            LRESULT(0)
        }
        NOTICE_MESSAGE => {
            loop {
                let next = NOTICES
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front();
                let Some(notice) = next else { break };
                show_notice(hwnd, &notice);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            remove_icon(hwnd);
            // SAFETY: our own window.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        m if m == tray.taskbar_created && m != 0 => {
            // Explorer restarted and lost every icon.
            add_icon(hwnd);
            LRESULT(0)
        }
        // SAFETY: the default handling for a message this window does not handle.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

fn add_icon(hwnd: HWND) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK_MESSAGE;
    set_state(&mut data);
    // SAFETY: the structure is ours and sized.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
}

/// Shows the latest state from the service: the tooltip, and the icon with a dot when something
/// needs the user.
fn show_state(hwnd: HWND) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    set_state(&mut data);
    // SAFETY: the structure is ours and sized.
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

/// Shows a Windows notification from the icon. Windows holds it back during Do not disturb and
/// full-screen apps. A click on it opens the UI at the notice's page.
fn show_notice(hwnd: HWND, notice: &TrayNotice) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_INFO;
    // No info icon: the notification carries Koppel's own.
    data.dwInfoFlags = NIIF_RESPECT_QUIET_TIME;
    copy(&mut data.szInfoTitle, &notice.title);
    copy(&mut data.szInfo, &notice.text);
    *NOTICE_PAGE.lock().unwrap_or_else(PoisonError::into_inner) = notice.page.clone();
    // SAFETY: the structure is ours and sized.
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

fn set_state(data: &mut NOTIFYICONDATAW) {
    let state = STATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(TrayState::plain);
    let plain = TRAY.get().map_or(0, |t| t.icon);
    let mut icon = plain;
    if state.attention && plain != 0 {
        let mut attention = ATTENTION_ICON.load(Ordering::Relaxed);
        if attention == 0 {
            let size = ICON_SIZE.load(Ordering::Relaxed) as i32;
            attention = attention::create(HICON(plain as *mut _), size).map_or(0, |h| h.0 as isize);
            ATTENTION_ICON.store(attention, Ordering::Relaxed);
        }
        if attention != 0 {
            icon = attention;
        }
    }
    data.hIcon = HICON(icon as *mut _);
    copy(&mut data.szTip, &state.tip);
}

/// Copies `text` into a fixed buffer, cut to leave room for the terminating zero.
fn copy(buffer: &mut [u16], text: &str) {
    buffer.fill(0);
    let room = buffer.len().saturating_sub(1);
    for (slot, unit) in buffer.iter_mut().take(room).zip(text.encode_utf16()) {
        *slot = unit;
    }
}

fn remove_icon(hwnd: HWND) {
    let data = icon_data(hwnd);
    // SAFETY: the structure is ours and sized.
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

fn show_menu(hwnd: HWND, tray: &Tray, x: i32, y: i32) {
    // SAFETY: our own window and menu. Without the foreground the menu does not close when the user
    // clicks elsewhere; WM_NULL after it is the documented fix.
    let command = unsafe {
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenuEx(
            HMENU(tray.menu as *mut _),
            (TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY).0,
            x,
            y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        command.0 as usize
    };
    match command {
        OPEN_COMMAND => open_ui(hwnd, ""),
        SHUT_DOWN_COMMAND => ask_and_send(
            hwnd,
            "Shut down Koppel?\n\nDevices this PC shares or has attached stop working until Koppel starts again. It starts again with Windows, or from Koppel in the Start menu.",
            SHUT_DOWN_CONTROL,
            "shut down",
        ),
        RESTART_COMMAND => ask_and_send(
            hwnd,
            "Restart Koppel?\n\nDevices this PC shares or has attached pause while it restarts, and clients attach theirs again when it is back. If no password is set yet, setup opens again.",
            RESTART_CONTROL,
            "restarted",
        ),
        _ => {}
    }
}

/// Opens the UI in the user's browser, at `page` (such as `#clients`, checked by [`wire`]).
fn open_ui(hwnd: HWND, page: &str) {
    let Some(tray) = TRAY.get() else { return };
    let url = format!("{}{page}", tray.url);
    if let Err(e) = shell::open(Some(hwnd), &url) {
        shell::show(
            Some(hwnd),
            &format!("Koppel could not open {url}: {e}"),
            Icon::Error,
        );
    }
}

/// Asks first, No being the default, then sends the service its control. Interactive users may send
/// a service its user-defined controls but may not stop or start it, so the service does that itself.
/// When it stops, the pipe closes and this icon goes with it; after a restart the service starts a
/// new one.
fn ask_and_send(hwnd: HWND, question: &str, control: u32, failed: &str) {
    if !shell::ask(Some(hwnd), question, true) {
        return;
    }
    if let Err(e) = shell::send_control(control) {
        shell::show(
            Some(hwnd),
            &format!("Koppel could not be {failed}: {e}"),
            Icon::Error,
        );
    }
}

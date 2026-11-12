//! Keeps a tray icon running in every active user session while the service runs, the way Apollo
//! does: the service owns the icons, so they come and go with it. Ported from 0.1.x's `TrayLauncher`.
//!
//! Each tray is `KoppelDesktop.exe --tray <ui url>`, run with the session user's token from
//! `WTSQueryUserToken`, not as LocalSystem, so the browser it opens is the user's. Its standard input
//! is the read end of a pipe from here, passed with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` so it
//! inherits nothing else, and it carries what the icon shows ([`crate::wire`]). Closing the pipe
//! tells the tray to remove its icon and exit, and the same happens when the service dies. A tray
//! that ends is started again, after a minute if it ended within 30 seconds. A tray that fails never
//! stops Koppel.

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use koppel_web::tray::TrayFeed;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW, WTSActive, WTSEnumerateSessionsW, WTSFreeMemory,
    WTSQueryUserToken,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::wire;

/// A tray that ends this soon after it started is started again only after `BACKOFF`.
const QUICK_EXIT: Duration = Duration::from_secs(30);
const BACKOFF: Duration = Duration::from_secs(60);
/// How long the trays get to exit by themselves when the service stops.
const EXIT_WAIT: Duration = Duration::from_secs(3);
/// Room for many messages, so a tray that is slow to read rarely holds up the launcher.
const PIPE_BUFFER: u32 = 64 * 1024;
/// Lines waiting for a tray's writer thread. A tray that is stuck reading fills it, and further
/// lines are dropped (a state is sent again on the next round, and a notice for a stuck tray is of
/// no use) instead of holding up the launcher.
const QUEUE: usize = 32;

/// The tray program's names next to Koppel.exe: as installed, and as Cargo builds it.
const DESKTOP_NAMES: [&str; 2] = ["KoppelDesktop.exe", "koppel-desktop.exe"];

/// The running launcher. [`Trays::close`] closes every tray.
pub struct Trays {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Trays {
    /// Closes every pipe so each tray removes its icon, ends any that have not gone after a few
    /// seconds, and stops the launcher.
    pub async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.task.await;
    }
}

/// Starts the launcher: the trays open `ui_url` and show what `feed` says.
pub fn start(ui_url: String, feed: TrayFeed) -> Trays {
    let (stop, stopped) = oneshot::channel();
    Trays {
        stop: Some(stop),
        task: tokio::spawn(run(ui_url, feed, stopped)),
    }
}

async fn run(ui_url: String, mut feed: TrayFeed, mut stopped: oneshot::Receiver<()>) {
    let Some(exe) = desktop_exe() else {
        tracing::warn!(
            "KoppelDesktop.exe is not next to Koppel.exe, so there is no tray icon. Run Koppel-Setup.exe again to repair it."
        );
        return;
    };
    let mut launcher = Launcher {
        exe,
        ui_url,
        trays: HashMap::new(),
        not_before: HashMap::new(),
    };
    loop {
        launcher.sync();
        launcher.publish(&mut feed);
        tokio::select! {
            () = feed.changed() => {}
            _ = &mut stopped => break,
        }
    }
    launcher.close_all().await;
}

/// The tray program next to this one.
fn desktop_exe() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_owned();
    DESKTOP_NAMES
        .iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
}

struct Launcher {
    exe: PathBuf,
    ui_url: String,
    trays: HashMap<u32, TrayProcess>,
    not_before: HashMap<u32, Instant>,
}

impl Launcher {
    /// Forgets trays that ended and starts one in each active session that has none.
    fn sync(&mut self) {
        let now = Instant::now();
        self.trays.retain(|&session, tray| {
            if !tray.has_exited() {
                return true;
            }
            if now - tray.started < QUICK_EXIT {
                // One that keeps ending at once is not started every few seconds.
                self.not_before.insert(session, now + BACKOFF);
                tracing::warn!(
                    "The tray icon in session {session} ended right after it started; trying again in a minute"
                );
            }
            false
        });
        let sessions = match active_sessions() {
            Ok(sessions) => sessions,
            Err(e) => {
                tracing::warn!("Could not check the signed-in sessions for tray icons: {e}");
                return;
            }
        };
        for session in sessions {
            if self.trays.contains_key(&session)
                || self.not_before.get(&session).is_some_and(|&t| now < t)
            {
                continue;
            }
            self.not_before.remove(&session);
            // Fails while the user is still signing in, and for a session with no user; the next
            // round tries again.
            let token = match user_token(session) {
                Ok(token) => token,
                Err(e) => {
                    tracing::debug!(
                        "No user token for session {session} yet ({e}); trying again shortly"
                    );
                    continue;
                }
            };
            match TrayProcess::start(&self.exe, &self.ui_url, Some(&token)) {
                Ok(tray) => {
                    tracing::info!("Started the tray icon in session {session}");
                    self.trays.insert(session, tray);
                }
                Err(e) => {
                    tracing::warn!(
                        "Could not start the tray icon in session {session}; trying again in a minute: {e}"
                    );
                    self.not_before.insert(session, now + BACKOFF);
                }
            }
        }
    }

    /// Tells every tray the state, when it differs from what that tray was last told, and every
    /// notice once. With nobody signed in a notice is dropped.
    fn publish(&mut self, feed: &mut TrayFeed) {
        let line = wire::state_line(&feed.state());
        for tray in self.trays.values_mut() {
            tray.send(&line, false);
        }
        for notice in feed.notices() {
            let line = wire::notice_line(&notice);
            for tray in self.trays.values_mut() {
                tray.send(&line, true);
            }
        }
    }

    async fn close_all(&mut self) {
        for tray in self.trays.values_mut() {
            tray.close_pipe();
        }
        let deadline = Instant::now() + EXIT_WAIT;
        while self.trays.values().any(|t| !t.has_exited()) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for tray in self.trays.values() {
            if !tray.has_exited() {
                tray.kill();
            }
        }
        self.trays.clear();
    }
}

/// The sessions with a signed-in user at the screen. Session 0 is the services' own and has no
/// desktop to show an icon on.
fn active_sessions() -> windows::core::Result<Vec<u32>> {
    let mut sessions: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: the out parameters are ours, and the list is freed below.
    unsafe {
        WTSEnumerateSessionsW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            0,
            1,
            &mut sessions,
            &mut count,
        )?;
        let list = std::slice::from_raw_parts(sessions, count as usize);
        let active = list
            .iter()
            .filter(|s| s.State == WTSActive && s.SessionId != 0)
            .map(|s| s.SessionId)
            .collect();
        WTSFreeMemory(sessions.cast());
        Ok(active)
    }
}

/// The primary token of the user signed in to `session`. Only LocalSystem may ask.
fn user_token(session: u32) -> windows::core::Result<OwnedHandle> {
    let mut token = HANDLE::default();
    // SAFETY: the out parameter is ours; the handle is owned from here.
    unsafe {
        WTSQueryUserToken(session, &mut token)?;
        Ok(OwnedHandle::from_raw_handle(token.0))
    }
}

/// One running tray and the write end of its pipe.
pub struct TrayProcess {
    process: OwnedHandle,
    /// Lines for the writer thread, which owns the write end of the pipe. Dropping this ends the
    /// thread, which closes the pipe.
    lines: Option<SyncSender<String>>,
    started: Instant,
    /// The state line this tray was last sent, so a state is sent once and not every round.
    sent: Option<String>,
}

impl TrayProcess {
    /// Starts `exe --tray <ui_url>` with the read end of a new pipe as its standard input: as the
    /// user `token` belongs to, on that user's desktop, or, without a token, as this process's user
    /// in this session, for trying the tray and the pipe without the service.
    pub fn start(
        exe: &Path,
        ui_url: &str,
        token: Option<&OwnedHandle>,
    ) -> windows::core::Result<Self> {
        let (read, write) = pipe()?;
        let mut environment: *mut c_void = std::ptr::null_mut();
        let mut attributes = vec![0u8; 0];
        let mut list = LPPROC_THREAD_ATTRIBUTE_LIST(std::ptr::null_mut());
        let result = (|| -> windows::core::Result<Self> {
            let token = token.map(|t| HANDLE(t.as_raw_handle()));
            // SAFETY: every pointer handed to these calls is ours and outlives the call; the
            // attribute list, the environment block and the handles are freed below.
            unsafe {
                if let Some(token) = token {
                    CreateEnvironmentBlock(&mut environment, Some(token), false)?;
                }
                let mut size = 0usize;
                // Fails by design, saying how big the list must be.
                let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
                attributes.resize(size, 0);
                list = LPPROC_THREAD_ATTRIBUTE_LIST(attributes.as_mut_ptr().cast());
                InitializeProcThreadAttributeList(Some(list), 1, None, &mut size)?;
                let inherited = HANDLE(read.as_raw_handle());
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some((&raw const inherited).cast()),
                    size_of::<HANDLE>(),
                    None,
                    None,
                )?;

                let mut desktop: Vec<u16> = "winsta0\\default\0".encode_utf16().collect();
                let mut startup = STARTUPINFOEXW::default();
                startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
                startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                startup.StartupInfo.hStdInput = inherited;
                startup.lpAttributeList = list;
                if token.is_some() {
                    startup.StartupInfo.lpDesktop = PWSTR(desktop.as_mut_ptr());
                }
                let mut command: Vec<u16> = format!("\"{}\" --tray \"{ui_url}\"\0", exe.display())
                    .encode_utf16()
                    .collect();
                let application: Vec<u16> = wide(exe.as_os_str());
                let folder: Option<Vec<u16>> = exe.parent().map(|f| wide(f.as_os_str()));
                let folder = folder
                    .as_ref()
                    .map_or(PCWSTR::null(), |f| PCWSTR(f.as_ptr()));
                let flags =
                    CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW;
                let mut process = PROCESS_INFORMATION::default();
                let env = (!environment.is_null()).then_some(environment.cast_const());
                match token {
                    Some(token) => CreateProcessAsUserW(
                        Some(token),
                        PCWSTR(application.as_ptr()),
                        Some(PWSTR(command.as_mut_ptr())),
                        None,
                        None,
                        true,
                        flags,
                        env,
                        folder,
                        &startup.StartupInfo,
                        &mut process,
                    )?,
                    None => CreateProcessW(
                        PCWSTR(application.as_ptr()),
                        Some(PWSTR(command.as_mut_ptr())),
                        None,
                        None,
                        true,
                        flags,
                        env,
                        folder,
                        &startup.StartupInfo,
                        &mut process,
                    )?,
                }
                let _ = CloseHandle(process.hThread);
                Ok(Self {
                    process: OwnedHandle::from_raw_handle(process.hProcess.0),
                    lines: Some(spawn_writer(File::from(write))),
                    started: Instant::now(),
                    sent: None,
                })
            }
        })();
        // SAFETY: freed once, after the process was created or not.
        unsafe {
            if !list.0.is_null() {
                DeleteProcThreadAttributeList(list);
            }
            if !environment.is_null() {
                let _ = DestroyEnvironmentBlock(environment);
            }
        }
        // The read end is the tray's now; this copy closes with `read`.
        drop(read);
        result
    }

    /// Queues a line for the tray. A state is skipped when it is the one last sent; a notice
    /// (`always`) never is. The pipe is written on a thread of its own, so a tray that does not read
    /// cannot block the launcher; when its queue is full the line is dropped. A tray that has gone is
    /// noticed on the next round.
    pub fn send(&mut self, line: &str, always: bool) {
        if !always && self.sent.as_deref() == Some(line) {
            return;
        }
        let Some(lines) = self.lines.as_ref() else {
            return;
        };
        match lines.try_send(format!(
            "{line}
"
        )) {
            Ok(()) => {
                if !always {
                    self.sent = Some(line.to_owned());
                }
            }
            Err(TrySendError::Full(_)) => {
                tracing::debug!("A tray icon is not reading its pipe; a line was dropped");
            }
            Err(TrySendError::Disconnected(_)) => self.lines = None,
        }
    }

    /// Closes the pipe, which tells the tray to remove its icon and exit. Lines already queued are
    /// written first.
    pub fn close_pipe(&mut self) {
        self.lines = None;
    }

    pub fn has_exited(&self) -> bool {
        // SAFETY: the handle is ours.
        unsafe { WaitForSingleObject(HANDLE(self.process.as_raw_handle()), 0) != WAIT_TIMEOUT }
    }

    /// Waits up to `timeout` for the tray to end, and returns its exit code if it did.
    pub fn wait(&self, timeout: Duration) -> Option<u32> {
        let handle = HANDLE(self.process.as_raw_handle());
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let mut code = 0u32;
        // SAFETY: the handle is ours.
        unsafe {
            (WaitForSingleObject(handle, ms) == WAIT_OBJECT_0
                && GetExitCodeProcess(handle, &mut code).is_ok())
            .then_some(code)
        }
    }

    fn kill(&self) {
        // SAFETY: the handle is ours.
        let _ = unsafe { TerminateProcess(HANDLE(self.process.as_raw_handle()), 1) };
    }
}

/// Starts the thread that writes queued lines to a tray's pipe, and ends, closing the pipe, when the
/// last sender is dropped or a write fails (the tray has gone).
fn spawn_writer(mut pipe: File) -> SyncSender<String> {
    let (lines, queue) = mpsc::sync_channel::<String>(QUEUE);
    let spawned = std::thread::Builder::new()
        .name("Koppel tray pipe".into())
        .spawn(move || {
            for line in queue {
                if pipe.write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("Could not start the thread that writes to a tray icon: {e}");
    }
    lines
}

/// An anonymous pipe: the read end inheritable, for the tray, the write end not.
fn pipe() -> windows::core::Result<(OwnedHandle, OwnedHandle)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    let inheritable = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    // SAFETY: the out parameters are ours; both handles are owned from here.
    unsafe {
        CreatePipe(&mut read, &mut write, Some(&inheritable), PIPE_BUFFER)?;
        let read = OwnedHandle::from_raw_handle(read.0);
        let write = OwnedHandle::from_raw_handle(write.0);
        SetHandleInformation(
            HANDLE(write.as_raw_handle()),
            HANDLE_FLAG_INHERIT.0,
            Default::default(),
        )?;
        Ok((read, write))
    }
}

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(Some(0)).collect()
}

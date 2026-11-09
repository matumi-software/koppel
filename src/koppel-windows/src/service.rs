//! Koppel.exe as the Windows service, as 0.1.x's `KoppelServiceLifetime` and `ServiceRestart`.
//!
//! The installer registers the service under LocalSystem with recovery: a restart after each
//! failure, and a stop with a non-zero exit code counts as one. [`dispatch`] connects to the service
//! manager when it started the program, and the program runs as it does on a console, stopped by the
//! service manager instead of Ctrl+C. It stops cleanly, with exit code 0, so recovery leaves it be;
//! a start that fails ends with exit code 1, which recovery acts on.
//!
//! The tray icon sends two user-defined controls, which interactive users may send though they may
//! not stop or start the service: [`SHUT_DOWN_CONTROL`] stops the service the same clean way, and
//! [`RESTART_CONTROL`] first starts `Koppel.exe --restart-service` ([`restart_service`]), which runs
//! on as LocalSystem, waits for the service to stop and starts it again. If that helper does not
//! start, the service logs it and keeps running. Each reason goes to the event log.

use std::ffi::OsString;
use std::net::IpAddr;
use std::os::windows::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use koppel_web::App;
use koppel_web::tray::TrayFeed;
use tokio::sync::watch;
use windows::Win32::Foundation::ERROR_FAILED_SERVICE_CONTROLLER_CONNECT;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState,
    ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::launcher::{self, Trays};
use crate::shell::{describe, wait_for};
use crate::{RESTART_CONTROL, SERVICE_NAME, SHUT_DOWN_CONTROL, ui_url};

/// What the service starts as it stops for the tray's Restart Koppel.
pub const RESTART_ARGUMENT: &str = "--restart-service";

/// How long the service manager is told a start or a stop may take.
const PENDING_HINT: Duration = Duration::from_secs(30);
/// How long the restart helper waits for the service to stop, and then to run again.
const STOP_WAIT: Duration = Duration::from_secs(120);
const START_WAIT: Duration = Duration::from_secs(60);

/// What the service runs: the program, given the service to report to. Its result is the exit code.
type Run = Box<dyn FnOnce(Service) -> u32 + Send>;

/// The run for `service_main`, which the service manager calls with no way to pass it along.
static RUN: Mutex<Option<Run>> = Mutex::new(None);
/// The exit code `service_main` ended with, for the process.
static EXIT: AtomicU32 = AtomicU32::new(0);

/// Runs `run` as the Windows service when the service manager started this program, and returns the
/// exit code once the service has stopped. `None` when it was started some other way, such as from
/// a console: run it as usual then.
pub fn dispatch(run: impl FnOnce(Service) -> u32 + Send + 'static) -> Option<ExitCode> {
    *RUN.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(run));
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Some(ExitCode::from(
            u8::try_from(EXIT.load(Ordering::SeqCst)).unwrap_or(1),
        )),
        Err(windows_service::Error::Winapi(e))
            if e.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT.0 as i32) =>
        {
            RUN.lock().unwrap_or_else(PoisonError::into_inner).take();
            None
        }
        Err(e) => {
            eprintln!("Could not connect to the service manager: {}", describe(e));
            Some(ExitCode::FAILURE)
        }
    }
}

define_windows_service!(ffi_service_main, service_main);

/// Why the service stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Running,
    Stop,
}

/// The service as the program sees it: when to stop, and where to say it has started.
#[derive(Clone)]
pub struct Service {
    status: Status,
    stop: watch::Receiver<Reason>,
}

impl Service {
    /// Ends when the service manager or the tray asked the service to stop.
    pub async fn stop_requested(&self) {
        let mut stop = self.stop.clone();
        let _ = stop.wait_for(|reason| *reason == Reason::Stop).await;
    }

    /// Says the service runs, publishes the UI's address for the Start menu entry, and starts the
    /// tray icons. Call it once the ports listen; close the trays when stopping.
    pub fn started(&self, app: &Arc<App>, address: Option<IpAddr>, ui_port: u16) -> Trays {
        self.status.report(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            ServiceExitCode::NO_ERROR,
        );
        let url = ui_url::for_listener(address, ui_port);
        ui_url::publish(&url);
        tracing::info!(
            "Koppel {} started. The UI is at {url}",
            env!("CARGO_PKG_VERSION")
        );
        launcher::start(url, TrayFeed::new(app))
    }
}

/// The handle the service reports its state through. Reporting may be done from any thread.
#[derive(Clone, Copy)]
struct Status(ServiceStatusHandle);

// SAFETY: SetServiceStatus may be called with the handle from any thread.
unsafe impl Send for Status {}
// SAFETY: as above.
unsafe impl Sync for Status {}

impl Status {
    fn report(&self, state: ServiceState, accept: ServiceControlAccept, exit: ServiceExitCode) {
        let pending = matches!(
            state,
            ServiceState::StartPending | ServiceState::StopPending
        );
        let _ = self.0.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: exit,
            checkpoint: u32::from(pending),
            wait_hint: if pending {
                PENDING_HINT
            } else {
                Duration::ZERO
            },
            process_id: None,
        });
    }
}

fn service_main(_arguments: Vec<OsString>) {
    let Some(run) = RUN.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return;
    };
    // A service starts in System32. Relative paths in the settings are read from the program's
    // folder, as .NET's service host did.
    if let Some(folder) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(ToOwned::to_owned))
    {
        let _ = std::env::set_current_dir(folder);
    }

    let (stop, stopped) = watch::channel(Reason::Running);
    let status: Arc<OnceLock<Status>> = Arc::new(OnceLock::new());
    let handler = {
        let status = status.clone();
        move |control| {
            let stopping = match control {
                ServiceControl::Interrogate => return ServiceControlHandlerResult::NoError,
                ServiceControl::Stop => {
                    tracing::info!("Stopping: Windows asked the service to stop");
                    true
                }
                ServiceControl::Shutdown => {
                    tracing::info!("Stopping: Windows is shutting down");
                    true
                }
                ServiceControl::UserEvent(code) if code.to_raw() == SHUT_DOWN_CONTROL => {
                    tracing::info!("Shutting down: asked to from the tray icon");
                    true
                }
                ServiceControl::UserEvent(code) if code.to_raw() == RESTART_CONTROL => {
                    // Without the helper the service would only stop, so it stays up then.
                    if begin_restart() {
                        tracing::info!("Restarting: asked to from the tray icon");
                        true
                    } else {
                        tracing::warn!(
                            "Could not restart as the tray icon asked: the helper that starts Koppel again did not start. Koppel keeps running."
                        );
                        false
                    }
                }
                _ => return ServiceControlHandlerResult::NotImplemented,
            };
            if stopping && *stop.borrow() != Reason::Stop {
                if let Some(status) = status.get() {
                    status.report(
                        ServiceState::StopPending,
                        ServiceControlAccept::empty(),
                        ServiceExitCode::NO_ERROR,
                    );
                }
                let _ = stop.send(Reason::Stop);
            }
            ServiceControlHandlerResult::NoError
        }
    };
    let handle = match service_control_handler::register(SERVICE_NAME, handler) {
        Ok(handle) => Status(handle),
        Err(e) => {
            eprintln!(
                "Could not register with the service manager: {}",
                describe(e)
            );
            EXIT.store(1, Ordering::SeqCst);
            return;
        }
    };
    let _ = status.set(handle);
    handle.report(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::NO_ERROR,
    );

    let code = run(Service {
        status: handle,
        stop: stopped,
    });
    if code == 0 {
        tracing::info!("Koppel stopped");
    }
    // Stored before the stop is reported: the dispatcher returns, and the process ends, once it is.
    EXIT.store(code, Ordering::SeqCst);
    handle.report(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(code),
    );
}

/// Starts the helper that brings the service back. False when it could not be started.
fn begin_restart() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut command = Command::new(&exe);
    command
        .arg(RESTART_ARGUMENT)
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(folder) = exe.parent() {
        command.current_dir(folder);
    }
    command.spawn().is_ok()
}

/// `Koppel.exe --restart-service`: waits for the service to stop, then starts it. 0 when it runs
/// again. Nobody sees this process: the service logged that it was restarting, and if it does not
/// come back it starts with Windows, or from Koppel in the Start menu.
pub fn restart_service() -> ExitCode {
    match restart() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn restart() -> Result<(), String> {
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
    wait_for(&state, ServiceState::Stopped, STOP_WAIT)?;
    service.start::<&str>(&[]).map_err(describe)?;
    wait_for(&state, ServiceState::Running, START_WAIT)
}

//! Koppel: USB devices shared over the network.
//!
//! Reads its settings (`config.rs`: options and `KOPPEL_` variables, with 0.1.x's names as aliases), converts a 0.1.x install's secret files once, opens the config
//! files, and serves the UI with its sign-in, REST API and live push over HTTPS on the UI port (and
//! the network port, when `--network-port` sets one apart), with the server certificate kept in
//! `secrets.json`. The server and client roles run on Skakel, with the USB layer behind them.
//!
//! `koppel reset-password` clears the UI password and ends every session, then exits.
//! `koppel healthcheck` asks a running Koppel's `/healthz` and exits 0 when it answers, 1 when it
//! does not: the image's health check.
//!
//! On Windows, when the service manager starts it, it runs as the Koppel service, with the tray
//! icons and the event log (`koppel-windows`).

mod config;
mod diagnostics;
mod healthcheck;
mod systemd;
mod usb;

use std::error::Error;
use std::future::Future;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

use config::{Command, Loaded, Settings};
use koppel_config::{KeySource, SecretBox};
use koppel_web::{App, AppOptions, Host, NetworkOptions};
use time::OffsetDateTime;
use tokio::sync::watch;
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The Windows service, when Koppel runs as one. There is none on other systems.
#[cfg(windows)]
use koppel_windows::service::Service;
#[cfg(not(windows))]
enum Service {}

/// What keeps the service's log file flowing; nothing on the systems that have no such file.
#[cfg(windows)]
type LogGuard = koppel_windows::log_file::LogGuard;
#[cfg(not(windows))]
type LogGuard = ();

/// The UI's port unless `--ui-port` says otherwise.
const UI_PORT: u16 = 5240;

fn main() -> ExitCode {
    // What the service starts as it stops for the tray's Restart Koppel.
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some(koppel_windows::service::RESTART_ARGUMENT) {
        return koppel_windows::service::restart_service();
    }
    let mut loaded = Loaded::read();
    match loaded.command {
        // A one-off for the image's health check: ask the UI port whether Koppel answers, and exit.
        // It stays quiet about how the settings were spelt, as it runs every 30 seconds.
        Command::Healthcheck => healthcheck::run(&loaded.settings),
        Command::Diagnostics { ref file } => diagnostics::run(&loaded.settings, file.as_deref()),
        Command::ResetPassword => {
            let _log = init_logging(false);
            loaded.log_notes();
            finish(block_on(reset_password(&loaded.settings)))
        }
        Command::Serve => {
            #[cfg(windows)]
            {
                let for_service = loaded.clone();
                let as_service = koppel_windows::service::dispatch(move |service| {
                    let mut loaded = for_service;
                    // Held until the service ends, which flushes the log file.
                    let _log = init_logging(true);
                    loaded.log_notes();
                    u32::from(
                        finish(block_on(run(&loaded.settings, Some(service)))) != ExitCode::SUCCESS,
                    )
                });
                if let Some(code) = as_service {
                    return code;
                }
            }
            let _log = init_logging(false);
            loaded.log_notes();
            finish(block_on(run(&loaded.settings, None)))
        }
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the async runtime starts")
        .block_on(future)
}

fn finish(result: Result<(), Box<dyn Error>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(settings: &Settings, service: Option<Service>) -> Result<(), Box<dyn Error>> {
    let ui_port = settings.ui_port();
    let network_port = settings.network_port();
    let address = settings.listen_address();
    let reach = settings.reach();
    let setup_window = settings.setup_window();
    let auth_per_minute = settings.auth_per_minute();
    let network = NetworkOptions {
        address,
        discovery: settings.discovery_enabled(),
        discovery_port: settings.discovery_port(),
        probe_targets: None,
        token_per_minute: settings.token_per_minute(),
        pair_per_minute: settings.pair_per_minute(),
    };
    let usb = usb::options(settings)?;

    let secrets = open_files(settings)?;
    let paths = settings.paths(&std::env::current_dir()?);
    let host = Host {
        name: koppel_web::machine_name(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        started_at: OffsetDateTime::now_utc(),
        ui_port,
        network_port,
        tray: service.is_some(),
        os: koppel_web::os_label(),
    };
    let app = App::start(AppOptions {
        paths,
        secrets,
        host,
        reach,
        setup_window,
        auth_per_minute,
        network,
        usb,
    })
    .await?;
    let acceptor = app.tls();

    // One listener for both surfaces when they share a port, otherwise one for each, each with its
    // own router.
    let routers = app.routers();
    let health = routers.ui.clone();
    let listeners = if network_port == ui_port {
        vec![(ui_port, routers.merged())]
    } else {
        vec![(ui_port, routers.ui), (network_port, routers.network)]
    };
    let (stop, stopped) = watch::channel(false);
    let mut servers = tokio::task::JoinSet::new();
    for (port, router) in listeners {
        let listener = koppel_web::bind(address, port).map_err(|e| {
            format!(
                "could not listen on port {port}{}: {e}",
                address.map(|a| format!(" at {a}")).unwrap_or_default()
            )
        })?;
        tracing::info!(
            "Koppel {} is listening on https://{}",
            env!("CARGO_PKG_VERSION"),
            listener.local_addr()?
        );
        let mut stopped = stopped.clone();
        servers.spawn(koppel_web::serve(
            listener,
            acceptor.clone(),
            router,
            async move {
                let _ = stopped.changed().await;
            },
        ));
    }
    systemd::ready();
    tokio::spawn(systemd::heartbeat(health));

    #[cfg(windows)]
    let trays = service
        .as_ref()
        .map(|service| service.started(&app, address, ui_port));

    #[cfg(windows)]
    if let Some(service) = &service {
        service.stop_requested().await;
    } else {
        stop_signal().await;
    }
    #[cfg(not(windows))]
    stop_signal().await;

    #[cfg(windows)]
    if let Some(trays) = trays {
        trays.close().await;
    }
    systemd::stopping();
    let _ = stop.send(true);
    while servers.join_next().await.is_some() {}
    app.stop().await;
    tracing::info!("Koppel stopped");
    Ok(())
}

/// Prepares the data folders, opens what seals the secret files, and upgrades the files of a 0.1.x
/// or 0.2.0 install, once (`koppel-upgrade`). A failed upgrade is logged and leaves the files as they
/// were; the start then stops on the first one still in an old format.
fn open_files(settings: &Settings) -> Result<Arc<SecretBox>, Box<dyn Error>> {
    let paths = settings.paths(&std::env::current_dir()?);
    paths.prepare().map_err(|e| {
        format!(
            "could not prepare {} and {}: {e}",
            paths.config.display(),
            paths.keys.display()
        )
    })?;
    let secrets = Arc::new(SecretBox::open(&paths.keys)?);
    let source = secrets.source();
    if *source == KeySource::KeysDirectory {
        tracing::info!(
            "The secret files are sealed with the {source}, beside the files it protects: see \"Threat model\" in docs/security.md for a key kept elsewhere"
        );
    } else {
        tracing::info!("The secret files are sealed with {source}");
    }
    // The upgrade logs what it did and why it could not; a start after a failure stops on the first
    // file it could not read.
    let _ = koppel_upgrade::upgrade(&paths, &secrets);
    // After a rotation, or a move of the key out of the keys directory, files sealed with an older
    // key are written again with the current one.
    let mut sealed = paths.secret_files().to_vec();
    sealed.push(paths.keys.join("ui-session.key"));
    secrets.reseal(&sealed);
    Ok(secrets)
}

/// `koppel reset-password`, as 0.1.x's `UiPasswordReset`.
async fn reset_password(settings: &Settings) -> Result<(), Box<dyn Error>> {
    let setup_window = settings.setup_window();
    let done = match open_files(settings) {
        Ok(secrets) => {
            let paths = settings.paths(&std::env::current_dir()?);
            koppel_web::reset_password(&paths, &secrets)
                .await
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = done {
        println!("Could not reset the password: {e}");
        println!(
            "Run this as an administrator (Windows), as root (Linux package: sudo koppel reset-password) or as the user Koppel runs as (Docker: docker compose exec koppel koppel reset-password)."
        );
        return Err("the password was not reset".into());
    }
    println!("The UI password is cleared and every signed-in session is ended.");
    println!(
        "Open the UI and set a new password within {} minutes: from now if Koppel is running, otherwise from when it starts. Whoever opens the UI first sets it.",
        setup_window.as_secs() / 60
    );
    Ok(())
}

/// Logs to standard output at Info, or as RUST_LOG says (`debug`, or `warn,koppel_web=debug`), in
/// colour only on a terminal, so a service log or `docker logs` gets plain text. The Windows service
/// also logs to the event log (`koppel_windows::event_log`) and to a rolling file under
/// `%ProgramData%\Koppel\logs` (`koppel_windows::log_file`), at the same levels as the console.
/// The result keeps the file's writer thread alive: hold it until the program ends.
fn init_logging(service: bool) -> Option<LogGuard> {
    let asked = std::env::var("RUST_LOG").ok();
    let parsed = asked.as_deref().map(str::parse::<Targets>);
    let filter = match &parsed {
        Some(Ok(targets)) => targets.clone(),
        _ => Targets::new().with_default(Level::INFO),
    };
    let registry = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(std::io::stdout().is_terminal())
            .with_filter(filter.clone()),
    );
    #[cfg(windows)]
    let registry = registry.with(service.then(koppel_windows::event_log::layer).flatten());
    #[cfg(windows)]
    let (registry, guard, file_error) = {
        let file = service.then(|| {
            koppel_config::prepare_log_folder()
                .and_then(|folder| koppel_windows::log_file::start(&folder, filter.clone()))
        });
        match file {
            Some(Ok((layer, guard))) => (registry.with(Some(layer)), Some(guard), None),
            Some(Err(e)) => (registry.with(None), None, Some(e)),
            None => (registry.with(None), None, None),
        }
    };
    #[cfg(not(windows))]
    let (guard, file_error): (Option<LogGuard>, Option<std::io::Error>) = {
        let _ = service;
        (None, None)
    };
    registry.init();
    if let Some(Err(e)) = parsed {
        tracing::warn!("RUST_LOG is not understood ({e}), so logging stays at Info");
    }
    if let Some(e) = file_error {
        tracing::warn!(
            "The log file could not be opened, so the service logs only to the event log: {e}"
        );
    }
    guard
}

/// Ctrl+C, and on Linux also SIGTERM, which is how a container or systemd stops a program.
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(e) => {
                tracing::warn!("SIGTERM cannot be caught ({e}), only Ctrl+C stops Koppel");
                ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    ctrl_c().await;
}

async fn ctrl_c() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::warn!("Ctrl+C cannot be caught: {e}");
        std::future::pending::<()>().await;
    }
}

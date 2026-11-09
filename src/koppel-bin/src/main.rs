//! Koppel: USB devices shared over the network.
//!
//! Reads its settings as 0.1.x did, converts a 0.1.x install's secret files once, opens the config
//! files, and serves the UI with its sign-in, REST API and live push over HTTPS on the UI port (and
//! the network port, when `Ports:Network` sets one apart), with the server certificate kept in
//! `secrets.json`. The roles and USB come with the issues of milestone v0.2.0 (#85 to #87).
//!
//! `koppel --reset-password` clears the UI password and ends every session, then exits.

mod config;

use std::error::Error;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

use config::Config;
use koppel_config::{SecretBox, convert_0_1_secrets};
use koppel_web::{App, AppOptions, Host, tls};
use time::OffsetDateTime;
use tokio::sync::watch;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The UI's port unless `Ports:Ui` says otherwise.
const UI_PORT: u16 = 5240;
/// Setup, login and password change a minute per address, unless `RateLimits:AuthPerMinute` says
/// otherwise.
const AUTH_PER_MINUTE: u32 = 30;

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config::read(std::env::vars_os(), &args);
    let reset = args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("--reset-password"));
    let result = if reset {
        reset_password(&config).await
    } else {
        run(&config).await
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: &Config) -> Result<(), Box<dyn Error>> {
    let ui_port = config.port("Ports:Ui", UI_PORT)?;
    let network_port = config.port("Ports:Network", ui_port)?;
    let address = config.listen_address()?;
    let reach = config.reach()?;
    let setup_window = config.setup_window()?;
    let auth_per_minute = config.count("RateLimits:AuthPerMinute", AUTH_PER_MINUTE)?;

    let secrets = open_files(config)?;
    let paths = config.paths(&std::env::current_dir()?);
    let host = Host {
        name: koppel_web::machine_name(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        started_at: OffsetDateTime::now_utc(),
        ui_port,
        network_port,
        tray: false,
        os: koppel_web::os_label(),
    };
    let app = App::start(AppOptions {
        paths,
        secrets,
        host,
        reach,
        setup_window,
        auth_per_minute,
    })
    .await?;
    let acceptor = tls::acceptor(app.certificate())?;

    let mut ports = vec![ui_port];
    if network_port != ui_port {
        ports.push(network_port);
    }
    let (stop, stopped) = watch::channel(false);
    let mut servers = tokio::task::JoinSet::new();
    for port in ports {
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
            app.router(),
            async move {
                let _ = stopped.changed().await;
            },
        ));
    }
    stop_signal().await;
    let _ = stop.send(true);
    while servers.join_next().await.is_some() {}
    app.stop();
    tracing::info!("Koppel stopped");
    Ok(())
}

/// Prepares the data folders, opens what seals the secret files, and converts a 0.1.x install's
/// secret files, once. A failed conversion is logged and leaves them as they were; the start then
/// stops on the first one still in the old format.
fn open_files(config: &Config) -> Result<Arc<SecretBox>, Box<dyn Error>> {
    let paths = config.paths(&std::env::current_dir()?);
    if paths.key_certificate.is_some() {
        tracing::warn!(
            "Paths:KeyCertificate is set, but Koppel 0.2 no longer reads it. It was deprecated in 0.1.2"
        );
    }
    paths.prepare().map_err(|e| {
        format!(
            "could not prepare {} and {}: {e}",
            paths.config.display(),
            paths.keys.display()
        )
    })?;
    let secrets = Arc::new(SecretBox::open(&paths.keys)?);
    if let Err(e) = convert_0_1_secrets(&paths, &secrets) {
        tracing::error!("{e}");
    }
    Ok(secrets)
}

/// `koppel --reset-password`, as 0.1.x's `UiPasswordReset`.
async fn reset_password(config: &Config) -> Result<(), Box<dyn Error>> {
    let setup_window = config.setup_window()?;
    let done = match open_files(config) {
        Ok(secrets) => {
            let paths = config.paths(&std::env::current_dir()?);
            koppel_web::reset_password(&paths, &secrets)
                .await
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = done {
        println!("Could not reset the password: {e}");
        println!(
            "Run this as an administrator (Windows), as root (Linux package: sudo koppel --reset-password) or as the user Koppel runs as (Docker: docker compose exec koppel koppel --reset-password)."
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
/// colour only on a terminal, so a service log or `docker logs` gets plain text.
fn init_logging() {
    let asked = std::env::var("RUST_LOG").ok();
    let parsed = asked.as_deref().map(str::parse::<Targets>);
    let filter = match &parsed {
        Some(Ok(targets)) => targets.clone(),
        _ => Targets::new().with_default(Level::INFO),
    };
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_ansi(std::io::stdout().is_terminal()))
        .with(filter)
        .init();
    if let Some(Err(e)) = parsed {
        tracing::warn!("RUST_LOG is not understood ({e}), so logging stays at Info");
    }
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

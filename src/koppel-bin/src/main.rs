//! Koppel: USB devices shared over the network.
//!
//! For now this serves the UI and `/healthz` over HTTPS on port 5240 with a certificate made at start.
//! Config, Koppel's own certificate, sign-in and the roles come with the issues of milestone v0.2.0.

use std::error::Error;
use std::io::IsTerminal;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use koppel_web::{CertificateSlot, tls};
use tokio::net::TcpListener;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The UI's port, `Ports:Ui` in 0.1.x.
const UI_PORT: u16 = 5240;

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let certificate = tls::self_signed(&["localhost"])?;
    let acceptor = tls::acceptor(CertificateSlot::new(certificate.certified))?;
    let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, UI_PORT));
    let listener = TcpListener::bind(address)
        .await
        .map_err(|e| format!("could not listen on {address}: {e}"))?;
    tracing::info!(
        "Koppel {} is listening on https://{address}",
        env!("CARGO_PKG_VERSION")
    );
    koppel_web::serve(listener, acceptor, koppel_web::router(), stop_signal()).await;
    tracing::info!("Koppel stopped");
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

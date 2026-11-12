//! What Koppel tells systemd when it runs as the `koppel` service of the Linux package
//! (`Type=notify`, `WatchdogSec=` in `koppel.service`): `READY=1` once its ports listen, `STOPPING=1`
//! when it stops, and `WATCHDOG=1` at half the interval systemd asks for (`WATCHDOG_USEC`), but only
//! while it still answers its own `/healthz`. A Koppel that hangs stops sending it, and systemd
//! restarts it.
//!
//! Outside systemd (Docker, Windows, a console) `NOTIFY_SOCKET` and `WATCHDOG_USEC` are not set, and
//! none of this does anything.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use tower_service::Service;

/// What Koppel tells systemd.
#[derive(Debug, Clone, Copy)]
enum State {
    Ready,
    Stopping,
    Watchdog,
}

/// Tells systemd that Koppel has started.
pub fn ready() {
    if let Err(e) = notify(State::Ready) {
        tracing::warn!("Could not tell systemd that Koppel is ready: {e}");
    }
}

/// Tells systemd that Koppel is stopping, so a slow stop is not taken for a hang.
pub fn stopping() {
    if let Err(e) = notify(State::Stopping) {
        tracing::warn!("Could not tell systemd that Koppel is stopping: {e}");
    }
}

/// Sends `state` to the socket in `NOTIFY_SOCKET`, and nothing when it is not set.
#[cfg(target_os = "linux")]
fn notify(state: State) -> std::io::Result<()> {
    use sd_notify::NotifyState;
    sd_notify::notify(&[match state {
        State::Ready => NotifyState::Ready,
        State::Stopping => NotifyState::Stopping,
        State::Watchdog => NotifyState::Watchdog,
    }])
}

#[cfg(not(target_os = "linux"))]
fn notify(_: State) -> std::io::Result<()> {
    Ok(())
}

/// The interval systemd's watchdog asks for, `None` when it is off or not watching this process.
#[cfg(target_os = "linux")]
fn watchdog_interval() -> Option<Duration> {
    sd_notify::watchdog_enabled().filter(|interval| !interval.is_zero())
}

#[cfg(not(target_os = "linux"))]
fn watchdog_interval() -> Option<Duration> {
    None
}

/// Sends systemd's watchdog its heartbeat for as long as Koppel runs, while `router` answers
/// `/healthz`. Returns at once when the watchdog is off.
pub async fn heartbeat(router: Router) {
    let Some(interval) = watchdog_interval() else {
        return;
    };
    tracing::info!(
        "systemd's watchdog restarts Koppel if it stops answering for {} s",
        interval.as_secs()
    );
    beat(interval, healthz(router), || notify(State::Watchdog)).await;
}

/// Asks the router for `/healthz` in process, as a request from this machine: the router refuses a
/// request that has no caller address, so the probe gives it the loopback one the accept loop would.
/// That goes through the guards and their settings, on the same runtime as every connection, so a runtime with every worker
/// stuck or a lock that never comes free fails it.
fn healthz(router: Router) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = bool> + Send>> {
    move || {
        let mut router = router.clone();
        Box::pin(async move {
            let mut request = Request::get("/healthz")
                .body(Body::empty())
                .expect("a GET of a fixed path is a valid request");
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
            matches!(router.call(request).await, Ok(response) if response.status().is_success())
        })
    }
}

/// The heartbeat itself: every half `interval`, runs `probe` and calls `send` when it says healthy
/// within a quarter `interval`. One late or failed probe costs one beat; systemd acts only after a
/// whole `interval` without one, so it takes two in a row.
async fn beat<P, F, S, E>(interval: Duration, mut probe: P, mut send: S)
where
    P: FnMut() -> F,
    F: Future<Output = bool>,
    S: FnMut() -> Result<(), E>,
    E: std::fmt::Display,
{
    let period = interval / 2;
    let deadline = interval / 4;
    let mut ticks = tokio::time::interval(period);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut failing = false;
    let mut send_failed = false;
    loop {
        ticks.tick().await;
        let healthy = tokio::time::timeout(deadline, probe()).await;
        if healthy == Ok(true) {
            if failing {
                tracing::info!("Koppel answers its health check again");
                failing = false;
            }
            match send() {
                Ok(()) => send_failed = false,
                Err(e) if !send_failed => {
                    tracing::warn!("Could not send systemd's watchdog its heartbeat: {e}");
                    send_failed = true;
                }
                Err(_) => {}
            }
        } else if !failing {
            let why = if healthy.is_err() {
                "did not answer in time"
            } else {
                "failed"
            };
            tracing::warn!(
                "Koppel's health check {why}, so systemd's watchdog gets no heartbeat until it passes"
            );
            failing = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use axum::routing::get;

    use super::*;

    const INTERVAL: Duration = Duration::from_secs(20);

    /// Runs `beat` for `elapsed` of paused time and counts the heartbeats.
    async fn beats_in<P, F>(elapsed: Duration, probe: P) -> usize
    where
        P: FnMut() -> F + Send + 'static,
        F: Future<Output = bool> + Send + 'static,
    {
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let task = tokio::spawn(beat(INTERVAL, probe, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<(), String>(())
        }));
        tokio::time::sleep(elapsed).await;
        task.abort();
        sent.load(Ordering::SeqCst)
    }

    #[tokio::test(start_paused = true)]
    async fn beat_sends_at_half_the_interval_while_healthy() {
        // Ticks at 0, 10, 20, 30 and 40 s; the sleep ends just after the one at 40.
        let sent = beats_in(INTERVAL * 2 + Duration::from_millis(1), || async { true }).await;
        assert_eq!(sent, 5);
    }

    #[tokio::test(start_paused = true)]
    async fn beat_sends_nothing_while_the_probe_fails() {
        let sent = beats_in(INTERVAL * 3, || async { false }).await;
        assert_eq!(sent, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn beat_sends_nothing_while_the_probe_hangs() {
        let sent = beats_in(INTERVAL * 3, std::future::pending::<bool>).await;
        assert_eq!(sent, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn beat_starts_again_when_the_probe_passes_again() {
        let healthy = Arc::new(AtomicBool::new(false));
        let flag = healthy.clone();
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let task = tokio::spawn(beat(
            INTERVAL,
            move || {
                let healthy = flag.load(Ordering::SeqCst);
                async move { healthy }
            },
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<(), String>(())
            },
        ));
        tokio::time::sleep(INTERVAL * 2).await;
        assert_eq!(sent.load(Ordering::SeqCst), 0);
        healthy.store(true, Ordering::SeqCst);
        tokio::time::sleep(INTERVAL).await;
        task.abort();
        assert!(sent.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test(start_paused = true)]
    async fn beat_keeps_going_when_sending_fails() {
        let tries = Arc::new(AtomicUsize::new(0));
        let counter = tries.clone();
        let task = tokio::spawn(beat(
            INTERVAL,
            || async { true },
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Err("no socket")
            },
        ));
        tokio::time::sleep(INTERVAL * 2 + Duration::from_millis(1)).await;
        task.abort();
        assert_eq!(tries.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn healthz_passes_when_the_route_answers() {
        let mut probe = healthz(Router::new().route("/healthz", get(|| async { "ok" })));
        assert!(probe().await);
    }

    #[tokio::test]
    async fn the_probe_comes_from_the_loopback_address() {
        // The router refuses a request that has no caller address, so the probe has to give one.
        let router = Router::new().route(
            "/healthz",
            get(|ConnectInfo(from): ConnectInfo<SocketAddr>| async move {
                if from.ip().is_loopback() {
                    axum::http::StatusCode::OK
                } else {
                    axum::http::StatusCode::FORBIDDEN
                }
            }),
        );
        let mut probe = healthz(router);
        assert!(probe().await);
    }

    #[tokio::test]
    async fn healthz_fails_when_the_route_does_not() {
        let mut probe = healthz(Router::new());
        assert!(!probe().await);
    }

    #[test]
    fn watchdog_is_off_outside_systemd() {
        // The test runner is not a systemd service with a watchdog.
        if std::env::var_os("WATCHDOG_USEC").is_none() {
            assert_eq!(watchdog_interval(), None);
        }
    }
}

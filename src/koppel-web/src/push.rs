//! The live push: what the server sends the UI after it wrote a change, as Server-Sent Events.
//!
//! The UI opens `GET /api/v1/events` with its session cookie and reads one event per change: the event's
//! name is what 0.1.x's SignalR hub called (`IUiClient`), its data the same JSON payload. Each is sent
//! only once the change is on disk, so the UI never shows what was not saved. A stream that falls
//! behind is closed rather than skipping events, and the UI fetches everything again when it opens a
//! new one. A comment goes out every 15 seconds, so a dead connection is noticed and proxies keep it
//! open. The stream ends when its session does (the session's token is cancelled): signing out, a
//! password change or reset elsewhere, the session running out, or the server stopping.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{self, KeepAlive, Sse};
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use koppel_config::files::KoppelSettings;
use koppel_usb::{ClientDevices, ServerDevices};

use crate::activity::ActivityEvent;
use crate::api::client::ConnectionView;
use crate::auth::SignedIn;
use crate::roles::{ClientView, DiscoveredServer, KeysView, PairingOverview, PairingStatus};
use crate::server::Shutdown;
use crate::state::PushApi;

/// One push, with the payload its name goes with. The name is the SSE event name the UI listens for,
/// the payload is the event's data as JSON and nothing else (no name inside it), so each name has
/// one payload type and a call cannot send a payload under the wrong name.
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum Event<'a> {
    /// `settings.json` was written: the settings.
    SettingsChanged(&'a KoppelSettings),
    /// Server: the registered clients changed.
    ClientsChanged(&'a [ClientView]),
    /// Server: the certificate or the prepared next certificate changed.
    ServerKeysChanged(&'a KeysView),
    /// Client: `connection.json` was written: the connection without its secret.
    ConnectionChanged(&'a ConnectionView),
    /// Server: this machine's devices changed.
    DevicesChanged(&'a ServerDevices),
    /// Client: the link, the server's devices or what is attached here changed.
    ClientDevicesChanged(&'a ClientDevices),
    /// Something went into the activity log: that one entry.
    ActivityRecorded(&'a ActivityEvent),
    /// Client: the servers found on the network changed.
    ServersDiscovered(&'a [DiscoveredServer]),
    /// Client: pairing moved on.
    PairingStatusChanged(&'a PairingStatus),
    /// Server: the pairing window or the pairing requests changed.
    PairingRequestsChanged(&'a PairingOverview),
}

impl Event<'_> {
    /// The name the event goes out under, as 0.1.x's SignalR hub called it.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::SettingsChanged(_) => "SettingsChanged",
            Self::ClientsChanged(_) => "ClientsChanged",
            Self::ServerKeysChanged(_) => "ServerKeysChanged",
            Self::ConnectionChanged(_) => "ConnectionChanged",
            Self::DevicesChanged(_) => "DevicesChanged",
            Self::ClientDevicesChanged(_) => "ClientDevicesChanged",
            Self::ActivityRecorded(_) => "ActivityRecorded",
            Self::ServersDiscovered(_) => "ServersDiscovered",
            Self::PairingStatusChanged(_) => "PairingStatusChanged",
            Self::PairingRequestsChanged(_) => "PairingRequestsChanged",
        }
    }
}

/// How often a quiet stream gets a comment.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Events a stream may fall behind by before it is closed. A UI reads them as they come, so only a
/// stalled connection gets this far.
const BACKLOG: usize = 256;

/// One event, its JSON written once for every stream.
#[derive(Debug)]
pub struct Pushed {
    pub name: &'static str,
    pub data: String,
}

/// Sends events to every open stream. Cheap to clone.
#[derive(Clone)]
pub struct Push {
    events: broadcast::Sender<Arc<Pushed>>,
}

impl Default for Push {
    fn default() -> Self {
        Self::new()
    }
}

impl Push {
    pub fn new() -> Self {
        Self {
            events: broadcast::channel(BACKLOG).0,
        }
    }

    /// Sends `event` to every open stream. Call it after the change is written. Nothing is written
    /// as JSON while no stream is open.
    pub(crate) fn send(&self, event: Event<'_>) {
        if self.events.receiver_count() == 0 {
            return;
        }
        let name = event.name();
        match serde_json::to_string(&event) {
            Ok(data) => {
                let _ = self.events.send(Arc::new(Pushed { name, data }));
            }
            Err(e) => tracing::error!("the push {name} could not be written as JSON: {e}"),
        }
    }

    /// A receiver for what is sent from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Pushed>> {
        self.events.subscribe()
    }
}

/// `GET /api/v1/events`: the stream for one signed-in session.
pub(crate) async fn events(
    State(app): State<PushApi>,
    Extension(session): Extension<SignedIn>,
    shutdown: Option<Extension<Shutdown>>,
) -> impl IntoResponse {
    let live = Live {
        events: app.push.subscribe(),
        ended: session.ended,
        // A request that did not come through the accept loop has none, and nothing ends it.
        shutdown: shutdown.map_or_else(CancellationToken::new, |Extension(Shutdown(token))| token),
        until: Instant::now() + session.valid_for,
    };
    let stream = futures_util::stream::unfold(Some(live), |state| async move {
        let mut live = state?;
        let event = live.next().await?;
        Some((Ok::<_, Infallible>(event), Some(live)))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
}

/// One open stream.
struct Live {
    events: broadcast::Receiver<Arc<Pushed>>,
    /// Cancelled when the session ends.
    ended: CancellationToken,
    /// Cancelled when the server stops, so the stream does not hold the stop up.
    shutdown: CancellationToken,
    until: Instant,
}

impl Live {
    /// The next event, or `None` when the stream is over.
    async fn next(&mut self) -> Option<sse::Event> {
        tokio::select! {
            pushed = self.events.recv() => match pushed {
                Ok(pushed) => Some(sse::Event::default().event(pushed.name).data(&pushed.data)),
                // Behind or shutting down: the UI opens a new stream and refetches.
                Err(RecvError::Lagged(_) | RecvError::Closed) => None,
            },
            () = self.ended.cancelled() => None,
            () = self.shutdown.cancelled() => None,
            () = tokio::time::sleep_until(self.until) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(push: &Push) -> (Live, CancellationToken, CancellationToken) {
        let (ended, shutdown) = (CancellationToken::new(), CancellationToken::new());
        let live = Live {
            events: push.subscribe(),
            ended: ended.clone(),
            shutdown: shutdown.clone(),
            until: Instant::now() + Duration::from_secs(12 * 3600),
        };
        (live, ended, shutdown)
    }

    /// With the clock paused, a stream that is not ended waits the 12 hours out in no time, so the
    /// elapsed time says whether the stream ended by itself or by running out.
    #[tokio::test(start_paused = true)]
    async fn the_server_stopping_ends_a_quiet_stream_at_once() {
        let push = Push::new();
        let (mut stream, _ended, shutdown) = live(&push);
        let started = Instant::now();
        let waiting = tokio::spawn(async move { stream.next().await.is_none() });
        tokio::task::yield_now().await;
        shutdown.cancel();
        assert!(waiting.await.unwrap(), "the stream is over");
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_carries_events_until_its_session_ends() {
        let push = Push::new();
        let (mut stream, ended, _shutdown) = live(&push);
        push.send(Event::SettingsChanged(&KoppelSettings::default()));
        assert!(stream.next().await.is_some());
        let started = Instant::now();
        ended.cancel();
        assert!(stream.next().await.is_none());
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_with_nothing_to_end_it_runs_out_with_its_session() {
        let push = Push::new();
        let (mut stream, _ended, _shutdown) = live(&push);
        let started = Instant::now();
        assert!(stream.next().await.is_none());
        assert_eq!(started.elapsed(), Duration::from_secs(12 * 3600));
    }
}

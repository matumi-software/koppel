//! The live push: what the server sends the UI after it wrote a change, as Server-Sent Events.
//!
//! The UI opens `GET /api/events` with its session cookie and reads one event per change: the event's
//! name is what 0.1.x's SignalR hub called (`IUiClient`), its data the same JSON payload. Each is sent
//! only once the change is on disk, so the UI never shows what was not saved. A stream that falls
//! behind is closed rather than skipping events, and the UI fetches everything again when it opens a
//! new one. A comment goes out every 15 seconds, so a dead connection is noticed and proxies keep it
//! open. The stream ends when its session does: signing out, a password change or reset elsewhere,
//! or the session's cookie running out.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;

use crate::app::App;
use crate::auth::{SessionEnd, SignedIn};

/// `settings.json` was written: the settings.
pub const SETTINGS_CHANGED: &str = "SettingsChanged";
/// Server: the registered clients changed.
pub const CLIENTS_CHANGED: &str = "ClientsChanged";
/// Server: the certificate or the prepared next certificate changed.
pub const SERVER_KEYS_CHANGED: &str = "ServerKeysChanged";
/// Client: `connection.json` was written: the connection without its secret.
pub const CONNECTION_CHANGED: &str = "ConnectionChanged";
/// Server: this machine's devices changed.
pub const DEVICES_CHANGED: &str = "DevicesChanged";
/// Client: the link, the server's devices or what is attached here changed.
pub const CLIENT_DEVICES_CHANGED: &str = "ClientDevicesChanged";
/// Something went into the activity log: that one entry.
pub const ACTIVITY_RECORDED: &str = "ActivityRecorded";
/// Client: the servers found on the network changed.
pub const SERVERS_DISCOVERED: &str = "ServersDiscovered";
/// Client: pairing moved on.
pub const PAIRING_STATUS_CHANGED: &str = "PairingStatusChanged";
/// Server: the pairing window or the pairing requests changed.
pub const PAIRING_REQUESTS_CHANGED: &str = "PairingRequestsChanged";

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

    /// Sends `value` as the event `name` to every open stream. Call it after the change is written.
    pub fn send<T: Serialize + ?Sized>(&self, name: &'static str, value: &T) {
        match serde_json::to_string(value) {
            Ok(data) => {
                // No stream open is fine: nobody is looking.
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

/// `GET /api/events`: the stream for one signed-in session.
pub(crate) async fn events(
    State(app): State<Arc<App>>,
    Extension(session): Extension<SignedIn>,
) -> impl IntoResponse {
    let live = Live {
        events: app.push.subscribe(),
        ends: app.auth.ends(),
        session: session.id,
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
    ends: broadcast::Receiver<SessionEnd>,
    session: String,
    until: Instant,
}

impl Live {
    /// The next event, or `None` when the stream is over.
    async fn next(&mut self) -> Option<Event> {
        loop {
            tokio::select! {
                pushed = self.events.recv() => {
                    return match pushed {
                        Ok(pushed) => Some(Event::default().event(pushed.name).data(&pushed.data)),
                        // Behind or shutting down: the UI opens a new stream and refetches.
                        Err(RecvError::Lagged(_) | RecvError::Closed) => None,
                    };
                }
                end = self.ends.recv() => match end {
                    Ok(end) if !end.ends(&self.session) => {}
                    // Ended, or the stream cannot tell any more whether it was.
                    _ => return None,
                },
                () = tokio::time::sleep_until(self.until) => return None,
            }
        }
    }
}

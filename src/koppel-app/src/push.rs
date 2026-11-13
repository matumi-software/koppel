//! The live push's bus: what a role sends after it wrote a change. The web server reads it as
//! Server-Sent Events at `/api/v1/events` (`koppel-web`'s `push.rs`), the Windows tray reads it to
//! know when to look again.

use std::sync::Arc;

use koppel_config::files::KoppelSettings;
use koppel_usb::{ClientDevices, ServerDevices};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::activity::ActivityEvent;
use crate::connection::ConnectionView;
use crate::roles::{ClientView, DiscoveredServer, KeysView, PairingOverview, PairingStatus};

/// Events a stream may fall behind by before it is closed. A UI reads them as they come, so only a
/// stalled connection gets this far.
pub const BACKLOG: usize = 256;

/// One push, with the payload its name goes with. The name is the SSE event name the UI listens for,
/// the payload is the event's data as JSON and nothing else (no name inside it), so each name has
/// one payload type and a call cannot send a payload under the wrong name.
#[derive(Serialize)]
#[serde(untagged)]
pub enum Event<'a> {
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
    pub fn name(&self) -> &'static str {
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

/// One event, its JSON written once for every stream.
#[derive(Debug)]
pub struct Pushed {
    pub name: &'static str,
    pub data: String,
}

impl Pushed {
    pub fn of(event: &Event<'_>) -> Option<Self> {
        let name = event.name();
        match serde_json::to_string(event) {
            Ok(data) => Some(Self { name, data }),
            Err(e) => {
                tracing::error!("the push {name} could not be written as JSON: {e}");
                None
            }
        }
    }
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
    pub fn send(&self, event: Event<'_>) {
        if self.events.receiver_count() == 0 {
            return;
        }
        if let Some(pushed) = Pushed::of(&event) {
            let _ = self.events.send(Arc::new(pushed));
        }
    }

    /// A receiver for what is sent from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Pushed>> {
        self.events.subscribe()
    }
}

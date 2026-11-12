//! What the roles take from outside and what they hand back to be saved. The USB layer does not
//! know where a setting or a list of devices lives: the program gives it an [`Observed`] value for
//! each setting it follows and a [`Store`] for each list it saves, and keeps the file format on its
//! own side.
//!
//! A change is delivered to the handlers one at a time, in order, and the one who made it waits
//! until the handlers are done. That is how a settings update returns only once the roles have
//! applied it, which is what the UI's confirmation relies on.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use koppel_secrets::Secret;

use crate::BoxFuture;

/// Which side of the link this machine plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    Server,
    Client,
    Both,
}

impl Role {
    /// This machine shares its devices.
    pub fn serves(self) -> bool {
        matches!(self, Self::Server | Self::Both)
    }

    /// This machine links to a server and attaches its devices.
    pub fn links(self) -> bool {
        self != Self::Server
    }
}

/// The settings the USB layer follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbSettings {
    pub role: Role,
    /// Share the devices that were shared before a restart.
    pub share_on_startup: bool,
}

impl Default for UsbSettings {
    fn default() -> Self {
        Self {
            role: Role::Server,
            share_on_startup: true,
        }
    }
}

/// The server a client links to and its credentials, as the USB layer reads them: it compares
/// them and hands them to the transport, and reads nothing else of them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Connection {
    pub server_url: Option<String>,
    /// The certificate pinned for this server.
    pub server_fingerprint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<Secret<String>>,
    /// The certificate the server said it will switch to, trusted as well until then.
    pub next_server_fingerprint: Option<String>,
}

impl Connection {
    /// Every part needed to link is there.
    pub fn is_complete(&self) -> bool {
        self.server_url.is_some()
            && self.server_fingerprint.is_some()
            && self.client_id.is_some()
            && self.client_secret.is_some()
    }
}

/// The bus IDs the user chose to share, with the VID:PID each had.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SharedDevices {
    pub bus_ids: Vec<String>,
    pub vid_pids: IndexMap<String, String>,
}

/// The devices this client attached and the server they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AttachedDevices {
    pub server_fingerprint: Option<String>,
    pub bus_ids: Vec<String>,
    pub vid_pids: IndexMap<String, String>,
}

/// A value could not be loaded or saved. The text is what the UI shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(String);

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

/// Ends a subscription when it is dropped.
pub type Subscription = Box<dyn Any + Send + Sync>;

/// What a handler is called with: the new value. The change is delivered once the handler's
/// future is done.
pub type Handler<T> = Box<dyn Fn(Arc<T>) -> BoxFuture<'static, ()> + Send + Sync>;

/// A value that changes while the program runs.
pub trait Observed<T>: Send + Sync {
    fn current(&self) -> Arc<T>;

    /// Calls `handler` with every change, in order, one at a time. The one who made the change
    /// waits for the handler. Dropping the returned [`Subscription`] ends it.
    fn subscribe(&self, handler: Handler<T>) -> Subscription;
}

/// A value the roles save: loaded when they start, changed through [`Store::update`].
pub trait Store<T>: Send + Sync {
    fn current(&self) -> Arc<T>;

    /// Loads what was saved, writing the defaults when nothing was.
    fn start(&self) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Stops looking for changes made by hand.
    fn stop(&self);

    /// Why what was saved is not being used, or `None`. [`Store::current`] is then the last good
    /// value, or the defaults.
    fn problem(&self) -> Option<String>;

    /// Saves the changed value, and returns once it is on disk.
    fn update<'a>(
        &'a self,
        change: Box<dyn FnOnce(&T) -> T + Send + 'a>,
    ) -> BoxFuture<'a, Result<(), StoreError>>;
}

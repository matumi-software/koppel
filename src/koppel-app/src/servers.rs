//! The servers a client attaches from. `connection.json` holds the list; this keeps one USB client
//! per entry, adds a server by enrolling with a pairing code or a server key (Skakel 0.3.0),
//! removes one, changes its link settings and tests its connection. See `docs/several-servers.md`.
//!
//! The confirm rule holds: a change is written to `connection.json`, the file's subscription then
//! pushes `ServersChanged` and brings the USB clients in line, so the UI shows what the file holds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koppel_config::files::ClientConnection;
use koppel_config::{
    AttachedByServer, ClientServers, ConfigFile, ConfigFileError, DEFAULT_GIVE_UP_AFTER_SECONDS,
    DEFAULT_RETRY_EVERY_SECONDS, ServerEntry,
};
use koppel_usb::{ClientServerDeps, ClientsError, UsbClients};
use serde::Deserialize;
use skakel_client::{Browser, ClientOptions, EnrollError, Url};
use tokio::sync::Mutex as AsyncMutex;
use ts_rs::TS;

use crate::activity::{ActivityLog, ActivityOutcome, ActivityReason, EntityKind, EntityRef};
use crate::connection::{URL_PROBLEM, https_authority, normalize, same_fingerprint};
use crate::push::{Event, Push};
use crate::random::{hex, random_bytes};
use crate::roles::skakel::{SkakelLinks, test_connection};
use crate::roles::{ConnectionTest, ServerView, ServersView, usb_store};
use crate::tasks::Tasks;

/// How long enrolling may take, the handshake and the key exchange together. Generous: a server on
/// slow hardware can take many seconds over its first request after a start.
const ENROLL_TIMEOUT: Duration = Duration::from_secs(60);

/// The longest a server's name is.
const MAX_NAME: usize = 80;

/// A server to add, with the code the server is showing or its server key.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AddServer {
    /// The server's HTTPS address, typed or from discovery.
    pub address: String,
    /// The eight-digit pairing code the server is showing.
    #[serde(default)]
    pub code: Option<String>,
    /// The server key copied from the server.
    #[serde(default)]
    pub key: Option<String>,
    /// What to call the server in the list; the server's own name when left out.
    #[serde(default)]
    pub name: Option<String>,
}

/// A change to one server's name or link settings. What is left out stays.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ServerSettings {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reconnect: Option<bool>,
    #[serde(default)]
    pub retry_every_seconds: Option<u32>,
    #[serde(default)]
    pub give_up_after_seconds: Option<u32>,
}

/// Why a request about the servers was not carried out. The text is what the UI shows.
#[derive(Debug, thiserror::Error)]
pub enum ServersError {
    #[error("There is no such server.")]
    NoSuchServer,
    /// A field's value is not acceptable: the field and what is wrong.
    #[error("{message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
    #[error("That server is added already.")]
    AlreadyAdded,
    /// The server did not enrol this client.
    #[error("{0}")]
    Enroll(String),
    #[error("The change could not be saved: {0}")]
    Save(String),
    #[error(transparent)]
    Usb(#[from] ClientsError),
}

impl ServersError {
    fn invalid(field: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            message: message.into(),
        }
    }
}

impl From<ConfigFileError> for ServersError {
    fn from(error: ConfigFileError) -> Self {
        Self::Save(error.to_string())
    }
}

/// One server's entry as the connection its link works from. A link saves what it learns through
/// this, so it never touches the other servers' entries.
#[derive(Clone)]
pub struct ServerSlot {
    file: ConfigFile<ClientServers>,
    id: String,
}

impl ServerSlot {
    pub fn new(file: ConfigFile<ClientServers>, id: impl Into<String>) -> Self {
        Self {
            file,
            id: id.into(),
        }
    }

    /// The saved connection, empty when the server is not saved any more.
    pub fn current(&self) -> Arc<ClientConnection> {
        Arc::new(
            self.file
                .current()
                .find(&self.id)
                .map(ServerEntry::connection)
                .unwrap_or_default(),
        )
    }

    /// Changes the connection of this server and nothing else. A server that is gone stays gone.
    pub async fn update(
        &self,
        change: impl FnOnce(&ClientConnection) -> ClientConnection,
    ) -> Result<Arc<ClientConnection>, ConfigFileError> {
        let id = self.id.clone();
        self.file
            .update(move |servers| {
                let mut next = servers.clone();
                if let Some(entry) = next.find_mut(&id) {
                    let changed = change(&entry.connection());
                    entry.server_url = changed.server_url;
                    entry.server_fingerprint = changed.server_fingerprint;
                    entry.client_id = changed.client_id;
                    entry.client_secret = changed.client_secret;
                    entry.next_server_fingerprint = changed.next_server_fingerprint;
                }
                next
            })
            .await?;
        Ok(self.current())
    }

    /// This server's link settings as saved.
    pub fn settings(&self) -> LinkSettings {
        self.file
            .current()
            .find(&self.id)
            .map(LinkSettings::of)
            .unwrap_or_default()
    }
}

/// What a server's link does when it drops, and how long it waits on one try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkSettings {
    pub reconnect: bool,
    pub retry_every: Duration,
    pub give_up_after: Duration,
}

impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            reconnect: true,
            retry_every: Duration::from_secs(u64::from(DEFAULT_RETRY_EVERY_SECONDS)),
            give_up_after: Duration::from_secs(u64::from(DEFAULT_GIVE_UP_AFTER_SECONDS)),
        }
    }
}

impl LinkSettings {
    fn of(entry: &ServerEntry) -> Self {
        Self {
            reconnect: entry.reconnect,
            retry_every: Duration::from_secs(u64::from(entry.retry_every_seconds)),
            give_up_after: Duration::from_secs(u64::from(entry.give_up_after_seconds)),
        }
    }
}

/// What the service is made from.
pub struct ServersParts {
    pub file: ConfigFile<ClientServers>,
    pub attached: ConfigFile<AttachedByServer>,
    pub clients: UsbClients,
    pub push: Push,
    pub browser: Option<Browser>,
    pub activity: Arc<ActivityLog>,
    pub tasks: Tasks,
    /// What the servers' people see this machine as.
    pub machine: String,
}

/// What is running for a server, to tell when it has to be made again.
#[derive(Clone, PartialEq, Eq)]
struct Applied {
    settings: LinkSettings,
    name: String,
}

pub struct Servers {
    file: ConfigFile<ClientServers>,
    attached: ConfigFile<AttachedByServer>,
    clients: UsbClients,
    push: Push,
    browser: Option<Browser>,
    activity: Arc<ActivityLog>,
    tasks: Tasks,
    machine: String,
    applied: Mutex<HashMap<String, Applied>>,
    /// One reconcile at a time.
    gate: AsyncMutex<()>,
}

impl Servers {
    pub fn new(parts: ServersParts) -> Arc<Self> {
        Arc::new(Self {
            file: parts.file,
            attached: parts.attached,
            clients: parts.clients,
            push: parts.push,
            browser: parts.browser,
            activity: parts.activity,
            tasks: parts.tasks,
            machine: parts.machine,
            applied: Mutex::new(HashMap::new()),
            gate: AsyncMutex::new(()),
        })
    }

    /// Starts a client for each saved server and follows `connection.json` from now on.
    pub async fn start(self: &Arc<Self>) -> Result<(), koppel_usb::StoreError> {
        self.attached
            .start()
            .await
            .map_err(|e| koppel_usb::StoreError::new(e.to_string()))?;
        self.reconcile().await;
        let weak = Arc::downgrade(self);
        self.file.subscribe(move |value| {
            let weak = weak.clone();
            async move {
                if let Some(servers) = weak.upgrade() {
                    servers
                        .push
                        .send(Event::ServersChanged(&ServersView::of(&value)));
                    servers.reconcile().await;
                }
            }
        });
        Ok(())
    }

    /// Stops every client, which keeps what it attached remembered, then the file it remembers in.
    pub async fn stop(&self) {
        self.clients.stop().await;
        self.attached.stop();
    }

    /// What the UI lists: the servers without their secrets.
    pub fn view(&self) -> ServersView {
        ServersView::of(&self.file.current())
    }

    /// Brings the USB clients in line with the saved servers: one each, made again when its link
    /// settings changed, renamed when its name did.
    async fn reconcile(&self) {
        let _one = self.gate.lock().await;
        let saved = self.file.current();
        for id in self.clients.ids() {
            if saved.find(&id).is_none() {
                self.clients.remove(&id).await;
                lock(&self.applied).remove(&id);
            }
        }
        for entry in &saved.servers {
            let want = Applied {
                settings: LinkSettings::of(entry),
                name: display_name(entry),
            };
            let have = lock(&self.applied).get(&entry.id).cloned();
            match have {
                Some(have) if have == want => continue,
                Some(have) if have.settings == want.settings => {
                    self.clients.rename(&entry.id, &want.name);
                }
                Some(_) => {
                    self.clients.remove(&entry.id).await;
                    self.make(entry, want).await;
                }
                None => self.make(entry, want).await,
            }
        }
    }

    async fn make(&self, entry: &ServerEntry, applied: Applied) {
        let slot = ServerSlot::new(self.file.clone(), entry.id.clone());
        let links = SkakelLinks::new(
            slot,
            self.browser.clone(),
            self.activity.clone(),
            self.tasks.clone(),
        );
        let deps = ClientServerDeps {
            name: applied.name.clone(),
            connection: usb_store::server_connection(&self.file, &entry.id),
            remembered: usb_store::server_attached(&self.file, &self.attached, &entry.id),
            links,
        };
        match self.clients.add(&entry.id, deps).await {
            Ok(()) => {
                lock(&self.applied).insert(entry.id.clone(), applied);
            }
            Err(e) => tracing::error!("Could not start the link to {}: {e}", applied.name),
        }
    }

    /// Enrols with the server and saves it as a new server of this client.
    pub async fn add(&self, request: AddServer) -> Result<ServerView, ServersError> {
        let address = https_authority(&request.address)
            .ok_or_else(|| ServersError::invalid("address", URL_PROBLEM))?;
        let url =
            Url::parse(&address).map_err(|_| ServersError::invalid("address", URL_PROBLEM))?;
        let name = request
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        if name.is_some_and(|n| n.chars().count() > MAX_NAME) {
            return Err(ServersError::invalid(
                "name",
                format!("Use a name of at most {MAX_NAME} characters."),
            ));
        }
        let options = ClientOptions::default().request_timeout(ENROLL_TIMEOUT);
        let (enrolled, linked_with) = match (request.code.as_deref(), request.key.as_deref()) {
            (Some(code), None) => (
                skakel_client::enroll_with_code(&url, code, &self.machine, &options).await,
                "code",
            ),
            (None, Some(key)) => (
                skakel_client::enroll_with_key(&url, key, &self.machine, &options).await,
                "key",
            ),
            _ => {
                return Err(ServersError::invalid(
                    "code",
                    "Give either the pairing code or the server key.",
                ));
            }
        };
        let enrolled = enrolled.map_err(enroll_error)?;
        let fingerprint = normalize(enrolled.pins.fingerprint());
        let mut made = None;
        let written = self
            .file
            .update(|servers| {
                let mut next = servers.clone();
                let same = next.servers.iter().any(|s| {
                    s.server_url.as_deref() == Some(address.as_str())
                        && same_fingerprint(s.server_fingerprint.as_deref(), Some(&fingerprint))
                });
                if same {
                    return next;
                }
                let id = next.free_id(new_id);
                let mut entry = ServerEntry::new(id);
                entry.name = Some(
                    name.map(str::to_owned)
                        .unwrap_or_else(|| enrolled.server_name.clone()),
                );
                entry.server_url = Some(address.clone());
                entry.server_fingerprint = Some(fingerprint.clone());
                entry.client_id = Some(enrolled.credentials.client_id.clone());
                entry.client_secret = Some(enrolled.credentials.client_secret.clone().into());
                entry.linked_with = Some(linked_with.to_owned());
                made = Some(ServerView::of(&entry));
                next.servers.push(entry);
                next
            })
            .await?;
        let view = made.ok_or(ServersError::AlreadyAdded)?;
        self.activity.record(
            EntityRef::new("server", view.id.clone(), EntityKind::Link),
            ActivityReason::Reconfigured,
            ActivityOutcome::Succeeded,
            format!("Added server {}", view.name.clone().unwrap_or_default()),
            None,
        );
        // The subscription brings the client up; wait for it so the caller sees the link starting.
        self.reconcile().await;
        drop(written);
        Ok(view)
    }

    /// Removes a server and what was remembered as attached from it. Its devices are released first.
    pub async fn remove(&self, id: &str) -> Result<(), ServersError> {
        let mut name = None;
        let mut found = false;
        self.file
            .update(|servers| {
                let mut next = servers.clone();
                if let Some(at) = next.servers.iter().position(|s| s.id == id) {
                    found = true;
                    name = Some(display_name(&next.servers[at]));
                    next.servers.remove(at);
                }
                next
            })
            .await?;
        if !found {
            return Err(ServersError::NoSuchServer);
        }
        self.reconcile().await;
        self.attached
            .update(|attached| {
                let mut next = attached.clone();
                next.servers.shift_remove(id);
                next
            })
            .await?;
        self.activity.record(
            EntityRef::new("server", id.to_owned(), EntityKind::Link),
            ActivityReason::Reconfigured,
            ActivityOutcome::Succeeded,
            format!("Removed server {}", name.unwrap_or_default()),
            None,
        );
        Ok(())
    }

    /// Changes a server's name or link settings.
    pub async fn change(
        &self,
        id: &str,
        change: ServerSettings,
    ) -> Result<ServerView, ServersError> {
        if let Some(name) = change.name.as_deref() {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > MAX_NAME {
                return Err(ServersError::invalid(
                    "name",
                    format!("Use a name of 1 to {MAX_NAME} characters."),
                ));
            }
        }
        if let Some(seconds) = change.retry_every_seconds
            && !(1..=3600).contains(&seconds)
        {
            return Err(ServersError::invalid(
                "retryEverySeconds",
                "Retry every 1 to 3600 seconds.",
            ));
        }
        if let Some(seconds) = change.give_up_after_seconds
            && !(1..=300).contains(&seconds)
        {
            return Err(ServersError::invalid(
                "giveUpAfterSeconds",
                "Give up after 1 to 300 seconds.",
            ));
        }
        let mut found = false;
        self.file
            .update(|servers| {
                let mut next = servers.clone();
                if let Some(entry) = next.find_mut(id) {
                    found = true;
                    if let Some(name) = change.name.as_deref() {
                        entry.name = Some(name.trim().to_owned());
                    }
                    if let Some(reconnect) = change.reconnect {
                        entry.reconnect = reconnect;
                    }
                    if let Some(seconds) = change.retry_every_seconds {
                        entry.retry_every_seconds = seconds;
                    }
                    if let Some(seconds) = change.give_up_after_seconds {
                        entry.give_up_after_seconds = seconds;
                    }
                }
                next
            })
            .await?;
        if !found {
            return Err(ServersError::NoSuchServer);
        }
        self.reconcile().await;
        self.file
            .current()
            .find(id)
            .map(ServerView::of)
            .ok_or(ServersError::NoSuchServer)
    }

    /// Tests the connection to a saved server.
    pub async fn test(&self, id: &str) -> Result<ConnectionTest, ServersError> {
        let connection = ServerSlot::new(self.file.clone(), id).current();
        let timeout = ServerSlot::new(self.file.clone(), id)
            .settings()
            .give_up_after;
        if self.file.current().find(id).is_none() {
            return Err(ServersError::NoSuchServer);
        }
        Ok(test_connection(&connection, timeout).await)
    }
}

/// What a server is called in the list: its name, else its address's host.
fn display_name(entry: &ServerEntry) -> String {
    entry
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| entry.server_url.as_deref().and_then(crate::tray::host_of))
        .unwrap_or_default()
}

/// `server-` and ten hex digits.
fn new_id() -> String {
    format!("server-{}", hex(&random_bytes::<5>()))
}

fn enroll_error(error: EnrollError) -> ServersError {
    match error {
        EnrollError::InvalidCode => ServersError::invalid("code", error.to_string()),
        EnrollError::InvalidKey(_) | EnrollError::KeyForAnotherServer => {
            ServersError::invalid("key", error.to_string())
        }
        other => ServersError::Enroll(other.to_string()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

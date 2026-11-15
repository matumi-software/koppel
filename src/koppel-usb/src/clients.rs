//! Several servers: one [`UsbClient`] per saved server, and what the rest of Koppel talks to. See
//! `docs/several-servers.md`.
//!
//! Each client keeps its own link, its own device list and its own part of `attached.json`, exactly as
//! the one client did. What is shared is what is the machine's: the platform (the kernel's virtual ports
//! and the driver), the settings, the activity log and the gateway.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::client::{BeginAttachError, GatewayRequest, Peeked, read_request};
use crate::tasks::{Listener, Tasks, listen, lock};
use crate::views::{ClientServersView, ServerDevicesView};
use crate::{
    ActivitySink, AttachedDevices, BusId, ClientOptions, Clock, Connection, Deps, Links, Observed,
    Platform, Store, StoreError, UsbClient, UsbSettings,
};

/// What every server's client shares.
pub struct Common {
    pub settings: Arc<dyn Observed<UsbSettings>>,
    /// What attaches on this machine: this kernel, or a USB/IP driver.
    pub platform: Platform,
    pub activity: Arc<dyn ActivitySink>,
    pub clock: Arc<dyn Clock>,
    pub options: ClientOptions,
}

/// What one server's client is made from.
pub struct ClientServerDeps {
    /// What the server calls itself, for the list.
    pub name: String,
    /// This server's saved connection.
    pub connection: Arc<dyn Observed<Connection>>,
    /// This server's part of `attached.json`.
    pub remembered: Arc<dyn Store<AttachedDevices>>,
    /// Makes the link to this server.
    pub links: Arc<dyn Links>,
}

struct Entry {
    client: UsbClient,
    name: String,
    /// Listens to the client, so a change of it reaches the list's listeners.
    _listener: Listener,
}

struct Inner {
    common: Common,
    entries: Mutex<IndexMap<String, Entry>>,
    /// Bumped each time any server's client says it changed, or one is added or removed.
    changed: watch::Sender<u64>,
    listeners: Tasks,
    gateway: Mutex<(Option<u16>, Option<String>)>,
}

/// One client per saved server. Cheap to clone.
#[derive(Clone)]
pub struct UsbClients {
    inner: Arc<Inner>,
}

impl UsbClients {
    pub fn new(common: Common) -> Self {
        Self {
            inner: Arc::new(Inner {
                common,
                entries: Mutex::new(IndexMap::new()),
                changed: watch::channel(0).0,
                listeners: Tasks::default(),
                gateway: Mutex::new((None, None)),
            }),
        }
    }

    fn bump(inner: &Inner) {
        inner.changed.send_modify(|version| *version += 1);
    }

    /// Adds a server's client and starts it. A server that is already here is left as it is.
    pub async fn add(&self, id: &str, deps: ClientServerDeps) -> Result<(), StoreError> {
        if lock(&self.inner.entries).contains_key(id) {
            return Ok(());
        }
        let common = &self.inner.common;
        let client = UsbClient::new(Deps {
            settings: common.settings.clone(),
            connection: deps.connection,
            remembered: deps.remembered,
            platform: common.platform.clone(),
            links: deps.links,
            activity: common.activity.clone(),
            clock: common.clock.clone(),
            options: common.options,
        });
        let weak = Arc::downgrade(&self.inner);
        let listener = client.on_changed(move |_| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    Self::bump(&inner);
                }
            }
        });
        // Its part of the gateway is the same as the others': one listener for the machine.
        let (port, fault) = lock(&self.inner.gateway).clone();
        if port.is_some() || fault.is_some() {
            client.set_gateway(port, fault).await;
        }
        client.start().await?;
        lock(&self.inner.entries).insert(
            id.to_owned(),
            Entry {
                client,
                name: deps.name,
                _listener: listener,
            },
        );
        Self::bump(&self.inner);
        Ok(())
    }

    /// Stops a server's client, which releases what it attached, and drops it. What it had remembered
    /// stays in its store, for the caller to delete when the server is forgotten, not merely stopped.
    pub async fn remove(&self, id: &str) {
        let entry = lock(&self.inner.entries).shift_remove(id);
        if let Some(entry) = entry {
            entry.client.stop().await;
            Self::bump(&self.inner);
        }
    }

    /// Changes the name a server shows under.
    pub fn rename(&self, id: &str, name: &str) {
        if let Some(entry) = lock(&self.inner.entries).get_mut(id) {
            entry.name = name.to_owned();
        }
        Self::bump(&self.inner);
    }

    /// The ids of the servers here, in the order they were added.
    pub fn ids(&self) -> Vec<String> {
        lock(&self.inner.entries).keys().cloned().collect()
    }

    /// One server's client.
    pub fn client(&self, id: &str) -> Option<UsbClient> {
        lock(&self.inner.entries)
            .get(id)
            .map(|entry| entry.client.clone())
    }

    /// What the UI shows now: every server with its link and its devices.
    pub fn current(&self) -> ClientServersView {
        let entries: Vec<(String, String, UsbClient)> = lock(&self.inner.entries)
            .iter()
            .map(|(id, entry)| (id.clone(), entry.name.clone(), entry.client.clone()))
            .collect();
        let mut view = ClientServersView {
            servers: Vec::with_capacity(entries.len()),
            unavailable: None,
            gateway: false,
            gateway_fault: lock(&self.inner.gateway).1.clone(),
        };
        for (id, name, client) in entries {
            let one = client.current();
            // What is the machine's rather than a server's is the same in every one's view.
            view.unavailable = view.unavailable.or(one.unavailable);
            view.gateway |= one.gateway;
            view.gateway_fault = view.gateway_fault.or(one.gateway_fault);
            view.servers.push(ServerDevicesView {
                id,
                name,
                link: one.link,
                link_message: one.link_message,
                devices: one.devices,
            });
        }
        view
    }

    /// Calls `listener` with what the UI shows, each time any server's client may have changed it.
    pub fn on_changed<F, Fut>(&self, listener: F) -> Listener
    where
        F: Fn(Arc<ClientServersView>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let this = self.clone();
        listen(
            &self.inner.listeners,
            "the client's servers",
            self.inner.changed.subscribe(),
            move |_| {
                let view = Arc::new(this.current());
                listener(view)
            },
        )
    }

    /// Why `attached.json` is not being used, or `None`.
    pub fn remembered_problem(&self) -> Option<String> {
        lock(&self.inner.entries)
            .values()
            .find_map(|entry| entry.client.remembered_problem())
    }

    /// Remembers the device of that server and starts attaching it. The same bus ID on another server
    /// is not attached by the same driver request at once: a driver names a device by bus ID alone, so
    /// a connection at the gateway could not tell which server's it was.
    pub async fn begin_attach(&self, server_id: &str, bus_id: &str) -> Result<(), ClientsError> {
        let client = self.client(server_id).ok_or(ClientsError::NoSuchServer)?;
        let parsed: BusId = bus_id
            .parse()
            .map_err(|_| ClientsError::Attach(BeginAttachError::InvalidBusId))?;
        let others: Vec<UsbClient> = lock(&self.inner.entries)
            .iter()
            .filter(|(id, _)| id.as_str() != server_id)
            .map(|(_, entry)| entry.client.clone())
            .collect();
        for other in others {
            if other.has_booking(&parsed).await {
                return Err(ClientsError::BusIdBusy);
            }
        }
        client
            .begin_attach(bus_id)
            .await
            .map_err(ClientsError::Attach)
    }

    /// Releases the device of that server and forgets it.
    pub async fn detach(&self, server_id: &str, bus_id: &str) -> Result<(), ClientsError> {
        let client = self.client(server_id).ok_or(ClientsError::NoSuchServer)?;
        client.detach(bus_id).await;
        Ok(())
    }

    /// Called by the gateway with its port when it starts listening, and `None` when it stops.
    pub async fn set_gateway(&self, port: Option<u16>, fault: Option<String>) {
        *lock(&self.inner.gateway) = (port, fault.clone());
        let clients: Vec<UsbClient> = lock(&self.inner.entries)
            .values()
            .map(|entry| entry.client.clone())
            .collect();
        for client in clients {
            client.set_gateway(port, fault.clone()).await;
        }
        Self::bump(&self.inner);
    }

    /// The port the gateway listens on, or `None` while it does not.
    pub fn gateway_port(&self) -> Option<u16> {
        lock(&self.inner.gateway).0
    }

    /// Stops every server's client.
    pub async fn stop(&self) {
        let entries: Vec<Entry> = lock(&self.inner.entries)
            .drain(..)
            .map(|(_, e)| e)
            .collect();
        for entry in entries {
            entry.client.stop().await;
        }
        self.inner
            .listeners
            .stop(std::time::Duration::from_secs(5))
            .await;
    }

    /// Serves one connection from a driver or `usbip attach` on this machine. The first thing it asks
    /// for decides which server's client gets it:
    ///
    /// - With a driver in play, the client that asked the driver for that bus ID and is waiting at the
    ///   gateway. [`begin_attach`](Self::begin_attach) lets only one wait for a given bus ID at a time.
    /// - Otherwise, the first server, in the order they were added, that is offering that bus ID.
    /// - A device list goes to the first server.
    ///
    /// A connection nobody can serve is dropped, as the single client dropped one it did not expect.
    pub async fn serve_gateway(
        &self,
        mut local: TcpStream,
        peer: SocketAddr,
        stopping: &CancellationToken,
    ) {
        let timeout = self.inner.common.options.attach_timeout;
        let request = match read_request(&mut local, timeout, stopping).await {
            Peeked::Request(request) => request,
            Peeked::Refuse(why) => {
                tracing::warn!("Gateway request failed: {why}");
                crate::client::refuse_import(&mut local).await;
                return;
            }
            Peeked::Over => return,
        };
        let clients: Vec<UsbClient> = lock(&self.inner.entries)
            .values()
            .map(|entry| entry.client.clone())
            .collect();
        let chosen = match &request {
            GatewayRequest::DevList => clients.into_iter().next(),
            GatewayRequest::Import(bus_id) => {
                let mut chosen = None;
                for client in clients {
                    let takes = if client.uses_driver() {
                        client.has_booking(bus_id).await
                    } else {
                        client.offers(bus_id)
                    };
                    if takes {
                        chosen = Some(client);
                        break;
                    }
                }
                chosen
            }
        };
        match chosen {
            Some(client) => {
                client
                    .serve_gateway_request(local, peer, stopping, request)
                    .await
            }
            None => {
                tracing::warn!("Gateway request failed: no server's client has asked for it");
                crate::client::refuse_import(&mut local).await;
            }
        }
    }
}

/// Why a request that names a server's device was not carried out.
#[derive(Debug, thiserror::Error)]
pub enum ClientsError {
    /// No such server is saved.
    #[error("There is no such server.")]
    NoSuchServer,
    /// Another server's device with the same bus ID is being attached through the driver.
    #[error(
        "Another server's device with the same bus ID is being attached. Try again in a moment."
    )]
    BusIdBusy,
    /// The attach was not started.
    #[error(transparent)]
    Attach(BeginAttachError),
}

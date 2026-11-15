//! koppel-usb's settings, connection and device lists over the config files. koppel-usb names no
//! file: it follows an [`Observed`] value and saves into a [`Store`], and these are the ones that
//! sit on `settings.json`, `connection.json`, `devices.json` and `attached.json`. A change is
//! delivered through the file, so whoever writes it still waits for koppel-usb to have applied it.

use std::sync::Arc;

use koppel_config::files::{self, ClientConnection, KoppelSettings};
use koppel_config::{AttachedByServer, AttachedFromServer, ClientServers, FIRST_SERVER};
use koppel_config::{ConfigFile, ConfigFileError, Subscription as FileSubscription};
use koppel_usb::{
    AttachedDevices, BoxFuture, Connection, Handler, Observed, Role, SharedDevices, Store,
    StoreError, Subscription, UsbSettings,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// What a config file's value must be for [`ConfigFile`].
trait FileValue: Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + 'static {}

impl<T> FileValue for T where
    T: Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + 'static
{
}

/// `settings.json` as koppel-usb follows it.
pub fn settings(file: &ConfigFile<KoppelSettings>) -> Arc<dyn Observed<UsbSettings>> {
    Arc::new(Mapped {
        file: file.clone(),
        to: usb_settings,
    })
}

/// `connection.json` as koppel-usb follows it.
pub fn connection(file: &ConfigFile<ClientConnection>) -> Arc<dyn Observed<Connection>> {
    Arc::new(Mapped {
        file: file.clone(),
        to: usb_connection,
    })
}

/// `devices.json` as koppel-usb saves it.
pub fn shared(file: &ConfigFile<files::SharedDevices>) -> Arc<dyn Store<SharedDevices>> {
    Arc::new(Persisted {
        file: file.clone(),
        to: usb_shared,
        from: file_shared,
    })
}

/// `attached.json` as koppel-usb saves it.
pub fn attached(file: &ConfigFile<files::AttachedDevices>) -> Arc<dyn Store<AttachedDevices>> {
    Arc::new(Persisted {
        file: file.clone(),
        to: usb_attached,
        from: file_attached,
    })
}

/// The connection koppel-usb hands to the transport, as the file holds it.
pub fn file_connection(connection: &Connection) -> ClientConnection {
    ClientConnection {
        server_url: connection.server_url.clone(),
        server_fingerprint: connection.server_fingerprint.clone(),
        client_id: connection.client_id.clone(),
        client_secret: connection.client_secret.clone(),
        next_server_fingerprint: connection.next_server_fingerprint.clone(),
    }
}

fn usb_settings(settings: &KoppelSettings) -> UsbSettings {
    UsbSettings {
        role: match settings.role {
            files::Role::Server => Role::Server,
            files::Role::Client => Role::Client,
            files::Role::Both => Role::Both,
        },
        share_on_startup: settings.share_on_startup,
    }
}

fn usb_connection(connection: &ClientConnection) -> Connection {
    Connection {
        server_url: connection.server_url.clone(),
        server_fingerprint: connection.server_fingerprint.clone(),
        client_id: connection.client_id.clone(),
        client_secret: connection.client_secret.clone(),
        next_server_fingerprint: connection.next_server_fingerprint.clone(),
    }
}

fn usb_shared(shared: &files::SharedDevices) -> SharedDevices {
    SharedDevices {
        bus_ids: shared.bus_ids.clone(),
        vid_pids: shared.vid_pids.clone(),
    }
}

fn file_shared(shared: &SharedDevices) -> files::SharedDevices {
    files::SharedDevices {
        bus_ids: shared.bus_ids.clone(),
        vid_pids: shared.vid_pids.clone(),
    }
}

fn usb_attached(attached: &files::AttachedDevices) -> AttachedDevices {
    AttachedDevices {
        server_fingerprint: attached.server_fingerprint.clone(),
        bus_ids: attached.bus_ids.clone(),
        vid_pids: attached.vid_pids.clone(),
    }
}

fn file_attached(attached: &AttachedDevices) -> files::AttachedDevices {
    files::AttachedDevices {
        server_fingerprint: attached.server_fingerprint.clone(),
        bus_ids: attached.bus_ids.clone(),
        vid_pids: attached.vid_pids.clone(),
    }
}

/// Stops listening to a file when it is dropped.
struct Unsubscribe<T: FileValue> {
    file: ConfigFile<T>,
    subscription: FileSubscription,
}

impl<T: FileValue> Drop for Unsubscribe<T> {
    fn drop(&mut self) {
        self.file.unsubscribe(self.subscription);
    }
}

/// A file followed as another type.
struct Mapped<T: FileValue, U> {
    file: ConfigFile<T>,
    to: fn(&T) -> U,
}

impl<T: FileValue, U: Send + Sync + 'static> Observed<U> for Mapped<T, U> {
    fn current(&self) -> Arc<U> {
        Arc::new((self.to)(&self.file.current()))
    }

    fn subscribe(&self, handler: Handler<U>) -> Subscription {
        let to = self.to;
        let subscription = self
            .file
            .subscribe(move |value| handler(Arc::new(to(&value))));
        Box::new(Unsubscribe {
            file: self.file.clone(),
            subscription,
        })
    }
}

/// A file saved as another type.
struct Persisted<T: FileValue, U> {
    file: ConfigFile<T>,
    to: fn(&T) -> U,
    from: fn(&U) -> T,
}

impl<T: FileValue, U: Send + Sync + 'static> Store<U> for Persisted<T, U> {
    fn current(&self) -> Arc<U> {
        Arc::new((self.to)(&self.file.current()))
    }

    fn start(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move { self.file.start().await.map_err(failed) })
    }

    fn stop(&self) {
        self.file.stop();
    }

    fn problem(&self) -> Option<String> {
        self.file.problem()
    }

    fn update<'a>(
        &'a self,
        change: Box<dyn FnOnce(&U) -> U + Send + 'a>,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let (to, from) = (self.to, self.from);
            self.file
                .update(move |current| from(&change(&to(current))))
                .await
                .map(|_| ())
                .map_err(failed)
        })
    }
}

fn failed(error: ConfigFileError) -> StoreError {
    StoreError::new(error.to_string())
}

/// One server's entry in `connection.json` as koppel-usb follows it. A server that is not in the
/// file any more reads as an empty connection.
pub fn server_connection(
    file: &ConfigFile<ClientServers>,
    id: &str,
) -> Arc<dyn Observed<Connection>> {
    Arc::new(ServerConnection {
        file: file.clone(),
        id: id.to_owned(),
    })
}

fn entry_connection(servers: &ClientServers, id: &str) -> Connection {
    servers
        .find(id)
        .map(|entry| usb_connection(&entry.connection()))
        .unwrap_or_default()
}

struct ServerConnection {
    file: ConfigFile<ClientServers>,
    id: String,
}

impl Observed<Connection> for ServerConnection {
    fn current(&self) -> Arc<Connection> {
        Arc::new(entry_connection(&self.file.current(), &self.id))
    }

    fn subscribe(&self, handler: Handler<Connection>) -> Subscription {
        let id = self.id.clone();
        let subscription = self
            .file
            .subscribe(move |value| handler(Arc::new(entry_connection(&value, &id))));
        Box::new(Unsubscribe {
            file: self.file.clone(),
            subscription,
        })
    }
}

/// One server's part of `attached.json`. koppel-usb tells a server by its fingerprint, which is
/// the entry's own, except for the first server of an old flat file, which keeps the fingerprint
/// that file recorded until the first save.
pub fn server_attached(
    servers: &ConfigFile<ClientServers>,
    attached: &ConfigFile<AttachedByServer>,
    id: &str,
) -> Arc<dyn Store<AttachedDevices>> {
    Arc::new(ServerAttached {
        servers: servers.clone(),
        attached: attached.clone(),
        id: id.to_owned(),
    })
}

struct ServerAttached {
    servers: ConfigFile<ClientServers>,
    attached: ConfigFile<AttachedByServer>,
    id: String,
}

impl ServerAttached {
    fn read(&self, attached: &AttachedByServer) -> AttachedDevices {
        let part = attached.servers.get(&self.id).cloned().unwrap_or_default();
        let legacy = (self.id == FIRST_SERVER)
            .then(|| attached.legacy_fingerprint.clone())
            .flatten();
        let fingerprint = legacy.or_else(|| {
            self.servers
                .current()
                .find(&self.id)
                .and_then(|entry| entry.server_fingerprint.clone())
        });
        AttachedDevices {
            server_fingerprint: fingerprint,
            bus_ids: part.bus_ids,
            vid_pids: part.vid_pids,
        }
    }
}

impl Store<AttachedDevices> for ServerAttached {
    fn current(&self) -> Arc<AttachedDevices> {
        Arc::new(self.read(&self.attached.current()))
    }

    // The file is shared by every server's client: the servers service starts and stops it.
    fn start(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }

    fn stop(&self) {}

    fn problem(&self) -> Option<String> {
        self.attached.problem()
    }

    fn update<'a>(
        &'a self,
        change: Box<dyn FnOnce(&AttachedDevices) -> AttachedDevices + Send + 'a>,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            self.attached
                .update(|current| {
                    let next = change(&self.read(current));
                    let mut written = current.clone();
                    written.servers.insert(
                        self.id.clone(),
                        AttachedFromServer {
                            bus_ids: next.bus_ids,
                            vid_pids: next.vid_pids,
                        },
                    );
                    if self.id == FIRST_SERVER {
                        written.legacy_fingerprint = None;
                    }
                    written
                })
                .await
                .map(|_| ())
                .map_err(failed)
        })
    }
}

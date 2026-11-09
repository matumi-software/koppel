//! The client's link to its saved server, on skakel-client, as koppel-usb's [`Links`] and [`Link`]:
//! the control channel with its token renewal, reconnecting, the certificate handover and following
//! a server that moved, the remote device list (subscribed again after each reconnect), and a
//! connection of its own for each USB stream.
//!
//! What the link learns is saved in `connection.json` as 0.1.x's `ConnectionStore` saved it: the
//! certificates the server announces (`SavePinsAsync`), only while the saved connection is still
//! that server, and the address a moved server answers at (`MoveServerAsync`), only while nobody
//! saved another connection meanwhile, with a `serverMoved` entry in the activity log.
//!
//! koppel-usb makes a link under its own lock and reads nothing it reports before it is started,
//! so making one only builds it; everything runs from [`Link::start`].

use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use koppel_config::ConfigFile;
use koppel_config::files::ClientConnection;
use koppel_usb::{
    BoxFuture, DeviceStream, Link, LinkEvents, LinkStatus, Links, OpenError, StreamOpener,
};
use skakel_client::{Browser, ClientOptions, Credentials, LinkState, Pins, Url};
use tokio_util::sync::CancellationToken;

use super::remote::from_proto;
use super::remote::proto::WatchDevicesRequest;
use super::remote::proto::remote_devices_client::RemoteDevicesClient;
use super::streams::open_usb;
use crate::activity::{ActivityLog, ActivityOutcome, ActivityReason, EntityRef};

/// How long a remote device list that ended waits before it is asked for again, while the link
/// stays up.
const RESUBSCRIBE: Duration = Duration::from_secs(2);

/// Makes the links, over `connection.json` and the browser the links follow a moved server with.
pub(crate) struct SkakelLinks {
    connection: ConfigFile<ClientConnection>,
    browser: Option<Browser>,
    activity: Arc<ActivityLog>,
}

impl SkakelLinks {
    pub(crate) fn new(
        connection: ConfigFile<ClientConnection>,
        browser: Option<Browser>,
        activity: Arc<ActivityLog>,
    ) -> Arc<Self> {
        Arc::new(Self {
            connection,
            browser,
            activity,
        })
    }
}

impl Links for SkakelLinks {
    fn connect(&self, connection: &ClientConnection, events: LinkEvents) -> Arc<dyn Link> {
        let made = make(connection, self.browser.clone());
        let shared = Arc::new(Shared {
            saved: self.connection.clone(),
            activity: self.activity.clone(),
            events,
            learned: Mutex::new(connection.clone()),
            watching: Mutex::new(None),
        });
        match made {
            Ok(link) => {
                let link = Arc::new(link);
                wire(&link, &shared);
                Arc::new(SkakelLink {
                    link: Ok(link),
                    shared,
                })
            }
            Err(message) => Arc::new(SkakelLink {
                link: Err(message),
                shared,
            }),
        }
    }
}

/// skakel-client's link for a saved connection, or why it cannot be made.
fn make(
    connection: &ClientConnection,
    browser: Option<Browser>,
) -> Result<skakel_client::Link, String> {
    let address = connection
        .server_url
        .as_deref()
        .and_then(|url| Url::parse(url).ok())
        .ok_or_else(|| "The saved server address is not an HTTPS address.".to_owned())?;
    let pins = Pins::with_next(
        connection.server_fingerprint.as_deref().unwrap_or_default(),
        connection.next_server_fingerprint.as_deref(),
    )
    .map_err(|_| "The saved fingerprint is not a SHA-256 fingerprint.".to_owned())?;
    let credentials = Credentials::new(
        connection.client_id.clone().unwrap_or_default(),
        connection.client_secret.clone().unwrap_or_default(),
    );
    skakel_client::Link::new(
        &address,
        pins,
        credentials,
        ClientOptions {
            browser,
            ..ClientOptions::default()
        },
    )
    .map_err(|e| e.failure().message().to_owned())
}

/// What the link's handlers share.
struct Shared {
    saved: ConfigFile<ClientConnection>,
    activity: Arc<ActivityLog>,
    events: LinkEvents,
    /// The connection the link is for, moved on as it learns new certificates and addresses.
    learned: Mutex<ClientConnection>,
    /// The remote device list while the link is up.
    watching: Mutex<Option<CancellationToken>>,
}

/// Registers the link's handlers. They hold the link weakly, so it goes when koppel-usb drops it.
fn wire(link: &Arc<skakel_client::Link>, shared: &Arc<Shared>) {
    {
        let shared = shared.clone();
        let weak = Arc::downgrade(link);
        link.on_status_changed(move |status| {
            let shared = shared.clone();
            let weak = weak.clone();
            async move {
                let reported = match status.state {
                    LinkState::Stopped => None,
                    LinkState::Connecting => Some(LinkStatus::Connecting),
                    LinkState::Connected => Some(LinkStatus::Connected),
                    LinkState::Failed | LinkState::Closed => Some(LinkStatus::Down(
                        status
                            .failure
                            .as_ref()
                            .map(|f| f.message().to_owned())
                            .unwrap_or_else(|| "The connection to the server was lost.".to_owned()),
                    )),
                };
                if status.state == LinkState::Connected {
                    shared.watch_devices(weak);
                } else {
                    shared.stop_watching();
                }
                if let Some(reported) = reported {
                    shared.events.status(reported).await;
                }
            }
        });
    }
    {
        let shared = shared.clone();
        link.on_pins_changed(move |pins| {
            let shared = shared.clone();
            async move { shared.save_pins(&pins).await }
        });
    }
    {
        let shared = shared.clone();
        link.on_address_changed(move |address| {
            let shared = shared.clone();
            async move { shared.move_server(&address).await }
        });
    }
}

/// The address as Koppel saves it: scheme, host and port, no path.
fn address_text(url: &Url) -> String {
    url.as_str().trim_end_matches('/').to_owned()
}

impl Shared {
    /// Stores the certificates the link trusts now, after the server told it of a next one,
    /// switched to it or cancelled it, as long as the saved connection is still the link's server.
    /// Writes nothing when that changes nothing. Fails when the saved connection is another
    /// server's, so the link does not tell the server it has the next certificate.
    async fn save_pins(&self, pins: &Pins) -> Result<(), String> {
        let link = self.learned();
        let with = |c: &ClientConnection| ClientConnection {
            server_fingerprint: Some(pins.fingerprint().to_owned()),
            next_server_fingerprint: pins.next_fingerprint().map(str::to_owned),
            ..c.clone()
        };
        let current = self.saved.current();
        if !continues(&current, &link) {
            return Err("The saved connection is for another server now.".into());
        }
        if with(&current) != *current {
            let written = self
                .saved
                .update(|c| {
                    if continues(c, &link) {
                        with(c)
                    } else {
                        c.clone()
                    }
                })
                .await
                .map_err(|e| e.to_string())?;
            if !continues(&written, &link) {
                return Err("The saved connection is for another server now.".into());
            }
        }
        *self.learned.lock().unwrap_or_else(PoisonError::into_inner) = with(&link);
        Ok(())
    }

    /// Points the saved connection at the address the server moved to, as long as it is still the
    /// link's server at the link's address: whatever the person saved meanwhile wins. Only the
    /// address changes.
    async fn move_server(&self, address: &Url) {
        let link = self.learned();
        let to = address_text(address);
        let same = |c: &ClientConnection| c.server_url == link.server_url && continues(c, &link);
        if !same(&self.saved.current()) {
            return;
        }
        let written = self
            .saved
            .update(|c| {
                if same(c) {
                    ClientConnection {
                        server_url: Some(to.clone()),
                        ..c.clone()
                    }
                } else {
                    c.clone()
                }
            })
            .await;
        match written {
            Ok(written)
                if written.server_url.as_deref() == Some(&to) && continues(&written, &link) =>
            {
                let from = link.server_url.clone().unwrap_or_default();
                self.learned
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .server_url = Some(to.clone());
                tracing::info!("The server moved from {from} to {to}; the connection follows it");
                self.activity.record(
                    EntityRef::new("link", host_of(&to), "link"),
                    ActivityReason::ServerMoved,
                    ActivityOutcome::Succeeded,
                    format!(
                        "The server moved from {} to {}",
                        host_of(&from),
                        host_of(&to)
                    ),
                    None,
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("Could not save where the server moved to: {e}"),
        }
    }

    fn learned(&self) -> ClientConnection {
        self.learned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Asks the server for its device list and passes each one on, while the link is up. A list
    /// that ends while the link stays up is asked for again.
    fn watch_devices(self: &Arc<Self>, link: Weak<skakel_client::Link>) {
        let stop = CancellationToken::new();
        if let Some(previous) = self
            .watching
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(stop.clone())
        {
            previous.cancel();
        }
        let shared = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(channel) = link.upgrade().map(|l| l.channel()) else {
                    return;
                };
                let mut client = RemoteDevicesClient::new(channel);
                let watched = async {
                    let mut lists = client.watch(WatchDevicesRequest {}).await?.into_inner();
                    while let Some(list) = lists.message().await? {
                        let Some(shared) = shared.upgrade() else {
                            return Ok(());
                        };
                        shared.events.devices(from_proto(list)).await;
                    }
                    Ok::<_, tonic::Status>(())
                };
                tokio::select! {
                    () = stop.cancelled() => return,
                    ended = watched => {
                        if let Err(status) = ended {
                            tracing::debug!("The server's device list ended: {:?} {}", status.code(), status.message());
                        }
                    }
                }
                tokio::select! {
                    () = stop.cancelled() => return,
                    () = tokio::time::sleep(RESUBSCRIBE) => {}
                }
            }
        });
    }

    fn stop_watching(&self) {
        if let Some(stop) = self
            .watching
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            stop.cancel();
        }
    }
}

/// This is `previous`'s server, at most moved to another address or on to its next certificate:
/// the same credentials, pinned to the same fingerprint or to the next one `previous` was told of.
/// 0.1.x's `ClientConnection.Continues`.
fn continues(now: &ClientConnection, previous: &ClientConnection) -> bool {
    let digits = |f: Option<&str>| -> String {
        f.unwrap_or_default()
            .chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let fingerprint = digits(now.server_fingerprint.as_deref());
    now.client_id == previous.client_id
        && now.client_secret == previous.client_secret
        && now.server_fingerprint.is_some()
        && (fingerprint == digits(previous.server_fingerprint.as_deref())
            || (previous.next_server_fingerprint.is_some()
                && fingerprint == digits(previous.next_server_fingerprint.as_deref())))
}

/// The host of an address such as `https://10.0.0.5:5240`, as the activity log names a server.
fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

/// One link. See the module docs.
struct SkakelLink {
    link: Result<Arc<skakel_client::Link>, String>,
    shared: Arc<Shared>,
}

impl Link for SkakelLink {
    fn start(&self) {
        match &self.link {
            Ok(link) => link.start(),
            Err(message) => {
                let events = self.shared.events.clone();
                let message = message.clone();
                tokio::spawn(async move { events.status(LinkStatus::Down(message)).await });
            }
        }
    }

    fn address(&self) -> String {
        match &self.link {
            Ok(link) => address_text(&link.address()),
            Err(_) => self.shared.learned().server_url.unwrap_or_default(),
        }
    }

    fn stop(&self) -> BoxFuture<'static, ()> {
        self.shared.stop_watching();
        let link = self.link.as_ref().ok().cloned();
        Box::pin(async move {
            if let Some(link) = link {
                link.stop().await;
            }
        })
    }
}

impl StreamOpener for SkakelLink {
    fn open(&self) -> BoxFuture<'_, Result<Box<dyn DeviceStream>, OpenError>> {
        Box::pin(async move {
            match &self.link {
                Ok(link) => open_usb(link).await,
                Err(message) => Err(OpenError::Failed(message.clone())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(fingerprint: &str, next: Option<&str>) -> ClientConnection {
        ClientConnection {
            server_url: Some("https://a:5240".into()),
            server_fingerprint: Some(fingerprint.into()),
            client_id: Some("client-1".into()),
            client_secret: Some("s".into()),
            next_server_fingerprint: next.map(str::to_owned),
        }
    }

    #[test]
    fn the_same_server_continues_onto_its_next_certificate_and_no_further() {
        let before = connection("AA:BB", Some("CC:DD"));
        assert!(continues(&connection("aabb", None), &before));
        assert!(continues(&connection("CC:DD", None), &before));
        assert!(!continues(&connection("EE:FF", None), &before));
        let other = ClientConnection {
            client_id: Some("client-2".into()),
            ..connection("AA:BB", None)
        };
        assert!(!continues(&other, &before));
    }

    #[test]
    fn a_moved_server_is_named_by_its_host() {
        assert_eq!(host_of("https://10.0.0.5:5240"), "10.0.0.5");
        assert_eq!(host_of("https://[fe80::1]:5240"), "[fe80::1]");
    }
}

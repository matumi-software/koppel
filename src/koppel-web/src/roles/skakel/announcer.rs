//! Answers Skakel discovery probes while the role is server or both, in Skakel's format (Skakel's
//! `docs/protocol.md`, "Discovery"). Koppel answers them itself, as 0.1.x's `ServerAnnouncer` did,
//! because skakel-server's answerer is on or off for the life of the server and the role changes
//! while Koppel runs. Each answer carries the certificate presented at that moment, so a switch is
//! announced at once. `Discovery:Enabled=false` turns it off.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use koppel_config::ConfigFile;
use koppel_config::files::{KoppelSettings, Role};
use skakel_discovery::net::should_answer;
use skakel_discovery::{DiscoveryAnswer, create_answer};
use skakel_protocol::{SkakelApp, VERSIONS};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// What the answers say, but the fingerprint.
pub(crate) struct Announcement {
    pub name: String,
    pub port: u16,
    pub app: SkakelApp,
}

/// Answers on one UDP port while the role serves.
pub(crate) struct Announcer {
    bind: SocketAddr,
    announcement: Arc<Announcement>,
    fingerprint: Arc<dyn Fn() -> String + Send + Sync>,
    running: Mutex<Option<(CancellationToken, JoinHandle<()>)>>,
}

impl Announcer {
    /// An announcer on UDP `port` at `address`: the address the web ports bind to when it is an IPv4
    /// one (a server that listens on loopback only has nobody else to answer), else every IPv4
    /// address, which receiving broadcasts takes.
    pub(crate) fn new(
        address: Option<IpAddr>,
        port: u16,
        announcement: Announcement,
        fingerprint: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> Arc<Self> {
        let ip = match address {
            Some(IpAddr::V4(v4)) => IpAddr::V4(v4),
            _ => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        Arc::new(Self {
            bind: SocketAddr::new(ip, port),
            announcement: Arc::new(announcement),
            fingerprint,
            running: Mutex::new(None),
        })
    }

    /// Follows the role: answers while it is server or both.
    pub(crate) async fn follow(self: &Arc<Self>, settings: &ConfigFile<KoppelSettings>) {
        self.apply(settings.current().role).await;
        let weak = Arc::downgrade(self);
        settings.subscribe(move |value| {
            let weak = weak.clone();
            async move {
                if let Some(announcer) = weak.upgrade() {
                    announcer.apply(value.role).await;
                }
            }
        });
    }

    async fn apply(&self, role: Role) {
        let wanted = role != Role::Client;
        if wanted == self.is_running() {
            return;
        }
        if !wanted {
            self.stop().await;
            tracing::info!("Stopped answering discovery on the local network");
            return;
        }
        let socket = match UdpSocket::bind(self.bind).await {
            Ok(socket) => socket,
            Err(e) => {
                // Another program holds the UDP port. Clients can still enter this server's address.
                tracing::warn!(
                    "Could not announce on the local network (UDP {}: {e}); clients must enter this server's address",
                    self.bind
                );
                return;
            }
        };
        let stop = CancellationToken::new();
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.is_some() {
            return;
        }
        tracing::info!(
            "Answering discovery on the local network as {} (UDP port {})",
            self.announcement.name,
            self.bind.port()
        );
        let task = tokio::spawn(answer(
            socket,
            self.announcement.clone(),
            self.fingerprint.clone(),
            stop.clone(),
        ));
        *running = Some((stop, task));
    }

    fn is_running(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Stops answering, and returns once the UDP port is free again.
    pub(crate) async fn stop(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some((stop, task)) = running {
            stop.cancel();
            let _ = task.await;
        }
    }
}

/// Answers a full-size probe only, and never with more bytes than it had, so nobody can use this
/// server to multiply traffic. A failed send is about that one answer; a failed receive that is
/// about one datagram does not stop the answers either.
async fn answer(
    socket: UdpSocket,
    announcement: Arc<Announcement>,
    fingerprint: Arc<dyn Fn() -> String + Send + Sync>,
    stop: CancellationToken,
) {
    let mut cached: Option<(String, Vec<u8>)> = None;
    let mut buffer = [0u8; 2048];
    loop {
        let received = tokio::select! {
            () = stop.cancelled() => return,
            received = socket.recv_from(&mut buffer) => received,
        };
        let (length, from) = match received {
            Ok(received) => received,
            Err(_) => {
                // Windows reports an earlier answer that could not be delivered, or a datagram
                // larger than the buffer, on the next receive. Anyone can send the second.
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
        };
        let now = fingerprint();
        if cached.as_ref().is_none_or(|(known, _)| *known != now) {
            match create_answer(&DiscoveryAnswer {
                name: announcement.name.clone(),
                port: announcement.port,
                fingerprint: now.clone(),
                app: announcement.app.clone(),
                versions: VERSIONS.iter().map(|v| *v as i32).collect(),
            }) {
                Ok(bytes) => cached = Some((now, bytes)),
                Err(e) => {
                    tracing::warn!("The discovery answer could not be made: {e:?}");
                    continue;
                }
            }
        }
        let Some((_, bytes)) = &cached else { continue };
        if !should_answer(&buffer[..length], from, bytes) {
            tracing::debug!("Ignored a discovery datagram from {from}");
            continue;
        }
        if let Err(e) = socket.send_to(bytes, from).await {
            tracing::debug!("Could not answer a discovery probe from {from}: {e}");
        }
    }
}

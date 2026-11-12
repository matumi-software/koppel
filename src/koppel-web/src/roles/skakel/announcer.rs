//! Answers Skakel discovery probes while the role is server or both, in Skakel's format (Skakel's
//! `docs/protocol.md`, "Discovery"). Koppel answers them itself, as 0.1.x's `ServerAnnouncer` did,
//! because skakel-server's answerer is on or off for the life of the server and the role changes
//! while Koppel runs. Each answer carries the certificate presented at that moment, so a switch is
//! announced at once. `Discovery:Enabled=false` turns it off.
//!
//! It answers over IPv4 and, when the web ports serve IPv6 too, over IPv6 on a socket of its own on
//! the same port, as Skakel 0.3.0's own answerer does. The IPv6 socket is IPv6 only, so on Linux it
//! never takes the IPv4 packets of the port, and it is best effort: a machine without IPv6 answers
//! over IPv4 alone.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use koppel_config::ConfigFile;
use koppel_config::files::{KoppelSettings, Role};
use skakel_wire::discovery::net::should_answer;
use skakel_wire::discovery::{DiscoveryAnswer, create_answer};
use skakel_wire::{SkakelApp, VERSIONS};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// What the answers say, but the fingerprint.
pub(crate) struct Announcement {
    pub name: String,
    pub port: u16,
    pub app: SkakelApp,
}

/// What an answering announcer holds.
struct Answering {
    stop: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
    addresses: Vec<SocketAddr>,
}

/// Answers on one UDP port while the role serves.
pub(crate) struct Announcer {
    /// The first socket, which must open, then the other family's, which may not.
    binds: Vec<IpAddr>,
    port: u16,
    announcement: Arc<Announcement>,
    fingerprint: Arc<dyn Fn() -> String + Send + Sync>,
    running: Mutex<Option<Answering>>,
}

impl Announcer {
    /// An announcer on UDP `port` for the web ports bound at `address`. An address the web ports
    /// bind to is answered at (a server that listens on loopback only has nobody else to answer);
    /// the unspecified address of one family, or none at all (the web ports' default, `::`, which
    /// serves both), is answered on every address of the families the web ports serve, which
    /// receiving broadcasts and the all-nodes group takes.
    pub(crate) fn new(
        address: Option<IpAddr>,
        port: u16,
        announcement: Announcement,
        fingerprint: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> Arc<Self> {
        let binds = match address {
            Some(IpAddr::V4(v4)) => vec![IpAddr::V4(v4)],
            Some(IpAddr::V6(v6)) if v6.is_unspecified() => {
                vec![IpAddr::V6(v6), IpAddr::V4(Ipv4Addr::UNSPECIFIED)]
            }
            Some(IpAddr::V6(v6)) => vec![IpAddr::V6(v6)],
            None => vec![
                IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            ],
        };
        Arc::new(Self {
            binds,
            port,
            announcement: Arc::new(announcement),
            fingerprint,
            running: Mutex::new(None),
        })
    }

    /// The addresses answering now, with their ports.
    #[cfg(test)]
    pub(crate) fn addresses(&self) -> Vec<SocketAddr> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|answering| answering.addresses.clone())
            .unwrap_or_default()
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
        let first = SocketAddr::new(self.binds[0], self.port);
        let primary = match bind(first) {
            Ok(socket) => socket,
            Err(e) => {
                // Another program holds the UDP port. Clients can still enter this server's address.
                tracing::warn!(
                    "Could not announce on the local network (UDP {first}: {e}); clients must enter this server's address"
                );
                return;
            }
        };
        let port = primary.local_addr().map_or(self.port, |a| a.port());
        let mut sockets = vec![primary];
        for ip in &self.binds[1..] {
            let address = SocketAddr::new(*ip, port);
            match bind(address) {
                Ok(socket) => sockets.push(socket),
                Err(e) => tracing::warn!(
                    "Discovery will not be answered on {address}: {e}; clients on that network family enter this server's address"
                ),
            }
        }
        let stop = CancellationToken::new();
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.is_some() {
            return;
        }
        let addresses: Vec<SocketAddr> =
            sockets.iter().filter_map(|s| s.local_addr().ok()).collect();
        tracing::info!(
            "Answering discovery on the local network as {} (UDP port {port}, {})",
            self.announcement.name,
            addresses
                .iter()
                .map(|a| if a.is_ipv4() { "IPv4" } else { "IPv6" })
                .collect::<Vec<_>>()
                .join(" and ")
        );
        let tasks = sockets
            .into_iter()
            .map(|socket| {
                tokio::spawn(answer(
                    socket,
                    self.announcement.clone(),
                    self.fingerprint.clone(),
                    stop.clone(),
                ))
            })
            .collect();
        *running = Some(Answering {
            stop,
            tasks,
            addresses,
        });
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
        if let Some(answering) = running {
            answering.stop.cancel();
            for task in answering.tasks {
                let _ = task.await;
            }
            tracing::debug!("Stopped answering discovery on {:?}", answering.addresses);
        }
    }
}

/// A UDP socket for `address`. An IPv6 one is IPv6 only, so it never shares the IPv4 packets of its
/// port on a system whose wildcard IPv6 socket would take them (Linux by default).
fn bind(address: SocketAddr) -> io::Result<UdpSocket> {
    let domain = if address.is_ipv6() {
        socket2::Domain::IPV6
    } else {
        socket2::Domain::IPV4
    };
    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
    if address.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    UdpSocket::from_std(socket.into())
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
                versions: VERSIONS.to_vec(),
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use skakel_wire::discovery::{create_probe, try_parse_answer};

    use super::*;

    const FINGERPRINT: &str = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";

    fn announcer(address: Option<IpAddr>) -> Arc<Announcer> {
        Announcer::new(
            address,
            0,
            Announcement {
                name: "bench".to_owned(),
                port: 5240,
                app: SkakelApp {
                    id: "koppel".to_owned(),
                    version: "0.2.1".to_owned(),
                },
            },
            Arc::new(|| FINGERPRINT.to_owned()),
        )
    }

    /// Sends a probe to `to` and reads the answer, from a socket of the same family.
    async fn ask(to: SocketAddr) -> Option<DiscoveryAnswer> {
        let local = if to.is_ipv6() {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
        };
        let socket = UdpSocket::bind(local).await.ok()?;
        socket.send_to(&create_probe(), to).await.ok()?;
        let mut buffer = [0u8; 2048];
        let (length, _) =
            tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
                .await
                .ok()?
                .ok()?;
        try_parse_answer(&buffer[..length])
    }

    #[tokio::test]
    async fn a_server_on_both_families_answers_a_probe_over_each() {
        let announcer = announcer(None);
        announcer.apply(Role::Server).await;
        let addresses = announcer.addresses();
        let v4 = addresses
            .iter()
            .find(|a| a.is_ipv4())
            .expect("an IPv4 socket");
        let Some(v6) = addresses.iter().find(|a| a.is_ipv6()) else {
            // A machine with IPv6 off (a container's default) answers over IPv4 alone.
            eprintln!("No IPv6 here: the IPv6 half of this test did not run");
            announcer.stop().await;
            return;
        };
        assert_eq!(v4.port(), v6.port(), "one port for both families");

        let over_v4 = ask(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), v4.port())).await;
        let over_v6 = ask(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), v6.port())).await;
        announcer.stop().await;

        let over_v4 = over_v4.expect("an answer over IPv4");
        let over_v6 = over_v6.expect("an answer over IPv6");
        assert_eq!(over_v4.name, "bench");
        assert_eq!(over_v6.port, 5240);
        assert_eq!(over_v4.fingerprint, over_v6.fingerprint);
    }

    #[tokio::test]
    async fn a_server_on_an_ipv4_address_does_not_open_an_ipv6_socket() {
        let announcer = announcer(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        announcer.apply(Role::Server).await;
        let addresses = announcer.addresses();
        announcer.stop().await;
        assert_eq!(addresses.len(), 1);
        assert!(addresses[0].is_ipv4());
    }

    #[tokio::test]
    async fn the_answers_follow_the_role_and_free_the_port() {
        let announcer = announcer(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        announcer.apply(Role::Client).await;
        assert!(announcer.addresses().is_empty());
        announcer.apply(Role::Both).await;
        assert!(!announcer.addresses().is_empty());
        let port = announcer.addresses()[0].port();
        announcer.apply(Role::Client).await;
        assert!(announcer.addresses().is_empty());
        // Free again: another socket takes it.
        UdpSocket::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .expect("the port is free");
    }

    /// The client's search, which is skakel-client's browser, hears an IPv6 answer and lists the
    /// server at its IPv6 address. Not on Windows: the browser listens on every address, which
    /// brings up a firewall prompt.
    #[cfg(not(windows))]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_finds_a_server_that_answers_over_ipv6() {
        use skakel_client::{Browser, BrowserOptions};

        let announcer = announcer(Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        announcer.apply(Role::Server).await;
        let Some(v6) = announcer.addresses().into_iter().find(|a| a.is_ipv6()) else {
            eprintln!("No IPv6 here: this test did not run");
            return;
        };
        let browser = Browser::new(
            BrowserOptions::new(v6.port())
                .app_ids(vec!["koppel".to_owned()])
                .probe_targets(vec![v6])
                .probe_interval(Duration::from_millis(200)),
        );
        browser.start().await.expect("the browser starts");
        let mut found = None;
        for _ in 0..40 {
            found = browser.servers().into_iter().next();
            if found.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        browser.stop();
        announcer.stop().await;
        let found = found.expect("the client found the server");
        assert!(found.address.is_ipv6(), "{}", found.address);
        assert_eq!(found.name, "bench");
        assert_eq!(found.port, 5240);
    }
}

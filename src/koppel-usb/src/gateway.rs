//! The client's plain USB/IP gateway, for a USB/IP driver on this machine.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koppel_config::files::{KoppelSettings, Role};
use koppel_config::{ConfigFile, Subscription};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::UsbClient;
use crate::peer_check::{AnyPeer, GatewayPeerCheck};
use crate::tasks::{BackgroundTasks, lock};

const STOP_WAIT: Duration = Duration::from_secs(5);
const MIN_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Where the gateway listens and how many it serves.
#[derive(Debug, Clone)]
pub struct GatewayOptions {
    /// `Gateway:Port`. 0 turns the gateway off; see [`GatewayOptions::default_port`].
    pub port: u16,
    /// `Gateway:Address`, `127.0.0.1` unless set. Must be an IP address.
    pub address: String,
    /// `Gateway:MaxSessions`, 64 unless set. At least one is always served.
    pub max_sessions: usize,
}

impl GatewayOptions {
    /// On Windows the gateway is how Koppel's own attach reaches the driver, so it is on by
    /// default.
    pub const WINDOWS_DEFAULT_PORT: u16 = 3242;
    pub const DEFAULT_MAX_SESSIONS: usize = 64;

    /// 3242 on Windows, off elsewhere.
    pub fn default_port() -> u16 {
        if cfg!(windows) {
            Self::WINDOWS_DEFAULT_PORT
        } else {
            0
        }
    }
}

impl Default for GatewayOptions {
    fn default() -> Self {
        Self {
            port: Self::default_port(),
            address: "127.0.0.1".to_owned(),
            max_sessions: Self::DEFAULT_MAX_SESSIONS,
        }
    }
}

/// Client role: a plain USB/IP server for a USB/IP driver on this machine, such as usbip-win2.
/// Koppel on Windows drives that driver itself and points it here; Koppel in a container on
/// Windows leaves it to the host's `usbip attach`. Each connection is carried to the server on a
/// stream of the client's link, so the network only sees encrypted traffic. It runs only while
/// the role is client or both.
///
/// Anyone who reaches this port gets this client's access to the server, with no login. In a
/// container, listen on all addresses and publish the port on the host's 127.0.0.1 only. On
/// native Windows with a loopback address, only connections the kernel owns (PID 4, the driver)
/// are served, and of those only the imports Koppel itself asked the driver for. At most
/// `max_sessions` connections are served at once; more are closed at once. A port that cannot be
/// bound is tried again with a wait that grows from 5 to 60 seconds, and the reason shows in the
/// client's device list.
pub struct UsbIpGateway {
    inner: Arc<Inner>,
}

struct Inner {
    client: UsbClient,
    settings: ConfigFile<KoppelSettings>,
    options: GatewayOptions,
    peer_check: Mutex<Option<Arc<dyn GatewayPeerCheck>>>,
    stopping: CancellationToken,
    /// Cancelled when the role starts or stops wanting the gateway; a child of `stopping`.
    role_changed: Mutex<CancellationToken>,
    running: AtomicBool,
    tasks: BackgroundTasks,
    run: Mutex<Option<JoinHandle<()>>>,
    subscription: Mutex<Option<Subscription>>,
}

impl UsbIpGateway {
    /// A gateway for `client`. `peer_check` decides which connections are served; `None` picks
    /// the kernel check on native Windows with a loopback address, and serves everyone elsewhere.
    pub fn new(
        client: UsbClient,
        settings: ConfigFile<KoppelSettings>,
        options: GatewayOptions,
        peer_check: Option<Arc<dyn GatewayPeerCheck>>,
    ) -> Self {
        let stopping = CancellationToken::new();
        Self {
            inner: Arc::new(Inner {
                client,
                settings,
                options,
                peer_check: Mutex::new(peer_check),
                role_changed: Mutex::new(stopping.child_token()),
                stopping,
                running: AtomicBool::new(false),
                tasks: BackgroundTasks::default(),
                run: Mutex::new(None),
                subscription: Mutex::new(None),
            }),
        }
    }

    /// Starts the gateway, unless its port is 0. A bad address turns it off and says why in the
    /// client's device list.
    pub async fn start(&self) {
        let inner = &self.inner;
        let port = inner.options.port;
        if port == 0 {
            return;
        }
        let Ok(address) = inner.options.address.parse::<IpAddr>() else {
            tracing::error!(
                "Gateway:Address {} is not an IP address; the gateway is off",
                inner.options.address
            );
            inner
                .client
                .set_gateway(
                    None,
                    Some(format!(
                        "Gateway:Address \"{}\" is not an IP address.",
                        inner.options.address
                    )),
                )
                .await;
            return;
        };
        let check = {
            let mut chosen = lock(&inner.peer_check);
            chosen
                .get_or_insert_with(|| choose_peer_check(address))
                .clone()
        };

        let weak = Arc::downgrade(inner);
        let subscription = inner.settings.subscribe(move |_| {
            let weak = weak.clone();
            async move {
                // Wake the loop when the role starts or stops wanting the gateway.
                if let Some(inner) = weak.upgrade()
                    && inner.wanted() != inner.running.load(Ordering::SeqCst)
                {
                    let next = inner.stopping.child_token();
                    std::mem::replace(&mut *lock(&inner.role_changed), next).cancel();
                }
            }
        });
        *lock(&inner.subscription) = Some(subscription);

        let slots = Arc::new(Semaphore::new(inner.options.max_sessions.max(1)));
        let running = inner.clone();
        let handle = tokio::spawn(async move { running.run(address, port, check, slots).await });
        *lock(&inner.run) = Some(handle);
    }

    /// Stops listening and ends the sessions.
    pub async fn stop(&self) {
        let inner = &self.inner;
        if let Some(subscription) = lock(&inner.subscription).take() {
            inner.settings.unsubscribe(subscription);
        }
        inner.stopping.cancel();
        let run = lock(&inner.run).take();
        if let Some(run) = run {
            let _ = run.await;
        }
    }
}

/// Native Windows with a loopback gateway: only the kernel may connect. Elsewhere, such as the
/// Linux container of Docker Desktop, the connection arrives from outside this machine's process
/// table, so the check does not apply and the gateway's port publishing is what keeps it local.
fn choose_peer_check(address: IpAddr) -> Arc<dyn GatewayPeerCheck> {
    #[cfg(windows)]
    if address.is_loopback() {
        use crate::peer_check::{KernelPeer, KernelPeerCheck};
        if address.is_ipv6() {
            tracing::warn!(
                "The gateway on {address} cannot tell the driver from other processes on an IPv6 address; it refuses every connection. Use 127.0.0.1"
            );
        }
        return Arc::new(KernelPeer(KernelPeerCheck::new(Arc::new(
            crate::windows::WindowsTcpTable,
        ))));
    }
    tracing::info!(
        "The gateway on {address} accepts every connection: the check for the driver's process only applies to a loopback address on Windows"
    );
    Arc::new(AnyPeer)
}

impl Inner {
    fn wanted(&self) -> bool {
        self.settings.current().role != Role::Server
    }

    async fn run(
        self: Arc<Self>,
        address: IpAddr,
        port: u16,
        check: Arc<dyn GatewayPeerCheck>,
        slots: Arc<Semaphore>,
    ) {
        let mut backoff = MIN_BACKOFF;
        while !self.stopping.is_cancelled() {
            let iteration = lock(&self.role_changed).clone();
            if !self.wanted() {
                self.client.set_gateway(None, None).await;
                iteration.cancelled().await;
                backoff = MIN_BACKOFF;
                continue;
            }

            self.running.store(true, Ordering::SeqCst);
            if !self.wanted() {
                self.running.store(false, Ordering::SeqCst);
                continue;
            }
            let listened = self
                .listen(SocketAddr::new(address, port), &check, &slots, &iteration)
                .await;
            match listened {
                Ok(()) => backoff = MIN_BACKOFF,
                Err(e) => {
                    // A port that is taken or reserved, or anything else: the host keeps running,
                    // the UI says why.
                    tracing::warn!(
                        "The USB/IP gateway could not listen on {address}:{port}; it will try again: {e}"
                    );
                    self.client
                        .set_gateway(
                            None,
                            Some(format!(
                                "The gateway could not listen on {address}:{port}: {e}"
                            )),
                        )
                        .await;
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = iteration.cancelled() => {}
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
            self.running.store(false, Ordering::SeqCst);
        }

        // Sessions end with the token; give them a moment to let go of their sockets.
        self.tasks.wait(STOP_WAIT).await;
        self.client.set_gateway(None, None).await;
    }

    /// Listens until `iteration` ends. An error means the port could not be bound or accepting
    /// failed.
    async fn listen(
        &self,
        gateway: SocketAddr,
        check: &Arc<dyn GatewayPeerCheck>,
        slots: &Arc<Semaphore>,
        iteration: &CancellationToken,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(gateway).await?;
        tracing::info!("USB/IP gateway listening on {gateway}");
        self.client.set_gateway(Some(gateway.port()), None).await;
        let accepted = loop {
            let (socket, peer) = tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok(accepted) => accepted,
                    Err(e) => break Err(e),
                },
                _ = iteration.cancelled() => break Ok(()),
            };
            let _ = socket.set_nodelay(true);
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                tracing::warn!(
                    "Refused a gateway connection from {peer}: too many sessions are open"
                );
                drop(socket);
                continue;
            };
            let client = self.client.clone();
            let check = check.clone();
            let session = iteration.clone();
            self.tasks.spawn("gateway session", async move {
                serve(client, socket, peer, gateway, check, session).await;
                drop(slot);
            });
        };
        self.client.set_gateway(None, None).await;
        accepted
    }
}

async fn serve(
    client: UsbClient,
    socket: TcpStream,
    peer: SocketAddr,
    gateway: SocketAddr,
    check: Arc<dyn GatewayPeerCheck>,
    stopping: CancellationToken,
) {
    let allowed = tokio::select! {
        allowed = check.allows(peer, gateway) => allowed,
        _ = stopping.cancelled() => return,
    };
    if !allowed {
        tracing::warn!(
            "Refused a gateway connection from {peer}: it is not the USB/IP driver (the kernel)"
        );
        return;
    }
    client.serve_gateway(socket, peer, &stopping).await;
}

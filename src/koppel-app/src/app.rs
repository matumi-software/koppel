//! Everything the roles hold, started from the config files as 0.1.x starts them: the server and
//! client roles on Skakel and the USB layer behind them, the activity log and the push bus. The web
//! server (`koppel-web`) puts the sign-in, the TLS ports and the routes on top of it.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use koppel_config::files::{
    AttachedDevices, ClientConnection, KoppelSettings, ServerSecrets, SharedDevices,
};
use koppel_config::{ConfigFile, ConfigFileError, KoppelPaths, SecretBox, open};
use koppel_usb::{ActivitySink, GatewayOptions, Platform, SystemClock, UsbDataPlane, UsbIpGateway};
use skakel_client::{Browser, BrowserOptions};
use skakel_wire::SkakelApp;
use tokio::sync::watch;

use crate::activity::ActivityLog;
use crate::certificate::CertificateSlot;
use crate::connection::ConnectionView;
use crate::host::Host;
use crate::push::{Event, Push};
use crate::roles::skakel::announcer::{Announcement, Announcer};
use crate::roles::skakel::{
    APP_ID, SecretsError, SecretsStore, ServerParts, ServerStartError, SkakelLinks,
};
use crate::roles::usb_store;
use crate::roles::{SkakelClientRole, SkakelServerRole, UsbActivity, UsbClientRole, UsbServerRole};
use crate::tasks::{STOP_WAIT, Tasks};

/// What the roles are started with.
pub struct AppOptions {
    pub paths: KoppelPaths,
    pub secrets: Arc<SecretBox>,
    pub host: Host,
    /// Discovery, and Skakel's limits.
    pub network: NetworkOptions,
    /// What this machine's USB is.
    pub usb: UsbOptions,
}

/// Discovery and the limits on Skakel's token and pairing calls.
#[derive(Debug, Clone)]
pub struct NetworkOptions {
    /// `Ports:Address`: the address the web ports bind to. Discovery answers there too when it is
    /// an IPv4 address.
    pub address: Option<IpAddr>,
    /// `Discovery:Enabled`.
    pub discovery: bool,
    /// `Discovery:Port`: the UDP port a server answers probes on and a client sends them to.
    pub discovery_port: u16,
    /// Where a client sends its probes instead of broadcasting, such as loopback in a test.
    pub probe_targets: Option<Vec<SocketAddr>>,
    /// `RateLimits:TokenPerMinute`.
    pub token_per_minute: u32,
    /// `RateLimits:PairPerMinute`.
    pub pair_per_minute: u32,
}

impl NetworkOptions {
    /// 0.1.x's defaults for a machine whose network port is `port`.
    pub fn defaults(port: u16) -> Self {
        Self {
            address: None,
            discovery: true,
            discovery_port: port,
            probe_targets: None,
            token_per_minute: 60,
            pair_per_minute: 120,
        }
    }
}

/// What shares and attaches USB devices on this machine, and the client's timings.
pub struct UsbOptions {
    /// What shares and attaches: the Linux kernel, a USB/IP server such as usbipd-win, a USB/IP
    /// driver such as usbip-win2, or none of them.
    pub platform: Platform,
    pub client: koppel_usb::ClientOptions,
    pub gateway: GatewayOptions,
}

impl UsbOptions {
    /// No USB at all, and why, as on a platform Koppel cannot drive.
    pub fn unsupported(reason: &str) -> Self {
        Self {
            platform: Platform::unsupported(reason),
            client: koppel_usb::ClientOptions::default(),
            gateway: GatewayOptions {
                port: 0,
                ..GatewayOptions::default()
            },
        }
    }
}

/// Why the roles could not start. The message names the file or the setting and what to do.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    File(#[from] ConfigFileError),
    /// `devices.json` or `attached.json`, as koppel-usb loads them.
    #[error(transparent)]
    Usb(#[from] koppel_usb::StoreError),
    /// `secrets.json` held nothing the server could start from.
    #[error(transparent)]
    Secrets(#[from] SecretsError),
    #[error(transparent)]
    Skakel(#[from] ServerStartError),
}

/// The config files the app opened, which stop watching their folders when Koppel stops.
struct Files {
    settings: ConfigFile<KoppelSettings>,
    server_secrets: ConfigFile<ServerSecrets>,
    connection: ConfigFile<ClientConnection>,
    devices: ConfigFile<SharedDevices>,
    attached: ConfigFile<AttachedDevices>,
}

impl Files {
    fn stop(&self) {
        self.settings.stop();
        self.server_secrets.stop();
        self.connection.stop();
        self.devices.stop();
        self.attached.stop();
    }
}

/// What runs behind the roles, stopped in order when Koppel stops.
struct Running {
    skakel: Arc<SkakelServerRole>,
    announcer: Option<Arc<Announcer>>,
    browser: Option<Browser>,
    usb_server: koppel_usb::UsbServer,
    data_plane: UsbDataPlane,
    usb_client: koppel_usb::UsbClient,
    gateway: UsbIpGateway,
}

/// The roles: the config files, the activity log, the push bus, Skakel's server and client and USB.
pub struct App {
    pub host: Host,
    pub settings: ConfigFile<KoppelSettings>,
    pub connection: ConfigFile<ClientConnection>,
    pub activity: Arc<ActivityLog>,
    pub push: Push,
    pub server: Arc<SkakelServerRole>,
    pub client: Arc<SkakelClientRole>,
    pub usb_server: Arc<UsbServerRole>,
    pub usb_client: Arc<UsbClientRole>,
    certificate: Arc<CertificateSlot>,
    running: Running,
    /// The config files, stopped last.
    files: Files,
    tasks: Tasks,
}

impl App {
    /// Opens every config file as 0.1.x opens it, makes the server certificate on a first start,
    /// starts Skakel's server, the USB layer and the client's link, and wires the pushes. Run after
    /// the folders are prepared and 0.1.x's secrets converted.
    pub async fn start(options: AppOptions) -> Result<Arc<Self>, StartError> {
        let AppOptions {
            paths,
            secrets,
            host,
            network,
            usb,
        } = options;
        let push = Push::new();
        let tasks = Tasks::new();

        let settings = open::settings(&paths);
        settings.start().await?;
        let server_secrets = open::server_secrets(&paths, &secrets);
        server_secrets.start().await?;
        let connection = open::connection(&paths, &secrets);
        connection.start().await?;
        // koppel-usb starts these two itself.
        let devices = open::devices(&paths);
        let attached = open::attached(&paths);

        {
            let push = push.clone();
            settings.subscribe(move |value| {
                push.send(Event::SettingsChanged(&value));
                async {}
            });
        }
        {
            let push = push.clone();
            connection.subscribe(move |value| {
                push.send(Event::ConnectionChanged(&ConnectionView::of(&value)));
                async {}
            });
        }

        let activity = Arc::new(ActivityLog::new(push.clone()));
        let usb_activity: Arc<dyn ActivitySink> = Arc::new(UsbActivity(activity.clone()));
        let clock = Arc::new(SystemClock);

        // The server side: USB, then Skakel's server over secrets.json with the USB streams and the
        // remote device list.
        let store = SecretsStore::open(server_secrets.clone(), &host.name).await?;
        let certificate = store.slot();
        let usb_server = koppel_usb::UsbServer::new(koppel_usb::ServerDeps {
            host: usb.platform.host.clone(),
            shared: usb_store::shared(&devices),
            settings: usb_store::settings(&settings),
            activity: usb_activity.clone(),
            clock: clock.clone(),
        });
        let data_plane = UsbDataPlane::new(
            usb_server.clone(),
            usb_activity.clone(),
            usb_store::settings(&settings),
        );
        let (remote, remote_view) = watch::channel(Arc::new(usb_server.current().for_remote()));
        usb_server.on_changed(move |devices| {
            remote.send_replace(Arc::new(devices.for_remote()));
            async {}
        });
        let skakel = SkakelServerRole::start(ServerParts {
            name: host.name.clone(),
            version: host.version.clone(),
            store,
            devices: remote_view,
            data_plane: data_plane.clone(),
            push: push.clone(),
            activity: activity.clone(),
            listen: SocketAddr::new(
                network.address.unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
                host.network_port,
            ),
            token_per_minute: network.token_per_minute,
            pair_per_minute: network.pair_per_minute,
            tasks: tasks.clone(),
        })
        .await?;
        skakel.follow(&settings);
        let own_fingerprint: Arc<dyn Fn() -> String + Send + Sync> = {
            let skakel = skakel.clone();
            Arc::new(move || skakel.fingerprint())
        };
        let announcer = if network.discovery {
            let announcer = Announcer::new(
                network.address,
                network.discovery_port,
                Announcement {
                    name: host.name.clone(),
                    port: host.network_port,
                    app: SkakelApp {
                        id: APP_ID.to_owned(),
                        version: host.version.clone(),
                    },
                },
                own_fingerprint.clone(),
            );
            announcer.follow(&settings).await;
            Some(announcer)
        } else {
            None
        };

        // The client side: the browser, the link on Skakel's client, USB, the gateway.
        let browser = network.discovery.then(|| {
            let mut options =
                BrowserOptions::new(network.discovery_port).app_ids(vec![APP_ID.to_owned()]);
            if let Some(targets) = network.probe_targets.clone() {
                options = options.probe_targets(targets);
            }
            Browser::new(options)
        });
        let links = SkakelLinks::new(
            connection.clone(),
            browser.clone(),
            activity.clone(),
            tasks.clone(),
        );
        let usb_client = koppel_usb::UsbClient::new(koppel_usb::Deps {
            settings: usb_store::settings(&settings),
            connection: usb_store::connection(&connection),
            remembered: usb_store::attached(&attached),
            platform: usb.platform.clone(),
            links,
            activity: usb_activity.clone(),
            clock,
            options: usb.client,
        });
        let gateway = UsbIpGateway::new(
            usb_client.clone(),
            usb_store::settings(&settings),
            usb.gateway,
            None,
        );
        let client = SkakelClientRole::new(
            connection.clone(),
            push.clone(),
            browser.clone(),
            own_fingerprint,
            host.name.clone(),
            tasks.clone(),
        );
        client.follow(&settings).await;

        let usb_server_role =
            UsbServerRole::new(usb_server.clone(), usb.platform.clone(), push.clone());
        let usb_client_role =
            UsbClientRole::new(usb_client.clone(), usb.platform.clone(), push.clone());
        usb_server.start().await?;
        data_plane.start();
        usb_client.start().await?;
        gateway.start().await;

        let files = Files {
            settings: settings.clone(),
            server_secrets,
            connection: connection.clone(),
            devices,
            attached,
        };

        Ok(Arc::new(Self {
            host,
            activity,
            server: skakel.clone(),
            client,
            usb_server: usb_server_role,
            usb_client: usb_client_role,
            settings,
            connection,
            push,
            certificate,
            running: Running {
                skakel,
                announcer,
                browser,
                usb_server,
                data_plane,
                usb_client,
                gateway,
            },
            files,
            tasks,
        }))
    }

    /// The certificate the web ports present, which a certificate switch replaces.
    pub fn certificate(&self) -> Arc<CertificateSlot> {
        self.certificate.clone()
    }

    /// Skakel's server's own TLS acceptor, when it is running.
    pub fn skakel_acceptor(&self) -> Option<tokio_rustls::TlsAcceptor> {
        self.running.skakel.acceptor()
    }

    /// Skakel's server, which the network router hands its requests to.
    pub fn skakel(&self) -> Arc<SkakelServerRole> {
        self.running.skakel.clone()
    }

    /// Stops everything in the reverse of the order it started: the client's link and devices
    /// (they stay remembered), the data plane and the server's polling, discovery, and Skakel's
    /// server, which tells its clients it is stopping. Then stops watching the config files.
    pub async fn stop(&self) {
        let running = &self.running;
        running.gateway.stop().await;
        running.usb_client.stop().await;
        running.data_plane.stop().await;
        running.usb_server.stop().await;
        if let Some(announcer) = &running.announcer {
            announcer.stop().await;
        }
        if let Some(browser) = &running.browser {
            browser.stop();
        }
        running.skakel.stop().await;
        // Whatever Koppel's own tasks still hold ends here, within a limit.
        self.tasks.shutdown(STOP_WAIT).await;
        self.files.stop();
    }
}

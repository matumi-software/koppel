//! Everything the UI's server side holds, started from the config files as 0.1.x starts them, with
//! the server and client roles on Skakel and the USB layer behind them.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::Method;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use koppel_config::files::{ClientConnection, KoppelSettings};
use koppel_config::{ConfigFile, ConfigFileError, KoppelPaths, SecretBox, open};
use koppel_usb::{
    ActivitySink, GatewayOptions, SystemClock, Unsupported, UsbAttacher, UsbDataPlane, UsbHost,
    UsbIpDriver, UsbIpGateway,
};
use skakel_client::{Browser, BrowserOptions};
use skakel_protocol::SkakelApp;
use tokio::sync::watch;

use crate::access::{UiAccess, UiReach};
use crate::activity::ActivityLog;
use crate::api::{self, client::ConnectionView};
use crate::auth::UiAuth;
use crate::auth::ticket::CookieKey;
use crate::guard::{self, RateLimit};
use crate::host::Host;
use crate::push::{self, Push};
use crate::roles::skakel::announcer::{Announcement, Announcer};
use crate::roles::skakel::front::{self, Front, FrontConnection, SkakelLimits};
use crate::roles::skakel::{APP_ID, SecretsStore, ServerParts, SkakelLinks};
use crate::roles::{
    ClientRole, ServerRole, SkakelClientRole, SkakelServerRole, UsbActivity, UsbClient,
    UsbClientRole, UsbServer, UsbServerRole,
};
use crate::surface::starts_with_segments;
use crate::tls::CertificateSlot;

/// What the app is started with.
pub struct AppOptions {
    pub paths: KoppelPaths,
    pub secrets: Arc<SecretBox>,
    pub host: Host,
    /// `Ui:AllowFrom`.
    pub reach: UiReach,
    /// `Ui:SetupWindowMinutes`.
    pub setup_window: Duration,
    /// `RateLimits:AuthPerMinute`.
    pub auth_per_minute: u32,
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
    /// What the server shares from: the Linux kernel, a USB/IP server such as usbipd-win, or none.
    pub host: Arc<dyn UsbHost>,
    /// What attaches through this machine's kernel (vhci-hcd), or none.
    pub attacher: Arc<dyn UsbAttacher>,
    /// A USB/IP driver on this machine Koppel drives (usbip-win2), or none.
    pub driver: Arc<dyn UsbIpDriver>,
    pub client: koppel_usb::ClientOptions,
    pub gateway: GatewayOptions,
}

impl UsbOptions {
    /// No USB at all, and why, as on a platform Koppel cannot drive.
    pub fn unsupported(reason: &str) -> Self {
        let none = Arc::new(Unsupported::new(reason));
        Self {
            host: none.clone(),
            attacher: none.clone(),
            driver: none,
            client: koppel_usb::ClientOptions::default(),
            gateway: GatewayOptions {
                port: 0,
                ..GatewayOptions::default()
            },
        }
    }
}

/// Why the app could not start. The message names the file or the setting and what to do.
#[derive(Debug)]
pub enum StartError {
    File(ConfigFileError),
    Other(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(e) => e.fmt(f),
            Self::Other(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for StartError {}

impl From<ConfigFileError> for StartError {
    fn from(e: ConfigFileError) -> Self {
        Self::File(e)
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

/// The UI's server side: the config files, sign-in, the activity log, the push and the roles.
pub struct App {
    pub(crate) host: Host,
    pub(crate) settings: ConfigFile<KoppelSettings>,
    pub(crate) connection: ConfigFile<ClientConnection>,
    pub(crate) auth: Arc<UiAuth>,
    pub(crate) access: UiAccess,
    pub(crate) auth_limit: RateLimit,
    pub(crate) skakel_limits: SkakelLimits,
    pub(crate) activity: Arc<ActivityLog>,
    pub(crate) push: Push,
    pub(crate) server: Arc<dyn ServerRole>,
    pub(crate) client: Arc<dyn ClientRole>,
    pub(crate) usb_server: Arc<dyn UsbServer>,
    pub(crate) usb_client: Arc<dyn UsbClient>,
    pub(crate) front: Arc<Front>,
    certificate: Arc<CertificateSlot>,
    running: Running,
    stoppers: Vec<Box<dyn Fn() + Send + Sync>>,
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
            reach,
            setup_window,
            auth_per_minute,
            network,
            usb,
        } = options;
        let push = Push::new();

        let settings = open::settings(&paths);
        settings.start().await?;
        let ui_auth = open::ui_auth(&paths, &secrets);
        ui_auth.start().await?;
        delete_old_setup_code(&paths);
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
                push.send(push::SETTINGS_CHANGED, &*value);
                async {}
            });
        }
        {
            let push = push.clone();
            connection.subscribe(move |value| {
                push.send(push::CONNECTION_CHANGED, &ConnectionView::of(&value));
                async {}
            });
        }

        let activity = Arc::new(ActivityLog::new(push.clone()));
        let usb_activity: Arc<dyn ActivitySink> = Arc::new(UsbActivity(activity.clone()));
        let clock = Arc::new(SystemClock);

        // The server side: USB, then Skakel's server over secrets.json with the USB streams and the
        // remote device list.
        let store = SecretsStore::open(server_secrets.clone(), &host.name)
            .await
            .map_err(StartError::Other)?;
        let certificate = store.slot();
        let usb_server = koppel_usb::UsbServer::new(
            usb.host.clone(),
            devices.clone(),
            settings.clone(),
            usb_activity.clone(),
            clock.clone(),
        );
        let data_plane =
            UsbDataPlane::new(usb_server.clone(), usb_activity.clone(), settings.clone());
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
        })
        .await
        .map_err(StartError::Other)?;
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
            Browser::with_options(BrowserOptions {
                app_ids: Some(vec![APP_ID.to_owned()]),
                probe_targets: network.probe_targets.clone(),
                ..BrowserOptions::new(network.discovery_port)
            })
        });
        let links = SkakelLinks::new(connection.clone(), browser.clone(), activity.clone());
        let usb_client = koppel_usb::UsbClient::new(
            settings.clone(),
            connection.clone(),
            attached.clone(),
            usb.attacher.clone(),
            usb.driver.clone(),
            links,
            usb_activity.clone(),
            clock,
            usb.client,
        );
        let gateway = UsbIpGateway::new(usb_client.clone(), settings.clone(), usb.gateway, None);
        let client = SkakelClientRole::new(
            connection.clone(),
            push.clone(),
            browser.clone(),
            own_fingerprint,
            host.name.clone(),
        );
        client.follow(&settings).await;

        let usb_server_role = UsbServerRole::new(usb_server.clone(), push.clone());
        let usb_client_role = UsbClientRole::new(
            usb_client.clone(),
            usb.attacher.clone(),
            usb.driver.clone(),
            push.clone(),
        );
        usb_server.start().await?;
        data_plane.start();
        usb_client.start().await?;
        gateway.start().await;

        let key = CookieKey::load(&paths.keys, &secrets).map_err(StartError::Other)?;
        let address = format!("https://{}:{}", host.name, host.ui_port);
        let auth = UiAuth::new(ui_auth.clone(), setup_window, address, key);

        let stoppers: Vec<Box<dyn Fn() + Send + Sync>> = vec![
            Box::new({
                let f = settings.clone();
                move || f.stop()
            }),
            Box::new(move || ui_auth.stop()),
            Box::new(move || server_secrets.stop()),
            Box::new({
                let f = connection.clone();
                move || f.stop()
            }),
            Box::new(move || devices.stop()),
            Box::new(move || attached.stop()),
        ];

        Ok(Arc::new(Self {
            host,
            access: UiAccess::new(reach),
            auth_limit: RateLimit::per_minute(auth_per_minute),
            skakel_limits: SkakelLimits::new(network.token_per_minute, network.pair_per_minute),
            activity,
            server: skakel.clone(),
            client,
            usb_server: usb_server_role,
            usb_client: usb_client_role,
            front: skakel.front(),
            settings,
            connection,
            auth,
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
            stoppers,
        }))
    }

    /// The certificate the web ports present, which a certificate switch replaces.
    pub fn certificate(&self) -> Arc<CertificateSlot> {
        self.certificate.clone()
    }

    /// Every route, with the guards in front of them.
    pub fn router(self: &Arc<Self>) -> Router {
        api::routes()
            .route("/api/remote/devices", get(forward))
            .route("/healthz", get(crate::healthz))
            .fallback(fallback)
            .layer(middleware::from_fn_with_state(self.clone(), guard::guard))
            .with_state(self.clone())
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
            announcer.stop();
        }
        if let Some(browser) = &running.browser {
            browser.stop();
        }
        running.skakel.stop().await;
        for stop in &self.stoppers {
            stop();
        }
    }
}

/// A path no route took: Skakel's (a gRPC call, or under `/skakel`), the UI's files for GET and
/// HEAD, and 404 for anything else. An unknown path under `/api` is 404 too, where 0.1.2 answered a
/// signed-in GET of one with index.html. The guard in front already decided the request may reach
/// what it asks for.
async fn fallback(State(app): State<Arc<App>>, request: Request) -> Response {
    if front::is_grpc(&request) || starts_with_segments(request.uri().path(), "/skakel") {
        return forward(State(app), request).await;
    }
    if request.method() == Method::GET || request.method() == Method::HEAD {
        let (parts, _) = request.into_parts();
        crate::ui::serve(parts.uri, parts.headers).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Hands a request to Skakel's server, on the loopback connection of the connection it came on.
async fn forward(State(app): State<Arc<App>>, request: Request) -> Response {
    let remote = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0)
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 0)));
    let connection = request
        .extensions()
        .get::<Arc<FrontConnection>>()
        .cloned()
        .unwrap_or_default();
    app.front.forward(&connection, remote, request).await
}

/// Earlier versions wrote a setup code to `setup-code.txt`. It does nothing now, so it should not lie
/// around.
fn delete_old_setup_code(paths: &KoppelPaths) {
    let file = paths.config.join("setup-code.txt");
    match std::fs::remove_file(&file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(
            "Could not delete {}, left by an older version ({e}). The setup code in it no longer does anything.",
            file.display()
        ),
    }
}

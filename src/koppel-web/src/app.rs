//! The web server's app: the roles of `koppel-app` with the sign-in, the access rules, the rate limit
//! and the TLS ports on top of them, and the routers that serve it all.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use koppel_app::tray::TrayFeed;
use koppel_app::{CertificateSlot, Host};
use koppel_config::files::{Role, UiCredentials};
use koppel_config::{ConfigFile, ConfigFileError, KoppelPaths, SecretBox, open};

use crate::WebError;
use crate::access::{UiAccess, UiReach};
use crate::auth::UiAuth;
use crate::guard::RateLimit;
use crate::routes::Routers;
use crate::tls::{self, Acceptors};

pub use koppel_app::{NetworkOptions, UsbOptions};

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

/// Why the app could not start. The message names the file or the setting and what to do.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    File(#[from] ConfigFileError),
    /// The roles could not start.
    #[error(transparent)]
    Roles(#[from] koppel_app::StartError),
    #[error(transparent)]
    Tls(#[from] WebError),
}

/// The UI's server side: the roles, sign-in, and the TLS ports.
pub struct App {
    pub(crate) core: Arc<koppel_app::App>,
    pub(crate) auth: Arc<UiAuth>,
    pub(crate) access: Arc<UiAccess>,
    pub(crate) auth_limit: Arc<RateLimit>,
    /// The web ports' acceptor while Skakel's server does not present the certificate.
    own_acceptor: tokio_rustls::TlsAcceptor,
    /// `ui-auth.json`, stopped with the roles' files.
    ui_auth_file: ConfigFile<UiCredentials>,
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
        let ui_auth = open::ui_auth(&paths, &secrets);
        ui_auth.start().await?;
        delete_old_setup_code(&paths);
        let core = koppel_app::App::start(koppel_app::AppOptions {
            paths: paths.clone(),
            secrets,
            host: host.clone(),
            network,
            usb,
        })
        .await;
        let core = match core {
            Ok(core) => core,
            Err(e) => {
                ui_auth.stop();
                return Err(e.into());
            }
        };
        let own_acceptor = match tls::acceptor(core.certificate()) {
            Ok(acceptor) => acceptor,
            Err(e) => {
                core.stop().await;
                ui_auth.stop();
                return Err(e.into());
            }
        };

        crate::auth::remove_session_key(&paths.keys);
        let address = format!("https://{}:{}", host.name, host.ui_port);
        // A native Windows install is local only and a service has no console to read a token from,
        // so there the first caller within Ui:AllowFrom still sets the password.
        let setup_token_required = !cfg!(windows) && reach != UiReach::Local;
        let auth = UiAuth::new(ui_auth.clone(), setup_window, address, setup_token_required);

        Ok(Arc::new(Self {
            core,
            access: Arc::new(UiAccess::new(reach)),
            auth_limit: Arc::new(RateLimit::per_minute(auth_per_minute)),
            auth,
            own_acceptor,
            ui_auth_file: ui_auth,
        }))
    }

    /// The roles under the web server.
    pub fn roles(&self) -> &Arc<koppel_app::App> {
        &self.core
    }

    /// The certificate the web ports present, which a certificate switch replaces.
    pub fn certificate(&self) -> Arc<CertificateSlot> {
        self.core.certificate()
    }

    /// What the web ports shake hands with, picked for each new connection. While the role is
    /// server or both, Skakel's server's own TLS settings, which present its current certificate
    /// on every handshake as its certificate handover needs; after a restart (the role left the
    /// server side), the new server's. Otherwise Koppel's own acceptor on [`App::certificate`],
    /// which a save of `secrets.json` keeps on the same certificate.
    pub fn tls(&self) -> Acceptors {
        let core = self.core.clone();
        let own = self.own_acceptor.clone();
        Acceptors::new(move || {
            if core.settings.current().role != Role::Client
                && let Some(acceptor) = core.skakel_acceptor()
            {
                return acceptor;
            }
            own.clone()
        })
    }

    /// The feed the Windows service reads for its trays: the roles' state and the sign-in's.
    pub fn tray_feed(&self) -> TrayFeed {
        TrayFeed::new(&self.core, self.auth.clone())
    }

    /// The router of each surface, with its guards. When `Ports:Network` sets the network port apart,
    /// each is served on its own listener.
    pub fn routers(self: &Arc<Self>) -> Routers {
        Routers::of(self)
    }

    /// Both surfaces in one router, for one listener: the UI and network ports are the same port.
    pub fn router(self: &Arc<Self>) -> Router {
        self.routers().merged()
    }

    /// Stops the roles in the reverse of the order they started (see `koppel_app::App::stop`), then
    /// stops watching `ui-auth.json`.
    pub async fn stop(&self) {
        self.core.stop().await;
        self.ui_auth_file.stop();
    }
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

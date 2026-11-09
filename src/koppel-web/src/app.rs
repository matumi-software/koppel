//! Everything the UI's server side holds, started from the config files as 0.1.x starts them.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::Method;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use koppel_config::files::{ClientConnection, KoppelSettings};
use koppel_config::{ConfigFile, ConfigFileError, KoppelPaths, SecretBox, open};

use crate::access::{UiAccess, UiReach};
use crate::activity::ActivityLog;
use crate::api::{self, client::ConnectionView};
use crate::auth::UiAuth;
use crate::auth::ticket::CookieKey;
use crate::guard::{self, RateLimit};
use crate::host::Host;
use crate::push::{self, Push};
use crate::roles::{
    ClientRole, ServerRole, StoredServer, StubClient, StubUsbClient, StubUsbServer, UsbClient,
    UsbServer,
};
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

/// The UI's server side: the config files, sign-in, the activity log, the push and the roles.
pub struct App {
    pub(crate) host: Host,
    pub(crate) settings: ConfigFile<KoppelSettings>,
    pub(crate) connection: ConfigFile<ClientConnection>,
    pub(crate) auth: Arc<UiAuth>,
    pub(crate) access: UiAccess,
    pub(crate) auth_limit: RateLimit,
    pub(crate) activity: Arc<ActivityLog>,
    pub(crate) push: Push,
    pub(crate) server: Arc<dyn ServerRole>,
    pub(crate) client: Arc<dyn ClientRole>,
    pub(crate) usb_server: Arc<dyn UsbServer>,
    pub(crate) usb_client: Arc<dyn UsbClient>,
    certificate: Arc<CertificateSlot>,
    stoppers: Vec<Box<dyn Fn() + Send + Sync>>,
}

impl App {
    /// Opens every config file as 0.1.x opens it, makes the server certificate on a first start,
    /// and wires the pushes. Run after the folders are prepared and 0.1.x's secrets converted.
    pub async fn start(options: AppOptions) -> Result<Arc<Self>, StartError> {
        let AppOptions {
            paths,
            secrets,
            host,
            reach,
            setup_window,
            auth_per_minute,
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
        let devices = open::devices(&paths);
        devices.start().await?;
        let attached = open::attached(&paths);
        attached.start().await?;

        let stored = StoredServer::open(server_secrets.clone(), &host.name, push.clone())
            .await
            .map_err(StartError::Other)?;
        let certificate = stored.slot();

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
            Box::new({
                let f = devices.clone();
                move || f.stop()
            }),
            Box::new({
                let f = attached.clone();
                move || f.stop()
            }),
        ];

        Ok(Arc::new(Self {
            host,
            access: UiAccess::new(reach),
            auth_limit: RateLimit::per_minute(auth_per_minute),
            activity: Arc::new(ActivityLog::new(push.clone())),
            server: stored,
            client: StubClient::new(connection.clone(), push.clone()),
            usb_server: StubUsbServer::new(devices),
            usb_client: StubUsbClient::new(attached, connection.clone(), push.clone()),
            settings,
            connection,
            auth,
            push,
            certificate,
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
            .route("/healthz", get(crate::healthz))
            .fallback(fallback)
            .layer(middleware::from_fn_with_state(self.clone(), guard::guard))
            .with_state(self.clone())
    }

    /// Stops watching the config files for hand edits.
    pub fn stop(&self) {
        for stop in &self.stoppers {
            stop();
        }
    }
}

/// A path no route took: the UI's files for GET and HEAD, 404 for anything else. An unknown path
/// under `/api` is 404 too, where 0.1.2 answered a signed-in GET of one with index.html.
async fn fallback(request: Request) -> Response {
    if request.method() == Method::GET || request.method() == Method::HEAD {
        let (parts, _) = request.into_parts();
        crate::ui::serve(parts.uri, parts.headers).await
    } else {
        StatusCode::NOT_FOUND.into_response()
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

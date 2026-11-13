//! What each router's handlers hold. A handler takes the state of its own router, never the whole
//! [`App`], so what it can reach is what its router was given, and a test builds one without the rest.
//! Each is made from the app with [`FromRef`].

use std::sync::Arc;

use axum::extract::FromRef;
use koppel_config::ConfigFile;
use koppel_config::files::{ClientConnection, KoppelSettings};

use crate::app::App;
use crate::auth::UiAuth;
use crate::tls::CertificateSlot;
use koppel_app::activity::ActivityLog;
use koppel_app::host::Host;
use koppel_app::push::Push;
use koppel_app::roles::{SkakelClientRole, SkakelServerRole, UsbClientRole, UsbServerRole};

/// `/api/v1/auth/*`: sign-in and the password.
#[derive(Clone)]
pub(crate) struct AuthApi {
    pub auth: Arc<UiAuth>,
    pub activity: Arc<ActivityLog>,
    pub host: Host,
}

/// `/api/v1/settings`, `/api/v1/activity` and `/api/v1/system/*`.
#[derive(Clone)]
pub(crate) struct SettingsApi {
    pub settings: ConfigFile<KoppelSettings>,
    pub activity: Arc<ActivityLog>,
    pub host: Host,
    pub server: Arc<SkakelServerRole>,
    pub usb_server: Arc<UsbServerRole>,
    pub usb_client: Arc<UsbClientRole>,
}

/// `/api/v1/server/*`.
#[derive(Clone)]
pub(crate) struct ServerApi {
    pub server: Arc<SkakelServerRole>,
    pub usb_server: Arc<UsbServerRole>,
    pub activity: Arc<ActivityLog>,
    pub host: Host,
}

/// `/api/v1/client/*`.
#[derive(Clone)]
pub(crate) struct ClientApi {
    pub client: Arc<SkakelClientRole>,
    pub connection: ConfigFile<ClientConnection>,
    pub usb_client: Arc<UsbClientRole>,
}

/// `/readyz`.
#[derive(Clone)]
pub(crate) struct ReadyApi {
    pub usb_client: Arc<UsbClientRole>,
    pub certificate: Arc<CertificateSlot>,
}

/// `/api/v1/events`.
#[derive(Clone)]
pub(crate) struct PushApi {
    pub push: Push,
    // The rest is what a new stream starts with: the state the GET routes read.
    pub settings: ConfigFile<KoppelSettings>,
    pub connection: ConfigFile<ClientConnection>,
    pub server: Arc<SkakelServerRole>,
    pub usb_server: Arc<UsbServerRole>,
    pub client: Arc<SkakelClientRole>,
    pub usb_client: Arc<UsbClientRole>,
}

impl FromRef<Arc<App>> for AuthApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            auth: app.auth.clone(),
            activity: app.core.activity.clone(),
            host: app.core.host.clone(),
        }
    }
}

impl FromRef<Arc<App>> for SettingsApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            settings: app.core.settings.clone(),
            activity: app.core.activity.clone(),
            host: app.core.host.clone(),
            server: app.core.server.clone(),
            usb_server: app.core.usb_server.clone(),
            usb_client: app.core.usb_client.clone(),
        }
    }
}

impl FromRef<Arc<App>> for ServerApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            server: app.core.server.clone(),
            usb_server: app.core.usb_server.clone(),
            activity: app.core.activity.clone(),
            host: app.core.host.clone(),
        }
    }
}

impl FromRef<Arc<App>> for ClientApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            client: app.core.client.clone(),
            connection: app.core.connection.clone(),
            usb_client: app.core.usb_client.clone(),
        }
    }
}

impl FromRef<Arc<App>> for ReadyApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            usb_client: app.core.usb_client.clone(),
            certificate: app.core.certificate(),
        }
    }
}

impl FromRef<Arc<App>> for PushApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            push: app.core.push.clone(),
            settings: app.core.settings.clone(),
            connection: app.core.connection.clone(),
            server: app.core.server.clone(),
            usb_server: app.core.usb_server.clone(),
            client: app.core.client.clone(),
            usb_client: app.core.usb_client.clone(),
        }
    }
}

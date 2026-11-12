//! What each router's handlers hold. A handler takes the state of its own router, never the whole
//! [`App`], so what it can reach is what its router was given, and a test builds one without the rest.
//! Each is made from the app with [`FromRef`].

use std::sync::Arc;

use axum::extract::FromRef;
use koppel_config::ConfigFile;
use koppel_config::files::{ClientConnection, KoppelSettings};

use crate::activity::ActivityLog;
use crate::app::App;
use crate::auth::UiAuth;
use crate::host::Host;
use crate::push::Push;
use crate::roles::{SkakelClientRole, SkakelServerRole, UsbClientRole, UsbServerRole};

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

/// `/api/v1/events`.
#[derive(Clone)]
pub(crate) struct PushApi {
    pub push: Push,
}

impl FromRef<Arc<App>> for AuthApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            auth: app.auth.clone(),
            activity: app.activity.clone(),
            host: app.host.clone(),
        }
    }
}

impl FromRef<Arc<App>> for SettingsApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            settings: app.settings.clone(),
            activity: app.activity.clone(),
            host: app.host.clone(),
            server: app.server.clone(),
            usb_server: app.usb_server.clone(),
            usb_client: app.usb_client.clone(),
        }
    }
}

impl FromRef<Arc<App>> for ServerApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            server: app.server.clone(),
            usb_server: app.usb_server.clone(),
            activity: app.activity.clone(),
            host: app.host.clone(),
        }
    }
}

impl FromRef<Arc<App>> for ClientApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            client: app.client.clone(),
            connection: app.connection.clone(),
            usb_client: app.usb_client.clone(),
        }
    }
}

impl FromRef<Arc<App>> for PushApi {
    fn from_ref(app: &Arc<App>) -> Self {
        Self {
            push: app.push.clone(),
        }
    }
}

//! What answers for the client role and USB until their issues land: the same shapes and errors as
//! 0.1.x, honest about what this build cannot do.

use std::sync::{Arc, Mutex, PoisonError, Weak};

use koppel_config::ConfigFile;
use koppel_config::files::{AttachedDevices, ClientConnection, SharedDevices};

use super::{
    ClientDevices, ClientRole, ConnectionTest, DiscoveredServer, Fut, PairingStatus, ServerDevices,
    SystemCheck, UsbClient, UsbServer,
};
use crate::push::{self, Push};

const NO_LINK: &str = "This build of Koppel cannot link to a server yet.";
const NO_PAIRING: &str = "Pairing is not in this build of Koppel yet.";
const NO_SHARING: &str = "Sharing USB devices is not in this build of Koppel yet.";
const NO_ATTACHING: &str = "Attaching USB devices is not in this build of Koppel yet.";

/// The client role without a Skakel link (#85): the test says what is missing or that it cannot
/// reach anything, nothing is found on the network, and pairing fails at once with a reason.
pub struct StubClient {
    connection: ConfigFile<ClientConnection>,
    push: Push,
    status: Mutex<PairingStatus>,
}

impl StubClient {
    pub fn new(connection: ConfigFile<ClientConnection>, push: Push) -> Arc<Self> {
        Arc::new(Self {
            connection,
            push,
            status: Mutex::new(PairingStatus::idle()),
        })
    }

    fn set(&self, status: PairingStatus) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
        self.push.send(push::PAIRING_STATUS_CHANGED, &status);
    }
}

impl ClientRole for StubClient {
    fn test(&self) -> Fut<'_, ConnectionTest> {
        Box::pin(async move {
            if !self.connection.current().is_complete() {
                return ConnectionTest {
                    ok: false,
                    outcome: "incomplete".into(),
                    message: "Fill in the server address, fingerprint, client ID and secret first."
                        .into(),
                };
            }
            ConnectionTest {
                ok: false,
                outcome: "unreachable".into(),
                message: NO_LINK.into(),
            }
        })
    }

    fn discovered(&self) -> Vec<DiscoveredServer> {
        Vec::new()
    }

    fn pairing(&self) -> PairingStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn begin_pairing<'a>(
        &'a self,
        server_url: &'a str,
        fingerprint: &'a str,
        server_name: Option<&'a str>,
    ) -> Fut<'a, ()> {
        Box::pin(async move {
            let mut status = PairingStatus::of("failed", Some(NO_PAIRING.into()));
            status.server_url = Some(server_url.to_owned());
            status.server_name = server_name.map(str::to_owned);
            status.server_fingerprint = Some(fingerprint.to_owned());
            self.set(status);
        })
    }

    fn confirm_pairing(&self) -> Fut<'_, bool> {
        Box::pin(async { false })
    }

    fn cancel_pairing(&self) -> Fut<'_, ()> {
        Box::pin(async move { self.set(PairingStatus::idle()) })
    }
}

/// The server's USB side before #86 and #87: no devices, and why. It keeps `devices.json` checked
/// as 0.1.x does, so a bad file still shows under the checks.
pub struct StubUsbServer {
    devices: ConfigFile<SharedDevices>,
}

impl StubUsbServer {
    pub fn new(devices: ConfigFile<SharedDevices>) -> Arc<Self> {
        Arc::new(Self { devices })
    }
}

impl UsbServer for StubUsbServer {
    fn current(&self) -> ServerDevices {
        ServerDevices {
            devices: Vec::new(),
            unavailable: Some(NO_SHARING.into()),
        }
    }

    fn set_shared<'a>(&'a self, _bus_id: &'a str, _shared: bool) -> Fut<'a, ()> {
        Box::pin(async {})
    }

    fn check(&self) -> SystemCheck {
        SystemCheck::new("share", false, "Sharing is not available.", NO_SHARING)
    }

    fn problem(&self) -> Option<String> {
        self.devices.problem()
    }
}

/// The client's USB side before #86 and #87: the link as the connection allows, no devices, and
/// why. It keeps `attached.json` checked as 0.1.x does, and tells the UI when the connection changes
/// what the link shows.
pub struct StubUsbClient {
    attached: ConfigFile<AttachedDevices>,
    connection: ConfigFile<ClientConnection>,
    push: Push,
}

impl StubUsbClient {
    pub fn new(
        attached: ConfigFile<AttachedDevices>,
        connection: ConfigFile<ClientConnection>,
        push: Push,
    ) -> Arc<Self> {
        let usb = Arc::new(Self {
            attached,
            connection: connection.clone(),
            push,
        });
        let weak: Weak<Self> = Arc::downgrade(&usb);
        connection.subscribe(move |_| {
            let weak = weak.clone();
            async move {
                if let Some(usb) = weak.upgrade() {
                    usb.push.send(push::CLIENT_DEVICES_CHANGED, &usb.current());
                }
            }
        });
        usb
    }
}

impl UsbClient for StubUsbClient {
    fn current(&self) -> ClientDevices {
        let configured = self.connection.current().is_complete();
        ClientDevices {
            link: if configured { "error" } else { "unconfigured" }.into(),
            link_message: configured.then(|| NO_LINK.to_owned()),
            devices: Vec::new(),
            unavailable: Some(NO_ATTACHING.into()),
            gateway: false,
            gateway_fault: None,
        }
    }

    fn attach<'a>(&'a self, _bus_id: &'a str) -> Fut<'a, ()> {
        Box::pin(async move {
            self.push
                .send(push::CLIENT_DEVICES_CHANGED, &self.current())
        })
    }

    fn detach<'a>(&'a self, _bus_id: &'a str) -> Fut<'a, ()> {
        Box::pin(async move {
            self.push
                .send(push::CLIENT_DEVICES_CHANGED, &self.current())
        })
    }

    fn check(&self) -> SystemCheck {
        SystemCheck::new("attach", false, "Attaching is not available.", NO_ATTACHING)
    }

    fn problem(&self) -> Option<String> {
        self.attached.problem()
    }
}

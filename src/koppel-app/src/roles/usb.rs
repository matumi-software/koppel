//! The UI's USB endpoints over koppel-usb: the server's devices and sharing, and the client's link,
//! the server's devices and attaching. Each change reaches the UI as `DevicesChanged` or
//! `ClientDevicesChanged` once koppel-usb has it, and koppel-usb's activity goes to the log.

use std::sync::Arc;

use koppel_usb::{
    ActivityOutcome, ActivityReason, ActivitySink, ClientServersView, ClientsError, EntityRef,
    Platform, ServerDevices, SystemCheck, UsbClients, UsbServer as Server, attach_check,
    share_check,
};

use crate::activity::ActivityLog;
use crate::push::{Event, Push};

/// The server's USB side.
pub struct UsbServerRole {
    server: Server,
    platform: Platform,
}

impl UsbServerRole {
    /// Pushes each new device list as `DevicesChanged`.
    pub fn new(server: Server, platform: Platform, push: Push) -> Arc<Self> {
        server.on_changed(move |devices: Arc<ServerDevices>| {
            let push = push.clone();
            async move { push.send(Event::DevicesChanged(&devices)) }
        });
        Arc::new(Self { server, platform })
    }

    pub fn current(&self) -> ServerDevices {
        ServerDevices::clone(&self.server.current())
    }

    /// Applies a share choice; the outcome follows as `DevicesChanged`.
    pub async fn set_shared(&self, bus_id: &str, shared: bool) {
        if let Err(e) = self.server.set_shared(bus_id, shared).await {
            tracing::warn!("Sharing {bus_id} failed: {e}");
        }
    }

    /// Whether this machine can share, for `/api/v1/system/checks`.
    pub fn check(&self) -> SystemCheck {
        share_check(&self.platform)
    }

    /// Why `devices.json` was not used, or `None`.
    pub fn problem(&self) -> Option<String> {
        self.server.shared_problem()
    }
}

/// The client's USB side: one client per saved server.
pub struct UsbClientRole {
    clients: UsbClients,
    platform: Platform,
}

impl UsbClientRole {
    /// Pushes what the UI shows as `ClientDevicesChanged` each time it may have changed.
    pub fn new(clients: UsbClients, platform: Platform, push: Push) -> Arc<Self> {
        clients.on_changed(move |view: Arc<ClientServersView>| {
            let push = push.clone();
            async move { push.send(Event::ClientDevicesChanged(&view)) }
        });
        Arc::new(Self { clients, platform })
    }

    pub fn current(&self) -> ClientServersView {
        self.clients.current()
    }

    /// Remembers the device of that server and attaches it; the outcome follows as
    /// `ClientDevicesChanged`.
    pub async fn attach(&self, server_id: &str, bus_id: &str) -> Result<(), ClientsError> {
        let result = self.clients.begin_attach(server_id, bus_id).await;
        if let Err(e) = &result {
            tracing::warn!("Attaching {bus_id} from {server_id} failed: {e}");
        }
        result
    }

    /// Detaches the device and forgets it.
    pub async fn detach(&self, server_id: &str, bus_id: &str) -> Result<(), ClientsError> {
        self.clients.detach(server_id, bus_id).await
    }

    /// Whether this machine can attach, for `/api/v1/system/checks`.
    pub fn check(&self) -> SystemCheck {
        attach_check(&self.platform)
    }

    /// Why `attached.json` was not used, or `None`.
    pub fn problem(&self) -> Option<String> {
        self.clients.remembered_problem()
    }
}

/// koppel-usb's activity into the log.
pub struct UsbActivity(pub Arc<ActivityLog>);

impl ActivitySink for UsbActivity {
    fn record(
        &self,
        entity: EntityRef,
        reason: ActivityReason,
        outcome: ActivityOutcome,
        summary: String,
        detail: Option<String>,
    ) {
        self.0.record(entity, reason, outcome, summary, detail);
    }
}

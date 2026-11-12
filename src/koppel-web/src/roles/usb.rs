//! The UI's USB endpoints over koppel-usb: the server's devices and sharing, and the client's link,
//! the server's devices and attaching. Each change reaches the UI as `DevicesChanged` or
//! `ClientDevicesChanged` once koppel-usb has it, and koppel-usb's activity goes to the log.

use std::sync::Arc;

use koppel_usb::{
    ActivityOutcome, ActivityReason, ActivitySink, ClientDevices, EntityRef, Platform,
    ServerDevices, SystemCheck, UsbClient as Client, UsbServer as Server, attach_check,
    share_check,
};

use crate::activity::ActivityLog;
use crate::push::{self, Push};

/// The server's USB side.
pub struct UsbServerRole {
    server: Server,
    platform: Platform,
}

impl UsbServerRole {
    /// Pushes each new device list as `DevicesChanged`.
    pub(crate) fn new(server: Server, platform: Platform, push: Push) -> Arc<Self> {
        server.on_changed(move |devices: Arc<ServerDevices>| {
            let push = push.clone();
            async move { push.send(push::DEVICES_CHANGED, &*devices) }
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

    /// Whether this machine can share, for `/api/system/checks`.
    pub fn check(&self) -> SystemCheck {
        share_check(&self.platform)
    }

    /// Why `devices.json` was not used, or `None`.
    pub fn problem(&self) -> Option<String> {
        self.server.shared_problem()
    }
}

/// The client's USB side.
pub struct UsbClientRole {
    client: Client,
    platform: Platform,
}

impl UsbClientRole {
    /// Pushes what the UI shows as `ClientDevicesChanged` each time it may have changed.
    pub(crate) fn new(client: Client, platform: Platform, push: Push) -> Arc<Self> {
        client.on_changed(move |devices: Arc<ClientDevices>| {
            let push = push.clone();
            async move { push.send(push::CLIENT_DEVICES_CHANGED, &*devices) }
        });
        Arc::new(Self { client, platform })
    }

    pub fn current(&self) -> ClientDevices {
        self.client.current()
    }

    /// Remembers the device and attaches it; the outcome follows as `ClientDevicesChanged`.
    pub async fn attach(&self, bus_id: &str) {
        if let Err(e) = self.client.begin_attach(bus_id).await {
            tracing::warn!("Attaching {bus_id} failed: {e}");
        }
    }

    /// Detaches the device and forgets it.
    pub async fn detach(&self, bus_id: &str) {
        self.client.detach(bus_id).await;
    }

    /// Whether this machine can attach, for `/api/system/checks`.
    pub fn check(&self) -> SystemCheck {
        attach_check(&self.platform)
    }

    /// Why `attached.json` was not used, or `None`.
    pub fn problem(&self) -> Option<String> {
        self.client.remembered_problem()
    }
}

/// koppel-usb's activity into the log.
pub(crate) struct UsbActivity(pub Arc<ActivityLog>);

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

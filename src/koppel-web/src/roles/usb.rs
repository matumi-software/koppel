//! The UI's USB endpoints over koppel-usb: the server's devices and sharing, and the client's link,
//! the server's devices and attaching. Each change reaches the UI as `DevicesChanged` or
//! `ClientDevicesChanged` once koppel-usb has it, and koppel-usb's activity goes to the log.

use std::sync::Arc;

use koppel_usb::{
    ActivitySink, UsbAttacher, UsbClient as Client, UsbHost, UsbIpDriver, UsbServer as Server,
    attach_check, share_check,
};

use super::{
    ClientDeviceView, ClientDevices, DeviceView, Fut, ServerDevices, SystemCheck, UsbClient,
    UsbServer,
};
use crate::activity::{self, ActivityLog};
use crate::push::{self, Push};

/// The server's USB side.
pub struct UsbServerRole {
    server: Server,
    host: Arc<dyn UsbHost>,
}

impl UsbServerRole {
    /// Pushes each new device list as `DevicesChanged`.
    pub(crate) fn new(server: Server, push: Push) -> Arc<Self> {
        server.on_changed(move |devices| {
            let push = push.clone();
            async move { push.send(push::DEVICES_CHANGED, &server_devices(&devices)) }
        });
        Arc::new(Self {
            host: server.host().clone(),
            server,
        })
    }
}

impl UsbServer for UsbServerRole {
    fn current(&self) -> ServerDevices {
        server_devices(&self.server.current())
    }

    fn set_shared<'a>(&'a self, bus_id: &'a str, shared: bool) -> Fut<'a, ()> {
        Box::pin(async move {
            if let Err(e) = self.server.set_shared(bus_id, shared).await {
                tracing::warn!("Sharing {bus_id} failed: {e}");
            }
        })
    }

    fn check(&self) -> SystemCheck {
        system_check(share_check(self.host.as_ref(), cfg!(windows)))
    }

    fn problem(&self) -> Option<String> {
        self.server.shared_problem()
    }
}

/// The client's USB side.
pub struct UsbClientRole {
    client: Client,
    attacher: Arc<dyn UsbAttacher>,
    driver: Arc<dyn UsbIpDriver>,
}

impl UsbClientRole {
    /// Pushes what the UI shows as `ClientDevicesChanged` each time it may have changed.
    pub(crate) fn new(
        client: Client,
        attacher: Arc<dyn UsbAttacher>,
        driver: Arc<dyn UsbIpDriver>,
        push: Push,
    ) -> Arc<Self> {
        client.on_changed(move |devices| {
            let push = push.clone();
            async move { push.send(push::CLIENT_DEVICES_CHANGED, &client_devices(&devices)) }
        });
        Arc::new(Self {
            client,
            attacher,
            driver,
        })
    }
}

impl UsbClient for UsbClientRole {
    fn current(&self) -> ClientDevices {
        client_devices(&self.client.current())
    }

    fn attach<'a>(&'a self, bus_id: &'a str) -> Fut<'a, ()> {
        Box::pin(async move {
            if let Err(e) = self.client.begin_attach(bus_id).await {
                tracing::warn!("Attaching {bus_id} failed: {e}");
            }
        })
    }

    fn detach<'a>(&'a self, bus_id: &'a str) -> Fut<'a, ()> {
        Box::pin(async move { self.client.detach(bus_id).await })
    }

    fn check(&self) -> SystemCheck {
        system_check(attach_check(
            self.attacher.as_ref(),
            self.driver.as_ref(),
            cfg!(windows),
        ))
    }

    fn problem(&self) -> Option<String> {
        self.client.remembered_problem()
    }
}

/// koppel-usb's activity into the log.
pub(crate) struct UsbActivity(pub Arc<ActivityLog>);

impl ActivitySink for UsbActivity {
    fn record(
        &self,
        entity: koppel_usb::EntityRef,
        reason: koppel_usb::ActivityReason,
        outcome: koppel_usb::ActivityOutcome,
        summary: String,
        detail: Option<String>,
    ) {
        use activity::ActivityReason as R;
        use koppel_usb::ActivityReason as U;
        let reason = match reason {
            U::Shared => R::Shared,
            U::Unshared => R::Unshared,
            U::Attached => R::Attached,
            U::Detached => R::Detached,
            U::Plugged => R::Plugged,
            U::Unplugged => R::Unplugged,
            U::LinkUp => R::LinkUp,
            U::LinkDown => R::LinkDown,
        };
        let outcome = match outcome {
            koppel_usb::ActivityOutcome::Succeeded => activity::ActivityOutcome::Succeeded,
            koppel_usb::ActivityOutcome::Failed => activity::ActivityOutcome::Failed,
        };
        self.0.record(
            activity::EntityRef::new(entity.id, entity.name, entity.kind),
            reason,
            outcome,
            summary,
            detail,
        );
    }
}

fn system_check(check: koppel_usb::SystemCheck) -> SystemCheck {
    SystemCheck {
        id: check.id,
        ok: check.ok,
        title: check.title,
        detail: check.detail,
    }
}

fn device(d: &koppel_usb::DeviceView) -> DeviceView {
    DeviceView {
        bus_id: d.bus_id.clone(),
        vid_pid: d.vid_pid.clone(),
        name: d.name.clone(),
        manufacturer: d.manufacturer.clone(),
        speed: d.speed.clone(),
        state: d.state.clone(),
        attached_to: d.attached_to.clone(),
        attached_since: d.attached_since,
        fault: d.fault.clone(),
        serial: d.serial.clone(),
        driver: d.driver.clone(),
        interfaces: d.interfaces.clone(),
    }
}

fn server_devices(devices: &koppel_usb::ServerDevices) -> ServerDevices {
    ServerDevices {
        devices: devices.devices.iter().map(device).collect(),
        unavailable: devices.unavailable.clone(),
    }
}

fn client_devices(devices: &koppel_usb::ClientDevices) -> ClientDevices {
    ClientDevices {
        link: devices.link.clone(),
        link_message: devices.link_message.clone(),
        devices: devices
            .devices
            .iter()
            .map(|d| ClientDeviceView {
                device: device(&d.device),
                attached_here: d.attached_here,
                port: d.port.and_then(|p| i32::try_from(p).ok()),
                attached_since: d.attached_since,
                fault: d.fault.clone(),
                attached_via: d.attached_via.clone(),
            })
            .collect(),
        unavailable: devices.unavailable.clone(),
        gateway: devices.gateway,
        gateway_fault: devices.gateway_fault.clone(),
    }
}

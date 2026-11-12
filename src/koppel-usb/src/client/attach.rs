//! The work an attach does outside the actor: opening the stream, importing the device, handing
//! it to the kernel or the driver, and undoing all of that. Nothing here changes the client's
//! state; each step ends in a command the actor decides about.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::actor::LOOPBACK;
use super::slots::{AttachKey, Attachment, DriverHold, Kind, Mode, Outcome, Plan};
use super::{AttachError, Command, Failure, Shared, check_import};
use crate::{
    ActivityOutcome, ActivityReason, BusId, CloseReason, DeviceStream, DeviceView, loopback_pair,
    protocol, relay,
};

/// The stream to the server for one imported device, and the device record it sent.
pub(super) struct RemoteImport {
    pub session: Box<dyn DeviceStream>,
    pub record: [u8; protocol::DEVICE_SIZE],
}

/// One attach, start to finish, in its own task. `cancel` ends when the connection changes, the
/// device is detached or Koppel stops.
pub(super) async fn run(
    shared: &Arc<Shared>,
    key: &AttachKey,
    attempt: u64,
    mode: Mode,
    cancel: &CancellationToken,
    releasing: &CancellationToken,
) -> Outcome {
    // A change that came first is still releasing what it ended; do not attach under its feet.
    tokio::select! {
        () = releasing.cancelled() => {}
        () = cancel.cancelled() => return Outcome::Cancelled,
    }
    let result = match mode {
        Mode::Driver => through_driver(shared, key).await,
        Mode::Kernel => through_kernel(shared, key, attempt, cancel).await,
    };
    match result {
        Ok(hold) => Outcome::Done(hold),
        Err(failure) => failure.outcome(),
    }
}

impl Shared {
    /// Opens a stream to the server on the link. USB/IP follows inside it. The caller sets the
    /// deadline.
    pub(super) async fn open_session(&self) -> Result<Box<dyn DeviceStream>, Failure> {
        let link = {
            let view = self.view();
            match (&view.link, view.listed) {
                (Some(link), true) => link.clone(),
                _ => return Err(AttachError::NotConnected.into()),
            }
        };
        link.open().await.map_err(|e| AttachError::Open(e).into())
    }

    /// Connects and imports `bus_id`, all within one deadline. Returns the stream and the
    /// server's device record.
    pub(super) async fn import(&self, bus_id: &BusId) -> Result<RemoteImport, Failure> {
        self.under_deadline(async {
            let mut session = self.open_session().await?;
            let handshake = async {
                session.write_all(&protocol::import_request(bus_id)).await?;
                let header = protocol::read_header(&mut *session).await?;
                if header.code != protocol::OP_REP_IMPORT || header.status != protocol::STATUS_OK {
                    return Err(Failure::from(AttachError::Refused));
                }
                let mut record = [0u8; protocol::DEVICE_SIZE];
                session.read_exact(&mut record).await?;
                Ok(record)
            };
            match handshake.await {
                Ok(record) => Ok(RemoteImport { session, record }),
                Err(e) => {
                    session.close(CloseReason::Done).await;
                    Err(e)
                }
            }
        })
        .await
    }

    /// The server's record checked against the device the user picked.
    pub(super) fn check_record(
        &self,
        record: &[u8; protocol::DEVICE_SIZE],
        bus_id: &BusId,
        picked: Option<&DeviceView>,
    ) -> Result<crate::UsbDevice, AttachError> {
        let device = protocol::read_device(record).map_err(|_| AttachError::BadRecord)?;
        check_import(&device, bus_id, picked)?;
        Ok(device)
    }
}

/// Asks the driver to attach from the gateway. The gateway sees the import and registers the
/// device; this returns the driver's port, which detaching needs.
async fn through_driver(
    shared: &Arc<Shared>,
    key: &AttachKey,
) -> Result<Option<DriverHold>, Failure> {
    let (listed, gateway) = {
        let view = shared.view();
        (
            view.listed,
            view.gateway_port.ok_or_else(|| view.gateway_fault.clone()),
        )
    };
    if !shared.connection.current().is_complete() || !listed {
        return Err(AttachError::NotConnected.into());
    }
    let gateway_port = gateway.map_err(|fault| {
        AttachError::GatewayDown(
            fault.unwrap_or_else(|| "The local USB/IP gateway is not running.".to_owned()),
        )
    })?;

    // The driver's own command has a time limit, and killing it half way could leave the attach
    // half done, so it is not cancelled; a change that came meanwhile is undone by the actor.
    let driver_port = shared
        .platform
        .driver()
        .map_err(AttachError::from)?
        .attach(&LOOPBACK.to_string(), gateway_port, &key.bus_id)
        .await
        .map_err(AttachError::from)?;
    tracing::info!("Attached {} on virtual port {driver_port}", key.bus_id);
    Ok(Some(DriverHold {
        gateway_port,
        driver_port,
    }))
}

/// Attaches through this kernel's virtual host controller.
async fn through_kernel(
    shared: &Arc<Shared>,
    key: &AttachKey,
    attempt: u64,
    cancel: &CancellationToken,
) -> Result<Option<DriverHold>, Failure> {
    if let Some(reason) = shared.platform.attach_unavailable() {
        return Err(AttachError::Usb(crate::UsbError::Unavailable(reason)).into());
    }
    let bus_id = &key.bus_id;
    let picked = shared
        .offered(bus_id)
        .ok_or_else(|| AttachError::NotOffered(bus_id.clone()))?;
    let import = tokio::select! {
        import = shared.import(bus_id) => import?,
        () = cancel.cancelled() => return Err(Failure::Cancelled),
    };
    let mut session = Some(import.session);
    let result = hand_to_kernel(shared, key, attempt, &picked, &mut session, &import.record).await;
    if let Some(session) = session {
        session.close(CloseReason::Done).await;
    }
    result.map(|()| None)
}

/// The rest of a kernel attach once the server answered. Takes the session only when it hands it
/// to the relay.
async fn hand_to_kernel(
    shared: &Arc<Shared>,
    key: &AttachKey,
    attempt: u64,
    picked: &DeviceView,
    session: &mut Option<Box<dyn DeviceStream>>,
    record: &[u8; protocol::DEVICE_SIZE],
) -> Result<(), Failure> {
    let bus_id = &key.bus_id;
    let device = shared.check_record(record, bus_id, Some(picked))?;
    if shared.view().epoch.id != key.epoch {
        return Err(AttachError::ConnectionChanged.into());
    }

    let (kernel_end, relay_end) = loopback_pair().await.map_err(AttachError::Loopback)?;
    let port = shared
        .platform
        .attach_kernel(&kernel_end, device.dev_id(), device.speed)
        .await
        .map_err(AttachError::from)?;

    let attachment = Attachment {
        id: shared.next_attachment_id(),
        port: Some(port),
        stop: CancellationToken::new(),
        since: shared.clock.now(),
        kind: Kind::Kernel,
    };
    let (stop, id) = (attachment.stop.clone(), attachment.id);
    let (reply, answer) = oneshot::channel();
    shared.send(Command::Register {
        key: key.clone(),
        attempt: Some(attempt),
        attachment,
        reply,
    });
    match answer.await {
        Ok(Ok(())) => {}
        refused => {
            // The attach is not wanted any more, or something else attached the device first;
            // give the port back.
            let _ = shared.platform.detach_kernel(port).await;
            return Err(match refused {
                Ok(Err(super::slots::RegisterError::AlreadyAttached)) => {
                    AttachError::AlreadyAttached(bus_id.clone()).into()
                }
                _ => Failure::Cancelled,
            });
        }
    }

    let mut session = session.take().expect("the session is handed over once");
    let relayed = shared.clone();
    let relay_key = key.clone();
    shared.workers.spawn(format!("relay {bus_id}"), async move {
        relay(&mut *session, relay_end, &stop).await;
        drop(kernel_end);
        session.close(CloseReason::Done).await;
        relayed.send(Command::RelayEnded {
            key: relay_key,
            attachment: id,
        });
    });

    tracing::info!("Attached {bus_id} on virtual port {port}");
    shared.activity.record(
        shared.device(bus_id),
        ActivityReason::Attached,
        ActivityOutcome::Succeeded,
        format!(
            "{} is attached here on virtual port {port}",
            shared.name_of(bus_id)
        ),
        None,
    );
    Ok(())
}

/// Undoes what `plan` holds: the driver's or the kernel's port, then the relay. A driver attach
/// is undone by the driver, which would otherwise attach it again. A visitor has no port here;
/// stopping its relay unplugs it on the other side. Returns what went wrong, and the hold when
/// the driver could not be told to let go of it.
pub(super) async fn release(
    shared: &Arc<Shared>,
    key: &AttachKey,
    plan: Plan,
) -> (Option<String>, Option<DriverHold>) {
    let bus_id = &key.bus_id;
    let mut hold_back = None;
    let undone: Result<bool, crate::UsbError> = async {
        let mut detached = false;
        if let Some(hold) = plan.hold {
            let loopback = LOOPBACK.to_string();
            if let Err(e) = shared
                .platform
                .driver()?
                .detach(&loopback, hold.gateway_port, bus_id, hold.driver_port)
                .await
            {
                // Still attached, so the next release tries again.
                hold_back = Some(hold);
                return Err(e);
            }
            detached = true;
        }
        let kernel_port = plan
            .attachment
            .as_ref()
            .filter(|a| a.kind == Kind::Kernel)
            .and_then(|a| a.port);
        if !detached && let Some(port) = kernel_port {
            shared.platform.detach_kernel(port).await?;
            detached = true;
        }
        Ok(detached)
    }
    .await;
    let failure = match undone {
        Ok(detached) => {
            if detached {
                shared.activity.record(
                    shared.device(bus_id),
                    ActivityReason::Detached,
                    ActivityOutcome::Succeeded,
                    format!("{} was detached", shared.name_of(bus_id)),
                    None,
                );
            }
            None
        }
        Err(e) => {
            tracing::warn!("Detaching {bus_id} failed: {e}");
            Some(e.to_string())
        }
    };
    if let Some(attachment) = plan.attachment {
        attachment.stop.cancel();
    }
    (failure, hold_back)
}

//! One connection to the client's gateway, from a USB/IP driver on this machine: what it asks
//! for, whether Koppel allows it, and carrying it to the server.

use std::net::{IpAddr, SocketAddr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::attach::RemoteImport;
use super::slots::{AttachKey, Attachment, Kind};
use super::{AttachError, Command, Failure, Shared};
use crate::upstream::MAX_LIST_BYTES;
use crate::{ActivityOutcome, ActivityReason, BusId, CloseReason, protocol};

/// What the driver asked for.
enum GatewayRequest {
    DevList,
    Import(BusId),
}

/// What reading the request came to.
enum Read {
    Request(GatewayRequest),
    /// An import of a bus ID that is not one, or an operation Koppel does not serve.
    Refuse(&'static str),
    /// The driver went away, said nothing in time, or Koppel is stopping.
    Over,
}

impl Shared {
    pub(super) async fn serve_gateway(
        &self,
        mut local: TcpStream,
        peer: SocketAddr,
        stopping: &CancellationToken,
    ) {
        let from = match peer.ip() {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => v6.to_canonical().to_string(),
        };
        let request = match self.read_request(&mut local, stopping).await {
            Read::Request(request) => request,
            Read::Refuse(why) => {
                tracing::warn!("Gateway request failed: {why}");
                refuse(&mut local, protocol::OP_REP_IMPORT).await;
                return;
            }
            Read::Over => return,
        };
        let epoch = self.view().epoch.id;
        let booked = match self.book(&request, epoch).await {
            Ok(booked) => booked,
            Err(()) => {
                tracing::warn!(
                    "Gateway request for {} failed: Koppel did not ask the driver for it",
                    match &request {
                        GatewayRequest::DevList => "the device list".to_owned(),
                        GatewayRequest::Import(bus_id) => bus_id.to_string(),
                    }
                );
                refuse(&mut local, reply_code(&request)).await;
                return;
            }
        };

        match request {
            GatewayRequest::DevList => {
                if let Err(Failure::Error(e)) = self.pass_dev_list(&mut local, stopping).await {
                    tracing::warn!("Gateway request for the device list failed: {e}");
                    refuse(&mut local, protocol::OP_REP_DEVLIST).await;
                }
            }
            GatewayRequest::Import(bus_id) => {
                let key = AttachKey { epoch, bus_id };
                self.serve_import(local, key, booked, &from, stopping).await;
            }
        }
    }

    /// Reads the request. A local peer that connects and says nothing must not hold the session
    /// open for ever.
    async fn read_request(&self, local: &mut TcpStream, stopping: &CancellationToken) -> Read {
        let request = async {
            let header = protocol::read_header(&mut *local).await?;
            let parsed = match header.code {
                protocol::OP_REQ_DEVLIST => Read::Request(GatewayRequest::DevList),
                protocol::OP_REQ_IMPORT => {
                    let text = protocol::read_bus_id(&mut *local).await?;
                    match text.parse::<BusId>() {
                        Ok(bus_id) => Read::Request(GatewayRequest::Import(bus_id)),
                        Err(_) => Read::Refuse("an invalid bus ID: not a USB bus ID"),
                    }
                }
                _ => Read::Over,
            };
            Ok::<_, std::io::Error>(parsed)
        };
        let request = tokio::select! {
            r = tokio::time::timeout(self.options.attach_timeout, request) => r,
            () = stopping.cancelled() => return Read::Over,
        };
        match request {
            Ok(Ok(read)) => read,
            Ok(Err(e)) => {
                tracing::debug!("Gateway session ended: {e}");
                Read::Over
            }
            Err(_) => {
                tracing::warn!(
                    "Gateway request failed: the driver did not finish its request in time"
                );
                Read::Over
            }
        }
    }

    /// With a driver on this machine the gateway serves only what Koppel itself asked the driver
    /// for: one import of one device per request. Another local user's "usbip attach" and the
    /// driver's own attempts to attach again after a drop are turned away; Koppel attaches again
    /// itself while it is wanted. Returns the attempt the import belongs to, or `None` when no
    /// driver is in play, and an error when the request is not one Koppel asked for.
    async fn book(&self, request: &GatewayRequest, epoch: u64) -> Result<Option<u64>, ()> {
        if !self.uses_driver(&self.view()) {
            return Ok(None);
        }
        let GatewayRequest::Import(bus_id) = request else {
            return Err(());
        };
        let (reply, answer) = oneshot::channel();
        self.send(Command::Book {
            key: AttachKey {
                epoch,
                bus_id: bus_id.clone(),
            },
            reply,
        });
        match answer.await {
            Ok(Some(attempt)) => Ok(Some(attempt)),
            _ => Err(()),
        }
    }

    fn ensure_client(&self) -> Result<(), Failure> {
        if !self.settings.current().role.links() {
            Err(AttachError::ServerRole.into())
        } else {
            Ok(())
        }
    }

    /// A device list, passed through from the server as it came, up to a cap.
    async fn pass_dev_list(
        &self,
        local: &mut TcpStream,
        stopping: &CancellationToken,
    ) -> Result<(), Failure> {
        self.ensure_client()?;
        self.under_deadline(async {
            let mut session = self.open_session().await?;
            let copied = async {
                session.write_all(&protocol::dev_list_request()).await?;
                let mut capped = (&mut session).take(MAX_LIST_BYTES as u64);
                tokio::select! {
                    copied = tokio::io::copy(&mut capped, local) => copied.map(|_| ()),
                    () = stopping.cancelled() => Ok(()),
                }
            }
            .await;
            session.close(CloseReason::Done).await;
            copied.map_err(Failure::from)
        })
        .await
    }

    /// The import, checked against the device the user picked.
    async fn import_device(&self, bus_id: &BusId) -> Result<RemoteImport, Failure> {
        self.ensure_client()?;
        let import = self.import(bus_id).await?;
        let offered = self.offered(bus_id);
        match self.check_record(&import.record, bus_id, offered.as_ref()) {
            Ok(_) => Ok(import),
            Err(e) => {
                import.session.close(CloseReason::Done).await;
                Err(e.into())
            }
        }
    }

    /// An import: ask the server, register the attachment, answer the driver and relay until
    /// either side ends.
    async fn serve_import(
        &self,
        mut local: TcpStream,
        key: AttachKey,
        booked: Option<u64>,
        from: &str,
        stopping: &CancellationToken,
    ) {
        let bus_id = key.bus_id.clone();
        let import = match self.import_device(&bus_id).await {
            Ok(import) => import,
            Err(failure) => {
                if let Failure::Error(e) = failure {
                    tracing::debug!("Gateway import failed: {e:?}");
                    self.send(Command::Fault {
                        key,
                        reason: e.to_string(),
                    });
                }
                refuse(&mut local, protocol::OP_REP_IMPORT).await;
                return;
            }
        };
        let RemoteImport {
            mut session,
            record,
        } = import;

        // An import this client's own attach asked the driver for is "attached here", not a visitor.
        let mine = booked.is_some();
        let attachment = Attachment {
            id: self.next_attachment_id(),
            port: None,
            stop: stopping.child_token(),
            since: self.clock.now(),
            kind: if mine {
                Kind::Driver
            } else {
                Kind::Visitor {
                    from: from.to_owned(),
                }
            },
        };
        let (stop, id) = (attachment.stop.clone(), attachment.id);
        let (reply, answer) = oneshot::channel();
        self.send(Command::Register {
            key: key.clone(),
            attempt: booked,
            attachment,
            reply,
        });
        if !matches!(answer.await, Ok(Ok(()))) {
            tracing::warn!("Gateway request for {bus_id} failed: already attached");
            refuse(&mut local, protocol::OP_REP_IMPORT).await;
            session.close(CloseReason::Done).await;
            return;
        }

        // Pass the server's device record on as it came, interface count and all.
        let mut reply = protocol::header(protocol::OP_REP_IMPORT, protocol::STATUS_OK).to_vec();
        reply.extend_from_slice(&record);
        if local.write_all(&reply).await.is_ok() {
            tracing::info!("Attached {bus_id} through the gateway for {from}");
            let name = self.name_of(&bus_id);
            self.activity.record(
                self.device(&bus_id),
                ActivityReason::Attached,
                ActivityOutcome::Succeeded,
                if mine {
                    format!("{name} is attached here")
                } else {
                    format!("{name} is attached through the gateway for {from}")
                },
                None,
            );
            usbip::relay(&mut *session, &mut local, stop.cancelled()).await;
        }

        let (reply, answer) = oneshot::channel();
        self.send(Command::Unregister {
            key,
            attachment: id,
            reply,
        });
        if answer.await.unwrap_or(false) {
            // Released on the other side, or the link dropped. A detach here records itself.
            let name = self.name_of(&bus_id);
            self.activity.record(
                self.device(&bus_id),
                ActivityReason::Detached,
                ActivityOutcome::Succeeded,
                if mine {
                    format!("{name} was detached when its connection ended")
                } else {
                    format!("{name} was released by {from}")
                },
                None,
            );
        }
        session.close(CloseReason::Done).await;
    }
}

fn reply_code(request: &GatewayRequest) -> u16 {
    match request {
        GatewayRequest::DevList => protocol::OP_REP_DEVLIST,
        GatewayRequest::Import(_) => protocol::OP_REP_IMPORT,
    }
}

/// Answers the driver with an error status, if it still listens.
async fn refuse(local: &mut TcpStream, code: u16) {
    let _ = local
        .write_all(&protocol::header(code, protocol::STATUS_ERROR))
        .await;
}

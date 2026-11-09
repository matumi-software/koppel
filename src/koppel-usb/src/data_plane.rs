//! The server's USB data plane: what a client asks for on a stream, and serving it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koppel_config::files::{KoppelSettings, Role};
use koppel_config::{ConfigFile, Subscription};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::tasks::lock;
use crate::{
    ActivityOutcome, ActivityReason, ActivitySink, CloseReason, DeviceStream, EntityRef,
    ExportStatus, UsbServer, bus_id, protocol,
};

/// Time for a client to ask for the device list or an import once its stream is open.
const REQUEST_BUDGET: Duration = Duration::from_secs(10);

/// How long ending the sessions waits for them to finish.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// Who opened a stream, as the transport checked them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteClient {
    pub id: String,
    pub name: String,
}

/// The server's USB data plane. The transport accepts a client's stream (checking its token and
/// its registration first) and hands it to [`UsbDataPlane::serve`], which speaks plain USB/IP
/// inside it: a device list, or an import followed by the device's traffic. On import the host
/// relays the stream to the device, so USB traffic never crosses the network unencrypted. Cheap
/// to clone.
///
/// Leaving the server role ends every session with [`CloseReason::Stopping`]. Keeping new streams
/// out while the role is not a server is the transport's job, as is ending a revoked client's.
#[derive(Clone)]
pub struct UsbDataPlane {
    inner: Arc<Inner>,
}

struct Inner {
    server: UsbServer,
    activity: Arc<dyn ActivitySink>,
    settings: ConfigFile<KoppelSettings>,
    sessions: Mutex<HashMap<u64, CancellationToken>>,
    next_session: AtomicU64,
    /// One lock per bus ID for the status check, the reset and the start of an export.
    bus_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Cancelled when the role leaves the server side; replaced for the sessions after.
    serving: Mutex<CancellationToken>,
    /// Cancelled when a session ends; a new one per session.
    ended: Mutex<HashMap<u64, CancellationToken>>,
    subscription: Mutex<Option<Subscription>>,
}

fn serves(role: Role) -> bool {
    matches!(role, Role::Server | Role::Both)
}

impl UsbDataPlane {
    pub fn new(
        server: UsbServer,
        activity: Arc<dyn ActivitySink>,
        settings: ConfigFile<KoppelSettings>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                server,
                activity,
                settings,
                sessions: Mutex::new(HashMap::new()),
                next_session: AtomicU64::new(1),
                bus_locks: Mutex::new(HashMap::new()),
                serving: Mutex::new(CancellationToken::new()),
                ended: Mutex::new(HashMap::new()),
                subscription: Mutex::new(None),
            }),
        }
    }

    /// Starts following the role: a machine that stops being a server ends what it serves.
    pub fn start(&self) {
        let weak = Arc::downgrade(&self.inner);
        let subscription = self.inner.settings.subscribe(move |next| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade()
                    && !serves(next.role)
                {
                    inner.end_serving().await;
                }
            }
        });
        *lock(&self.inner.subscription) = Some(subscription);
    }

    /// Ends every session and stops following the role.
    pub async fn stop(&self) {
        if let Some(subscription) = lock(&self.inner.subscription).take() {
            self.inner.settings.unsubscribe(subscription);
        }
        self.inner.end_serving().await;
    }

    /// How many sessions are being served.
    pub fn sessions(&self) -> usize {
        lock(&self.inner.sessions).len()
    }

    /// Serves one stream until the session ends, then closes it: with [`CloseReason::Stopping`]
    /// when the role left the server side, otherwise [`CloseReason::Done`].
    pub async fn serve(&self, mut stream: Box<dyn DeviceStream>, client: RemoteClient) {
        let inner = &self.inner;
        let abort = lock(&inner.serving).child_token();
        let id = inner.next_session.fetch_add(1, Ordering::Relaxed);
        let ended = CancellationToken::new();
        lock(&inner.sessions).insert(id, abort.clone());
        lock(&inner.ended).insert(id, ended.clone());

        if !abort.is_cancelled() {
            inner.serve_core(&mut *stream, &client, &abort).await;
        }

        lock(&inner.sessions).remove(&id);
        lock(&inner.ended).remove(&id);
        let reason = if abort.is_cancelled() {
            CloseReason::Stopping
        } else {
            CloseReason::Done
        };
        stream.close(reason).await;
        ended.cancel();
    }
}

impl Inner {
    async fn serve_core(
        &self,
        session: &mut dyn DeviceStream,
        client: &RemoteClient,
        abort: &CancellationToken,
    ) {
        let request = async {
            let header = protocol::read_header(&mut *session).await?;
            let bus_id = if header.code == protocol::OP_REQ_IMPORT {
                Some(protocol::read_bus_id(&mut *session).await?)
            } else {
                None
            };
            Ok::<_, std::io::Error>((header, bus_id))
        };
        let request = tokio::select! {
            r = tokio::time::timeout(REQUEST_BUDGET, request) => r,
            _ = abort.cancelled() => {
                tracing::debug!("USB session ended: it was cancelled");
                return;
            }
        };
        let (header, bus_id) = match request {
            Ok(Ok(request)) => request,
            Ok(Err(e)) => {
                tracing::debug!("USB session ended: {e}");
                return;
            }
            Err(_) => {
                tracing::debug!("USB session ended: it ran out of time");
                return;
            }
        };

        if header.code == protocol::OP_REQ_DEVLIST {
            match self.server.exportable().await {
                Ok(devices) => {
                    let reply = protocol::dev_list_reply(&devices);
                    tokio::select! {
                        r = session.write_all(&reply) => {
                            if let Err(e) = r {
                                tracing::debug!("USB session ended: {e}");
                            }
                        }
                        _ = abort.cancelled() => {}
                    }
                }
                Err(e) => tracing::warn!("A USB session failed: {e}"),
            }
        } else if let Some(bus_id) = bus_id {
            self.import(session, &bus_id, client, abort).await;
        }
    }

    async fn import(
        &self,
        session: &mut dyn DeviceStream,
        bus_id: &str,
        client: &RemoteClient,
        abort: &CancellationToken,
    ) {
        let refuse = protocol::import_reply(None);
        if !bus_id::is_valid(bus_id) {
            let _ = session.write_all(&refuse).await;
            return;
        }

        // The status check, the reset and the start of the export are one step per device.
        // Without it, two imports can both see the device available, and the second one's reset
        // hits the first one's session. The lock is let go as soon as the export has begun, when
        // the device reads as in use.
        let bus_lock = lock(&self.bus_locks)
            .entry(bus_id.to_owned())
            .or_default()
            .clone();
        let mut guard = tokio::select! {
            g = bus_lock.lock_owned() => Some(g),
            _ = abort.cancelled() => return,
        };

        let host = self.server.host().clone();
        let device = match self.server.exportable().await {
            Ok(devices) => devices.into_iter().find(|d| d.bus_id == bus_id),
            Err(e) => {
                tracing::warn!("A USB session failed: {e}");
                None
            }
        };
        let available = match &device {
            Some(_) => matches!(host.status(bus_id).await, Ok(ExportStatus::Available)),
            None => false,
        };
        let Some(device) = device.filter(|_| available) else {
            let _ = session.write_all(&refuse).await;
            return;
        };

        if host.can_reset()
            && let Err(e) = host.reset(bus_id).await
        {
            // Most devices work without it, so the attach goes ahead.
            tracing::warn!("Could not reset {bus_id} before export: {e}");
        }

        let export = match host.export(&device).await {
            Ok(export) => export,
            Err(e) => {
                tracing::warn!("Export of {bus_id} failed: {e}");
                let _ = session.write_all(&refuse).await;
                return;
            }
        };
        guard.take();

        if let Err(e) = session
            .write_all(&protocol::import_reply(Some(&device)))
            .await
        {
            tracing::debug!("USB session ended: {e}");
            return;
        }
        let device_name = device.product.clone().unwrap_or_else(|| bus_id.to_owned());
        tracing::info!("Device {bus_id} attached by {}", client.id);
        self.activity.record(
            EntityRef::device(bus_id, &device_name),
            ActivityReason::Attached,
            ActivityOutcome::Succeeded,
            format!("{} attached {device_name}", client.name),
            Some(client.id.clone()),
        );
        self.server.set_attached(bus_id, Some(&client.id));

        // Returns when the client detaches or the link drops; the host then frees the device.
        export.relay(session, abort.clone()).await;

        tracing::info!("Device {bus_id} released by {}", client.id);
        self.activity.record(
            EntityRef::device(bus_id, &device_name),
            ActivityReason::Detached,
            ActivityOutcome::Succeeded,
            format!("{} released {device_name}", client.name),
            Some(client.id.clone()),
        );
        self.server.set_attached(bus_id, None);
    }

    /// Ends every session, telling each client the server no longer serves USB, and waits a
    /// little for them to finish, so nothing outlives its owner.
    async fn end_serving(&self) {
        let ending = std::mem::replace(&mut *lock(&self.serving), CancellationToken::new());
        let running: Vec<CancellationToken> = lock(&self.ended).values().cloned().collect();
        ending.cancel();
        if running.is_empty() {
            return;
        }
        let finished = tokio::time::timeout(STOP_GRACE, async {
            for ended in running {
                ended.cancelled().await;
            }
        })
        .await;
        if finished.is_err() {
            tracing::warn!(
                "{} USB sessions were still running when serving stopped",
                lock(&self.sessions).len()
            );
        }
    }
}

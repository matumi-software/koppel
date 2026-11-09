//! The server role on skakel-server: registered clients, revoking and rotating, the token signing
//! key, the certificate and its handover, pairing requests, Koppel's remote device list and the
//! `usb` streams. Its state is `secrets.json` ([`SecretsStore`]).
//!
//! Skakel's server runs on a loopback port and Koppel's web ports hand it its requests (see
//! [`super::front`]). Leaving the server role starts it again, which ends every client's control
//! call, remote device list and USB stream with `STOPPING`, as 0.1.x ended its hub connections and
//! streams with close code 4003. The UI's server endpoints keep working on the new one.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use koppel_config::ConfigFile;
use koppel_config::files::{KoppelSettings, Role};
use koppel_usb::{RemoteDevices, UsbDataPlane};
use skakel_server::{
    Clients, Keys, Pairing, PairingDecision, ServerEvent, SkakelServer, SkakelServerBuilder,
};
use tokio::sync::watch;

use super::front::Front;
use super::remote;
use super::store::SecretsStore;
use super::streams::{USB_STREAM, usb_handler};
use crate::activity::{ActivityLog, ActivityOutcome, ActivityReason, EntityRef};
use crate::push::{self, Push};
use crate::roles::{
    Approval, ClientView, Fut, KeysView, NewClient, PairingOverview, PairingRequestView,
    RegisterError, RoleError, ServerRole, Switched,
};

/// Skakel's app id for Koppel.
pub(crate) const APP_ID: &str = "koppel";

/// What the server is made with.
pub(crate) struct ServerParts {
    pub name: String,
    pub version: String,
    pub store: Arc<SecretsStore>,
    pub devices: watch::Receiver<Arc<RemoteDevices>>,
    pub data_plane: UsbDataPlane,
    pub push: Push,
    pub activity: Arc<ActivityLog>,
}

/// What the server's events need.
struct Wiring {
    push: Push,
    activity: Arc<ActivityLog>,
    front: Arc<Front>,
    overview: Mutex<PairingOverview>,
    /// The requests a person here refused. 0.1.x's server dropped them from the list at once;
    /// Skakel 0.2.0 shows them as refused for a few seconds, as it does after three wrong codes.
    refused: Mutex<HashSet<String>>,
}

/// The server role. See the module docs.
pub struct SkakelServerRole {
    parts: ServerParts,
    wiring: Arc<Wiring>,
    running: Mutex<Option<SkakelServer>>,
    restarting: tokio::sync::Mutex<()>,
}

impl SkakelServerRole {
    /// Starts Skakel's server on a loopback port over `secrets.json`.
    pub(crate) async fn start(parts: ServerParts) -> Result<Arc<Self>, String> {
        let wiring = Arc::new(Wiring {
            push: parts.push.clone(),
            activity: parts.activity.clone(),
            front: Front::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
            overview: Mutex::new(PairingOverview {
                requests: Vec::new(),
                budget_exhausted: false,
            }),
            refused: Mutex::new(HashSet::new()),
        });
        let server = build(&parts, &wiring).await?;
        wiring.front.retarget(server.local_addr());
        Ok(Arc::new(Self {
            parts,
            wiring,
            running: Mutex::new(Some(server)),
            restarting: tokio::sync::Mutex::new(()),
        }))
    }

    /// Where Koppel's web ports hand Skakel's requests.
    pub(crate) fn front(&self) -> Arc<Front> {
        self.wiring.front.clone()
    }

    /// Starts Skakel's server again whenever the role leaves the server side, which ends what its
    /// clients have open.
    pub(crate) fn follow(self: &Arc<Self>, settings: &ConfigFile<KoppelSettings>) {
        let weak = Arc::downgrade(self);
        let serving = Mutex::new(settings.current().role != Role::Client);
        settings.subscribe(move |value| {
            let weak = weak.clone();
            let now = value.role != Role::Client;
            let was = std::mem::replace(
                &mut *serving.lock().unwrap_or_else(PoisonError::into_inner),
                now,
            );
            async move {
                if was
                    && !now
                    && let Some(role) = weak.upgrade()
                {
                    role.restart().await;
                }
            }
        });
    }

    /// A new server first, then the old one stops, so the UI's calls always find one.
    async fn restart(&self) {
        let _one_at_a_time = self.restarting.lock().await;
        let next = match build(&self.parts, &self.wiring).await {
            Ok(next) => next,
            Err(e) => {
                tracing::error!("Skakel's server could not start again: {e}");
                return;
            }
        };
        self.wiring.front.retarget(next.local_addr());
        let old = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(next);
        // The new server has no pairing requests yet.
        let empty = PairingOverview {
            requests: Vec::new(),
            budget_exhausted: false,
        };
        *self
            .wiring
            .overview
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = empty.clone();
        self.wiring
            .push
            .send(push::PAIRING_REQUESTS_CHANGED, &empty);
        if let Some(old) = old {
            old.shutdown().await;
        }
        tracing::info!("The role left the server side; every client's link and USB stream ended");
    }

    /// Stops Skakel's server, telling the clients it is stopping.
    pub(crate) async fn stop(&self) {
        let old = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(old) = old {
            old.shutdown().await;
        }
    }

    fn with<T>(&self, use_it: impl FnOnce(&SkakelServer) -> T) -> T {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        use_it(
            running
                .as_ref()
                .expect("Skakel's server runs until Koppel stops"),
        )
    }

    fn clients_handle(&self) -> Clients {
        self.with(SkakelServer::clients)
    }

    fn keys_handle(&self) -> Keys {
        self.with(SkakelServer::keys)
    }

    fn pairing_handle(&self) -> Pairing {
        self.with(SkakelServer::pairing)
    }

    /// The clients that had a session open on Skakel's server, for a test to wait on.
    #[doc(hidden)]
    pub fn open_sessions(&self, client_id: &str) -> usize {
        self.with(|server| server.open_sessions(client_id))
    }

    fn push_clients(&self) {
        self.wiring
            .push
            .send(push::CLIENTS_CHANGED, &self.clients());
    }
}

/// Builds and starts Skakel's server. Discovery is Koppel's own (see `announcer.rs`), and its
/// per-address limits are kept on the web ports, where the caller's address is known.
async fn build(parts: &ServerParts, wiring: &Arc<Wiring>) -> Result<SkakelServer, String> {
    let events = Arc::downgrade(wiring);
    SkakelServerBuilder::new(APP_ID, &parts.version, parts.store.clone())
        .name(&parts.name)
        .certificate_host_name(&parts.name)
        .listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .discovery(false)
        .keep_client_secrets(true)
        .token_requests_per_minute(u32::MAX)
        .pair_requests_per_minute(u32::MAX)
        .grpc_service(remote::Service::server(parts.devices.clone()))
        .stream(USB_STREAM, usb_handler(parts.data_plane.clone()))
        .routes(remote::routes(parts.devices.clone()))
        .on_event(move |event| {
            if let Some(wiring) = events.upgrade() {
                wiring.on_event(event);
            }
        })
        .start()
        .await
        .map_err(|e| format!("Skakel's server could not start: {e}"))
}

impl Wiring {
    fn on_event(&self, event: &ServerEvent) {
        match event {
            ServerEvent::Pairing(overview) => {
                let ids: Vec<String> = overview.requests.iter().map(|r| r.id.clone()).collect();
                let refused = {
                    let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
                    refused.retain(|id| ids.contains(id));
                    refused.clone()
                };
                let view = PairingOverview {
                    requests: overview
                        .requests
                        .iter()
                        .filter(|r| !(r.state == "denied" && refused.contains(&r.id)))
                        .map(|r| PairingRequestView {
                            id: r.id.clone(),
                            client_name: r.client_name.clone(),
                            from: self
                                .front
                                .pair_from(&r.id)
                                .unwrap_or_else(|| r.from.clone()),
                            requested_at: r.requested_at,
                            expires_at: r.expires_at,
                            state: r.state.clone(),
                            message: r.message.clone(),
                        })
                        .collect(),
                    budget_exhausted: overview.budget_exhausted,
                };
                *self.overview.lock().unwrap_or_else(PoisonError::into_inner) = view.clone();
                self.front.keep_pair_from(&ids);
                self.push.send(push::PAIRING_REQUESTS_CHANGED, &view);
            }
            ServerEvent::Decided(decision) => self.record(decision),
            ServerEvent::Clients(clients) => {
                let list: Vec<ClientView> = clients.iter().map(client_view).collect();
                self.push.send(push::CLIENTS_CHANGED, &list);
            }
            ServerEvent::Keys(keys) => {
                self.push.send(push::SERVER_KEYS_CHANGED, &keys_view(keys));
            }
        }
    }

    /// Each pairing that was approved or refused goes to the activity log, as 0.1.x's
    /// `PairingActivity` put it there.
    fn record(&self, decision: &PairingDecision) {
        let from = self
            .front
            .pair_from(&decision.request_id)
            .unwrap_or_else(|| decision.from.clone());
        match &decision.client_id {
            Some(client_id) => {
                self.activity.record(
                    EntityRef::new(client_id, &decision.client_name, "client"),
                    ActivityReason::ClientAdded,
                    ActivityOutcome::Succeeded,
                    format!("{} paired from {from}", decision.client_name),
                    None,
                );
            }
            None => {
                self.activity.record(
                    EntityRef::new(&decision.request_id, &decision.client_name, "client"),
                    ActivityReason::ClientAdded,
                    ActivityOutcome::Failed,
                    format!("Pairing with {} was refused", decision.client_name),
                    decision.refusal.clone(),
                );
            }
        }
    }
}

fn client_view(client: &skakel_server::ClientView) -> ClientView {
    ClientView {
        id: client.id.clone(),
        name: client.name.clone(),
        created_at: client.created_at,
        last_seen: client.last_seen,
        has_next_certificate: client.has_next_certificate,
        has_secret: client.has_secret,
    }
}

fn keys_view(keys: &skakel_server::KeysView) -> KeysView {
    KeysView {
        fingerprint: keys.fingerprint.clone(),
        next_fingerprint: keys.next_fingerprint.clone(),
        next_prepared_at: keys.next_prepared_at,
        signing_key_id: keys.signing_key_id.clone(),
        signing_key_since: keys.signing_key_since,
        retired_keys_until: keys.retired_keys_until,
    }
}

fn failed(e: skakel_server::ServerError) -> RoleError {
    RoleError(e.to_string())
}

impl ServerRole for SkakelServerRole {
    /// The certificate the web ports present, which is Skakel's: a save that switches it replaces
    /// both. Read from there, so it answers while Skakel's server starts again or stops.
    fn fingerprint(&self) -> String {
        skakel_protocol::fingerprints::of_der(&self.parts.store.slot().current().cert[0])
    }

    fn clients(&self) -> Vec<ClientView> {
        self.clients_handle()
            .all()
            .iter()
            .map(client_view)
            .collect()
    }

    fn name_of(&self, id: &str) -> Option<String> {
        self.clients_handle().name_of(id)
    }

    fn secret_of(&self, id: &str) -> Option<Option<String>> {
        let clients = self.clients_handle();
        clients.name_of(id)?;
        Some(clients.secret_of(id))
    }

    fn register<'a>(&'a self, name: &'a str) -> Fut<'a, Result<NewClient, RegisterError>> {
        Box::pin(async move {
            if skakel_server::clean_client_name(name).is_empty() {
                return Err(RegisterError::EmptyName);
            }
            let client = self
                .clients_handle()
                .register(name)
                .await
                .map_err(|e| RegisterError::Failed(failed(e)))?;
            Ok(NewClient {
                client_id: client.client_id,
                client_secret: client.client_secret,
                name: client.name,
            })
        })
    }

    fn revoke<'a>(&'a self, id: &'a str) -> Fut<'a, Result<bool, RoleError>> {
        Box::pin(async move { self.clients_handle().revoke(id).await.map_err(failed) })
    }

    fn rotate_secret<'a>(&'a self, id: &'a str) -> Fut<'a, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            let secret = self
                .clients_handle()
                .rotate_secret(id)
                .await
                .map_err(failed)?;
            if secret.is_some() {
                // Nothing the list shows changes, so Skakel does not say; 0.1.x pushed it anyway.
                self.push_clients();
            }
            Ok(secret)
        })
    }

    fn keys(&self) -> KeysView {
        keys_view(&self.keys_handle().view())
    }

    fn rotate_signing_key(&self) -> Fut<'_, Result<String, RoleError>> {
        Box::pin(async move {
            self.keys_handle()
                .rotate_signing_key()
                .await
                .map_err(failed)
        })
    }

    fn prepare_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            self.keys_handle()
                .prepare_next_certificate()
                .await
                .map_err(failed)
        })
    }

    fn cancel_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            let keys = self.keys_handle();
            let Some(next) = keys.next_fingerprint() else {
                return Ok(None);
            };
            let cancelled = keys.cancel_next_certificate().await.map_err(failed)?;
            Ok(cancelled.then_some(next))
        })
    }

    fn switch_certificate(&self) -> Fut<'_, Result<Option<Switched>, RoleError>> {
        Box::pin(async move {
            let switched = self
                .keys_handle()
                .switch_certificate()
                .await
                .map_err(failed)?;
            Ok(switched.map(|s| Switched {
                to: s.to,
                not_received: s.not_received,
            }))
        })
    }

    fn pairing(&self) -> PairingOverview {
        self.wiring
            .overview
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn approve<'a>(&'a self, id: &'a str, code: &'a str) -> Fut<'a, Approval> {
        Box::pin(async move {
            match self.pairing_handle().approve(id, code).await {
                Ok(skakel_server::Approval::Approved) => Approval::Approved,
                Ok(skakel_server::Approval::WrongCode) => Approval::WrongCode,
                Ok(skakel_server::Approval::NotWaiting) => Approval::NotFound,
                Err(e) => {
                    tracing::error!("Approving a pairing request failed: {e}");
                    Approval::NotFound
                }
            }
        })
    }

    fn deny<'a>(&'a self, id: &'a str) -> Fut<'a, bool> {
        Box::pin(async move {
            // Noted first: the list Skakel raises while it refuses leaves the request out already.
            self.wiring
                .refused
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.to_owned());
            let denied = self.pairing_handle().deny(id).await;
            if !denied {
                self.wiring
                    .refused
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(id);
            }
            denied
        })
    }
}

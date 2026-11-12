//! The server role on skakel-server: registered clients, revoking and rotating, the certificate and
//! its handover, the pairing window and its requests, Koppel's remote device list and the `usb`
//! streams. Its state is `secrets.json` ([`SecretsStore`]).
//!
//! Skakel's server is mounted on Koppel's web ports: it binds nothing, and the web ports hand it what
//! is Skakel's after their port and role check ([`SkakelServerRole::serve`]), with the caller's real
//! address, and shake hands with its TLS settings while the role is server or both
//! ([`SkakelServerRole::acceptor`]). Leaving the server role starts it again, which ends every
//! client's control call, remote device list and USB stream with `STOPPING`, as 0.1.x ended its hub
//! connections and streams with close code 4003. New requests then go to the new server's service,
//! new connections take its TLS settings once the role is server or both again, and the UI's server
//! endpoints keep working on it.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use koppel_config::ConfigFile;
use koppel_config::files::{KoppelSettings, Role};
use koppel_usb::{RemoteDevices, UsbDataPlane};
use skakel_server::{
    Certificates, Clients, Pairing, PairingDecision, PairingWindow, ServerEvent, ServerEvents,
    SkakelServer, SkakelServerBuilder,
};
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

use super::remote;
use super::store::SecretsStore;
use super::streams::{USB_STREAM, usb_handler};
use crate::activity::{ActivityLog, ActivityOutcome, ActivityReason, EntityRef};
use crate::push::{self, Push};
use crate::roles::{
    Approval, ClientView, Fut, KeysView, NewClient, PairingOverview, PairingRequestView,
    PairingWindowView, RegisterError, RoleError, ServerRole, Switched,
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
    /// The address the network surface's port is bound to, which Skakel's server reports as its own.
    pub listen: SocketAddr,
    /// `RateLimits:TokenPerMinute`, per caller's address.
    pub token_per_minute: u32,
    /// `RateLimits:PairPerMinute`, per caller's address.
    pub pair_per_minute: u32,
}

/// What the server's events need.
struct Wiring {
    push: Push,
    activity: Arc<ActivityLog>,
    overview: Mutex<PairingOverview>,
    /// The requests a person here refused. 0.1.x's server dropped them from the list at once;
    /// Skakel 0.2.0 shows them as refused for a few seconds, as it does after three wrong codes.
    refused: Mutex<HashSet<String>>,
}

/// A running server and the task that turns its events into pushes.
struct Running {
    server: SkakelServer,
    events: JoinHandle<()>,
}

impl Running {
    /// Ends the server and, once it is stopped, the task that read its events.
    async fn shut_down(self) {
        self.server.shutdown().await;
        self.events.abort();
    }
}

/// The server role. See the module docs.
pub struct SkakelServerRole {
    parts: ServerParts,
    wiring: Arc<Wiring>,
    running: Mutex<Option<Running>>,
    restarting: tokio::sync::Mutex<()>,
}

impl SkakelServerRole {
    /// Starts Skakel's server over `secrets.json`, mounted on the web ports.
    pub(crate) async fn start(parts: ServerParts) -> Result<Arc<Self>, String> {
        let wiring = Arc::new(Wiring {
            push: parts.push.clone(),
            activity: parts.activity.clone(),
            overview: Mutex::new(PairingOverview::empty()),
            refused: Mutex::new(HashSet::new()),
        });
        let running = build(&parts, &wiring).await?;
        Ok(Arc::new(Self {
            parts,
            wiring,
            running: Mutex::new(Some(running)),
            restarting: tokio::sync::Mutex::new(()),
        }))
    }

    /// Hands a request the web ports' guards let through to the running server's service: a gRPC
    /// call, a path under `/skakel` or `/api/remote`. The request carries the caller's address
    /// (`ConnectInfo`), which Skakel's per-address limits and a pairing request's address use. The
    /// server that runs now answers it, so after a restart new requests reach the new one.
    pub(crate) async fn serve(&self, request: Request) -> Response {
        let service = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|running| running.server.service());
        let Some(mut service) = service else {
            return not_running(super::is_grpc(&request));
        };
        let Ok(response) = service.call(request).await;
        response
    }

    /// The TLS acceptor of the running server: its certificate on every handshake, so a switch shows
    /// from the next connection on and a prepared certificate never shows before it, ALPN `h2` and
    /// `http/1.1`, and no session resumption. Built from the server that runs now, so a restart
    /// moves new connections to the new server's.
    pub(crate) fn acceptor(&self) -> Option<TlsAcceptor> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|running| TlsAcceptor::from(running.server.tls_config()))
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
        // From here new connections and requests reach the new server; the old one's calls end
        // with STOPPING when it shuts down.
        let old = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(next);
        // The new server has no pairing requests yet, and its pairing window is closed.
        let empty = PairingOverview::empty();
        *self
            .wiring
            .overview
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = empty.clone();
        self.wiring
            .push
            .send(push::PAIRING_REQUESTS_CHANGED, &empty);
        if let Some(old) = old {
            old.shut_down().await;
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
            old.shut_down().await;
        }
    }

    fn with<T>(&self, use_it: impl FnOnce(&SkakelServer) -> T) -> T {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        use_it(
            &running
                .as_ref()
                .expect("Skakel's server runs until Koppel stops")
                .server,
        )
    }

    fn clients_handle(&self) -> Clients {
        self.with(SkakelServer::clients)
    }

    fn certificates_handle(&self) -> Certificates {
        self.with(SkakelServer::certificates)
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

/// Builds and starts Skakel's server, mounted: it binds nothing, and the web ports serve it.
/// Discovery is Koppel's own (see `announcer.rs`).
async fn build(parts: &ServerParts, wiring: &Arc<Wiring>) -> Result<Running, String> {
    let server = SkakelServerBuilder::new(APP_ID, &parts.version, parts.store.clone())
        .name(&parts.name)
        .certificate_host_name(&parts.name)
        .discovery(false)
        .keep_client_secrets(true)
        .token_requests_per_minute(parts.token_per_minute)
        .pair_requests_per_minute(parts.pair_per_minute)
        .grpc_service(remote::Service::server(parts.devices.clone()))
        .stream(USB_STREAM, usb_handler(parts.data_plane.clone()))
        .routes(remote::routes(parts.devices.clone()))
        .start_mounted(parts.listen)
        .await
        .map_err(|e| format!("Skakel's server could not start: {e}"))?;
    let events = tokio::spawn(read_events(server.subscribe(), Arc::downgrade(wiring)));
    Ok(Running { server, events })
}

/// Turns what the server tells into pushes, on a task of Koppel's own, until the server is gone.
async fn read_events(mut events: ServerEvents, wiring: std::sync::Weak<Wiring>) {
    while let Some(event) = events.recv().await {
        let Some(wiring) = wiring.upgrade() else {
            return;
        };
        wiring.on_event(&event);
    }
}

/// What a caller gets once Skakel's server has stopped with Koppel: gRPC's `UNAVAILABLE` for a
/// call, 503 for the rest.
fn not_running(grpc: bool) -> Response {
    const WHY: &str = "Skakel's server is not running.";
    if grpc {
        return tonic::Status::unavailable(WHY).into_http();
    }
    (StatusCode::SERVICE_UNAVAILABLE, WHY).into_response()
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
                            from: r.from.clone(),
                            requested_at: r.requested_at,
                            expires_at: r.expires_at,
                            state: r.state.clone(),
                            message: r.message.clone(),
                        })
                        .collect(),
                    budget_exhausted: overview.budget_exhausted,
                    window: self.current_window(),
                };
                *self.overview.lock().unwrap_or_else(PoisonError::into_inner) = view.clone();
                self.push.send(push::PAIRING_REQUESTS_CHANGED, &view);
            }
            ServerEvent::PairingWindow(window) => {
                let view = {
                    let mut overview = self.overview.lock().unwrap_or_else(PoisonError::into_inner);
                    overview.window = window_view(window);
                    overview.clone()
                };
                self.push.send(push::PAIRING_REQUESTS_CHANGED, &view);
            }
            ServerEvent::Decided(decision) => self.record(decision),
            ServerEvent::Clients(clients) => {
                let list: Vec<ClientView> = clients.iter().map(client_view).collect();
                self.push.send(push::CLIENTS_CHANGED, &list);
            }
            ServerEvent::Certificates(certificates) => {
                self.push
                    .send(push::SERVER_KEYS_CHANGED, &keys_view(certificates));
            }
            // Events a later Skakel adds say nothing the UI shows.
            _ => {}
        }
    }

    fn current_window(&self) -> PairingWindowView {
        self.overview
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .window
            .clone()
    }

    /// Each pairing that was approved or refused goes to the activity log, as 0.1.x's
    /// `PairingActivity` put it there.
    fn record(&self, decision: &PairingDecision) {
        let from = &decision.from;
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

fn keys_view(keys: &skakel_server::CertificatesView) -> KeysView {
    KeysView {
        fingerprint: keys.fingerprint.clone(),
        next_fingerprint: keys.next_fingerprint.clone(),
        next_prepared_at: keys.next_prepared_at,
    }
}

/// The window as the UI shows it: the moment it closes, not the time left, so the UI can count
/// down without being told again.
fn window_view(window: &PairingWindow) -> PairingWindowView {
    PairingWindowView {
        open: window.open,
        always_open: window.always_open,
        until: window
            .remaining
            .map(|left| OffsetDateTime::now_utc() + left),
    }
}

fn failed(e: skakel_server::ServerError) -> RoleError {
    RoleError(e.to_string())
}

impl ServerRole for SkakelServerRole {
    /// The certificate the web ports present, which is Skakel's: a save that switches it replaces
    /// both. Read from there, so it answers while Skakel's server starts again or stops.
    fn fingerprint(&self) -> String {
        skakel_wire::fingerprints::of_der(&self.parts.store.slot().current().cert[0])
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
            if skakel_wire::pairing::clean_client_name(name).is_empty() {
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
        keys_view(&self.certificates_handle().view())
    }

    fn prepare_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            self.certificates_handle()
                .prepare_next_certificate()
                .await
                .map_err(failed)
        })
    }

    fn cancel_next_certificate(&self) -> Fut<'_, Result<Option<String>, RoleError>> {
        Box::pin(async move {
            let certificates = self.certificates_handle();
            let Some(next) = certificates.next_fingerprint() else {
                return Ok(None);
            };
            let cancelled = certificates
                .cancel_next_certificate()
                .await
                .map_err(failed)?;
            Ok(cancelled.then_some(next))
        })
    }

    fn switch_certificate(&self) -> Fut<'_, Result<Option<Switched>, RoleError>> {
        Box::pin(async move {
            let switched = self
                .certificates_handle()
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

    fn open_pairing(&self, duration: Duration) {
        self.with(|server| server.open_pairing(duration));
    }

    fn close_pairing(&self) {
        self.with(SkakelServer::close_pairing);
    }

    fn approve<'a>(&'a self, id: &'a str, code: &'a str) -> Fut<'a, Approval> {
        Box::pin(async move {
            match self.pairing_handle().approve(id, code).await {
                Ok(skakel_server::Approval::Approved) => Approval::Approved,
                Ok(skakel_server::Approval::WrongCode) => Approval::WrongCode,
                Ok(_) => Approval::NotFound,
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

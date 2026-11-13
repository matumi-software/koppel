//! The client role on skakel-client: the connection test, the servers found on the local network,
//! and pairing by the eight-digit code, as 0.1.x's `RemoteServer`, `ServerFinder` and
//! `PairingClient` did them. The link itself is koppel-usb's client over [`super::links`].

use std::sync::{Arc, Weak};
use std::time::Duration;

use koppel_config::ConfigFile;
use koppel_config::files::{ClientConnection, KoppelSettings, Role};
use skakel_client::{
    Browser, ClientOptions, Credentials, FailureKind, PairingClient, PairingError, Pins,
    StatusCode, Url,
};
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::connection::ConnectionUpdate;
use crate::push::{Event, Push};
use crate::roles::{ConnectionTest, DiscoveredServer, Outcome, PairingState, PairingStatus};
use crate::tasks::Tasks;

/// How long the connection test may take, the token request and the hello together.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Generous: a server on slow hardware can take many seconds over its first request after a start.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(60);

/// The client role. See the module docs.
pub struct SkakelClientRole {
    connection: ConfigFile<ClientConnection>,
    push: Push,
    browser: Option<Browser>,
    /// This machine's own server certificate, which discovery leaves out.
    own: Arc<dyn Fn() -> String + Send + Sync>,
    /// The name the server's people see this machine as.
    machine: String,
    tasks: Tasks,
    /// The pairing, with one owner: every change goes through this watch.
    pairing: watch::Sender<Pairings>,
    me: Weak<Self>,
}

/// Where the pairing stands. Idle until a run starts.
struct Pairings {
    run: Option<Arc<Run>>,
    status: PairingStatus,
    /// What the server approved, which waits for the person to confirm or has been saved.
    approved: Option<ConnectionUpdate>,
    /// The person confirmed and the save is under way.
    confirming: bool,
    next: u64,
}

impl Default for Pairings {
    fn default() -> Self {
        Self {
            run: None,
            status: PairingStatus::idle(),
            approved: None,
            confirming: false,
            next: 0,
        }
    }
}

impl Pairings {
    /// Back to idle; run numbers keep counting so an old run can never match a new one.
    fn reset(&mut self) {
        *self = Self {
            next: self.next,
            ..Self::default()
        };
    }
}

/// One pairing attempt. Its identity decides who may change the pairing.
struct Run {
    id: u64,
    server_url: String,
    fingerprint: String,
    server_name: String,
    stop: CancellationToken,
}

impl SkakelClientRole {
    pub fn new(
        connection: ConfigFile<ClientConnection>,
        push: Push,
        browser: Option<Browser>,
        own: Arc<dyn Fn() -> String + Send + Sync>,
        machine: String,
        tasks: Tasks,
    ) -> Arc<Self> {
        let role = Arc::new_cyclic(|me| Self {
            connection: connection.clone(),
            push,
            browser,
            own,
            machine,
            tasks,
            pairing: watch::channel(Pairings::default()).0,
            me: me.clone(),
        });
        let weak = Arc::downgrade(&role);
        connection.subscribe(move |saved| {
            let weak = weak.clone();
            async move {
                if let Some(role) = weak.upgrade() {
                    role.on_connection_changed(&saved);
                }
            }
        });
        role
    }

    /// Looks for servers while this machine can be a client, and pushes the list as it changes.
    pub async fn follow(self: &Arc<Self>, settings: &ConfigFile<KoppelSettings>) {
        let Some(browser) = self.browser.clone() else {
            return;
        };
        apply_browsing(&browser, settings.current().role).await;
        {
            let browser = browser.clone();
            settings.subscribe(move |value| {
                let browser = browser.clone();
                async move { apply_browsing(&browser, value.role).await }
            });
        }
        let mut changes = browser.watch();
        let weak: Weak<Self> = Arc::downgrade(self);
        self.tasks.spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(role) = weak.upgrade() else {
                    return;
                };
                role.push.send(Event::ServersDiscovered(&role.discovered()));
            }
        });
    }

    /// Whether `run` is still the pairing's run.
    fn is_current(&self, run: &Run) -> bool {
        self.pairing
            .borrow()
            .run
            .as_ref()
            .is_some_and(|r| r.id == run.id)
    }

    /// Sets the status for a run, unless another run or a cancel has taken over since.
    fn set(&self, run: &Run, status: PairingStatus) {
        self.set_with(run, status, |_| {});
    }

    /// Sets the status for a run and changes the rest of the pairing in the same step.
    fn set_with(&self, run: &Run, status: PairingStatus, also: impl FnOnce(&mut Pairings)) {
        let pushed = status.clone();
        let changed = self.pairing.send_if_modified(|pairings| {
            if pairings.run.as_ref().is_none_or(|r| r.id != run.id) {
                return false;
            }
            pairings.status = status;
            also(pairings);
            true
        });
        if changed {
            self.push.send(Event::PairingStatusChanged(&pushed));
        }
    }

    fn status(run: &Run, state: PairingState) -> PairingStatus {
        PairingStatus {
            state,
            server_name: Some(run.server_name.clone()),
            server_url: Some(run.server_url.clone()),
            code: None,
            message: None,
            expires_at: None,
            server_fingerprint: Some(run.fingerprint.clone()),
        }
    }

    async fn run(self: Arc<Self>, run: Arc<Run>) {
        let mut status = Self::status(&run, PairingState::Requesting);
        let address = match Url::parse(&run.server_url) {
            Ok(address) => address,
            Err(e) => {
                status.state = PairingState::Failed;
                status.message = Some(format!("Could not pair with {}: {e}", run.server_name));
                self.set(&run, status);
                return;
            }
        };
        let options = ClientOptions::default().request_timeout(PAIRING_TIMEOUT);
        let opened = tokio::select! {
            () = run.stop.cancelled() => return,
            opened = PairingClient::open(&address, &run.fingerprint, &self.machine, &options) => opened,
        };
        let request = match opened {
            Ok(request) => request,
            Err(PairingError::InvalidClientName) => {
                status.state = PairingState::Failed;
                status.message = Some(format!(
                    "Could not pair with {}: this machine's name is not one a server takes.",
                    run.server_name
                ));
                self.set(&run, status);
                return;
            }
            Err(PairingError::Closed) => {
                status.state = PairingState::Failed;
                status.message = Some(closed_message(&run.server_name));
                self.set(&run, status);
                return;
            }
            Err(PairingError::Failed(error)) => {
                tracing::warn!(
                    "Pairing with {} failed: {}",
                    run.server_name,
                    error.failure().code()
                );
                self.set(&run, failed(&run, status, &error));
                return;
            }
            Err(error) => {
                tracing::warn!("Pairing with {} failed: {error}", run.server_name);
                status.state = PairingState::Failed;
                status.message = Some(format!("Could not pair with {}: {error}", run.server_name));
                self.set(&run, status);
                return;
            }
        };
        status.state = PairingState::Waiting;
        status.code = Some(request.code().to_owned());
        status.expires_at = Some(OffsetDateTime::from(request.expires_at()));
        self.set(&run, status.clone());
        tracing::info!("Waiting for the code to be typed on {}", run.server_name);

        let credentials = tokio::select! {
            () = run.stop.cancelled() => {
                // Cancelled or replaced: withdraw it so it does not sit on the server's Clients page.
                request.withdraw().await;
                return;
            }
            waited = request.wait() => waited,
        };
        match credentials {
            Ok(credentials) => self.approved(&run, status, credentials).await,
            Err(error) => {
                tracing::warn!(
                    "Pairing with {} failed: {}",
                    run.server_name,
                    error.failure().code()
                );
                self.set(&run, failed(&run, status, &error));
            }
        }
    }

    /// The server approved. The credentials become the connection as they are: saved at once for
    /// a client with no connection or one for the same server, otherwise held until the person
    /// confirms.
    async fn approved(&self, run: &Run, mut status: PairingStatus, credentials: Credentials) {
        if !self.is_current(run) {
            return;
        }
        let update = ConnectionUpdate::paired(
            &run.server_url,
            &run.fingerprint,
            &credentials.client_id,
            &credentials.client_secret,
        );
        let existing = self.connection.current();
        status.code = None;
        if (existing.server_url.is_some() || existing.client_id.is_some())
            && !update.keeps_server(&existing)
        {
            tracing::info!(
                "{} approved; waiting for the person to confirm replacing the connection",
                run.server_name
            );
            status.state = PairingState::Confirm;
            status.message = Some(format!(
                "{} approved this machine. Saving replaces your connection to {}.",
                run.server_name,
                existing.server_url.as_deref().unwrap_or_default()
            ));
            self.set_with(run, status, |p| p.approved = Some(update));
            return;
        }
        if let Err(e) = self.connection.update(|c| update.apply(c)).await {
            tracing::warn!("Pairing with {} failed: {e}", run.server_name);
            status.state = PairingState::Failed;
            status.message = Some(format!(
                "Could not save the connection to {}: {e}",
                run.server_name
            ));
            self.set(run, status);
            return;
        }
        tracing::info!("Paired with {}", run.server_name);
        self.set_with(run, Self::status(run, PairingState::Paired), |p| {
            p.approved = Some(update);
        });
    }

    /// "Paired" holds only while the saved connection is the one it made.
    fn on_connection_changed(&self, saved: &ClientConnection) {
        let reset = self.pairing.send_if_modified(|pairings| {
            if pairings.run.is_none() {
                return false;
            }
            let paired = pairings.status.state == PairingState::Paired;
            let still = pairings
                .approved
                .as_ref()
                .is_some_and(|made| made.matches(saved));
            if !paired || still {
                return false;
            }
            pairings.reset();
            true
        });
        if reset {
            self.push
                .send(Event::PairingStatusChanged(&PairingStatus::idle()));
        }
    }
}

/// Browses while the role can be a client.
async fn apply_browsing(browser: &Browser, role: Role) {
    if role == Role::Server {
        browser.stop();
    } else if let Err(e) = browser.start().await {
        tracing::warn!(
            "Could not look for servers on the local network ({e}); enter the server's address instead"
        );
    }
}

/// Skakel's failure as the status the UI shows.
fn failed(run: &Run, mut status: PairingStatus, error: &skakel_client::Error) -> PairingStatus {
    status.code = None;
    let name = &run.server_name;
    let (state, message) = match error.kind() {
        FailureKind::PairingDenied => {
            (PairingState::Denied, format!("{name} refused the request."))
        }
        FailureKind::PairingExpired => {
            (PairingState::Expired, error.failure().message().to_owned())
        }
        FailureKind::TooManyRequests => (
            PairingState::Failed,
            format!(
                "{name} is taking no more pairing requests for now. Try again in a few minutes."
            ),
        ),
        FailureKind::PairingClosed => (PairingState::Failed, closed_message(name)),
        // Skakel's words name both versions and say which end to update: a Koppel 0.2.0 server
        // speaks protocol 2, this one protocol 3, and they do not pair.
        FailureKind::VersionMismatch => (
            PairingState::Failed,
            format!(
                "Could not pair with {name}: {} Koppel 0.2.0 speaks Skakel 2 and Koppel 0.2.1 speaks Skakel 3, so update both machines to the same release.",
                error.failure().message()
            ),
        ),
        // A machine that is a client only answers 404 to everything a pairing client asks.
        FailureKind::UnexpectedAnswer if answered(error) == Some(StatusCode::NOT_FOUND) => (
            PairingState::Failed,
            format!("{name} does not take pairing requests. It may be set up as a client only."),
        ),
        _ => (
            PairingState::Failed,
            format!("Could not pair with {name}: {}", error.failure().message()),
        ),
    };
    status.state = state;
    status.message = Some(message);
    status
}

/// What a server that is not taking pairing requests is told to do about it.
fn closed_message(name: &str) -> String {
    format!(
        "{name} is not taking pairing requests. On {name}, open the Clients section and choose Allow pairing, then try again."
    )
}

/// The HTTP status the server answered with, when skakel-client kept one. 0.3.0 keeps it only in
/// its error's cause (`StatusError`, private) and shows it as "The server answered 404.", so this
/// is the one place that reads the number back out of that sentence. A typed status from Skakel
/// would replace it.
fn answered(error: &skakel_client::Error) -> Option<StatusCode> {
    let mut cause = std::error::Error::source(error);
    while let Some(e) = cause {
        let text = e.to_string();
        if let Some(code) = text
            .strip_prefix("The server answered ")
            .and_then(|rest| rest.strip_suffix('.'))
            .and_then(|code| code.parse::<u16>().ok())
        {
            return StatusCode::from_u16(code).ok();
        }
        cause = e.source();
    }
    None
}

/// Skakel's failure as one of the outcomes, as 0.1.x's `ConnectionTestResult.From`. A version
/// mismatch keeps Skakel's wording, which names both sides' versions; a revoked or rotated client
/// reads as one the server no longer knows; anything a test cannot run into counts as unreachable.
fn tested(failure: &skakel_client::Failure) -> ConnectionTest {
    match failure.kind() {
        FailureKind::CertificateMismatch => ConnectionTest::of(Outcome::CertificateMismatch),
        FailureKind::RejectedCredentials => ConnectionTest::of(Outcome::RejectedCredentials),
        FailureKind::ClientUnknown | FailureKind::Revoked | FailureKind::Rotated => {
            ConnectionTest::of(Outcome::ClientUnknown)
        }
        FailureKind::VersionMismatch => {
            ConnectionTest::saying(Outcome::VersionMismatch, failure.message())
        }
        FailureKind::UnexpectedAnswer => ConnectionTest::of(Outcome::UnexpectedAnswer),
        FailureKind::TooManyRequests => ConnectionTest::of(Outcome::TooManyRequests),
        _ => ConnectionTest::of(Outcome::Unreachable),
    }
}

impl SkakelClientRole {
    pub async fn test(&self) -> ConnectionTest {
        let saved = self.connection.current();
        if !saved.is_complete() {
            return ConnectionTest::of(Outcome::Incomplete);
        }
        let Some(address) = saved.server_url.as_deref().and_then(|u| Url::parse(u).ok()) else {
            return ConnectionTest::of(Outcome::Unreachable);
        };
        let Ok(pins) = Pins::with_next(
            saved.server_fingerprint.as_deref().unwrap_or_default(),
            saved.next_server_fingerprint.as_deref(),
        ) else {
            return ConnectionTest::of(Outcome::CertificateMismatch);
        };
        let credentials = Credentials::new(
            saved.client_id.clone().unwrap_or_default(),
            saved.client_secret.as_deref().cloned().unwrap_or_default(),
        );
        let options = ClientOptions::default().request_timeout(TEST_TIMEOUT);
        let link = match skakel_client::Link::new(&address, pins, credentials, options) {
            Ok(link) => link,
            Err(e) => return tested(e.failure()),
        };
        match link.hello().await {
            Ok(hello) => ConnectionTest::saying(
                Outcome::Ok,
                format!("Connected to {} as {}.", hello.server, hello.client_id),
            ),
            Err(e) => tested(e.failure()),
        }
    }

    pub fn discovered(&self) -> Vec<DiscoveredServer> {
        let Some(browser) = &self.browser else {
            return Vec::new();
        };
        let own = (self.own)();
        let own = digits(&own);
        browser
            .servers()
            .into_iter()
            .filter(|s| digits(&s.fingerprint) != own)
            .map(|s| {
                let url = s.url();
                DiscoveredServer {
                    name: s.name.clone(),
                    address: s.address.to_string(),
                    https_port: s.port,
                    fingerprint: s.fingerprint.clone(),
                    version: Some(s.app.version.clone()),
                    other_addresses: s
                        .other_endpoints
                        .iter()
                        .map(|e| {
                            if e.port() == s.port {
                                e.ip().to_string()
                            } else {
                                e.to_string()
                            }
                        })
                        .collect(),
                    url: url.as_str().trim_end_matches('/').to_owned(),
                }
            })
            .collect()
    }

    pub fn pairing(&self) -> PairingStatus {
        self.pairing.borrow().status.clone()
    }

    pub async fn begin_pairing(
        &self,
        server_url: &str,
        fingerprint: &str,
        server_name: Option<&str>,
    ) {
        let Some(me) = self.myself() else {
            return;
        };
        let server_name = server_name
            .filter(|n| !n.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                Url::parse(server_url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| server_url.to_owned())
            });
        let mut started = None;
        self.pairing.send_modify(|pairings| {
            pairings.next += 1;
            let run = Arc::new(Run {
                id: pairings.next,
                server_url: server_url.to_owned(),
                fingerprint: crate::connection::normalize(fingerprint),
                server_name,
                stop: CancellationToken::new(),
            });
            pairings.approved = None;
            pairings.confirming = false;
            // The status the request answers 202 for is already what a read shows: the run is
            // requesting from this moment, not from when its task first gets to run.
            pairings.status = Self::status(&run, PairingState::Requesting);
            if let Some(previous) = pairings.run.replace(run.clone()) {
                previous.stop.cancel();
            }
            started = Some(run);
        });
        let Some(run) = started else {
            return;
        };
        self.push.send(Event::PairingStatusChanged(&Self::status(
            &run,
            PairingState::Requesting,
        )));
        self.tasks.spawn(me.run(run));
    }

    pub async fn confirm_pairing(&self) -> bool {
        let mut taken = None;
        self.pairing.send_if_modified(|pairings| {
            if pairings.status.state == PairingState::Confirm
                && !pairings.confirming
                && let (Some(run), Some(update)) = (pairings.run.clone(), pairings.approved.clone())
            {
                pairings.confirming = true;
                taken = Some((run, update));
            }
            false
        });
        let Some((run, update)) = taken else {
            return false;
        };
        if let Err(e) = self.connection.update(|c| update.apply(c)).await {
            tracing::warn!("Pairing with {} failed: {e}", run.server_name);
            let mut status = Self::status(&run, PairingState::Failed);
            status.message = Some(format!(
                "Could not save the connection to {}: {e}",
                run.server_name
            ));
            self.set(&run, status);
            return true;
        }
        tracing::info!("Paired with {}", run.server_name);
        self.set(&run, Self::status(&run, PairingState::Paired));
        true
    }

    pub async fn cancel_pairing(&self) {
        self.pairing.send_modify(|pairings| {
            if let Some(run) = pairings.run.take() {
                run.stop.cancel();
            }
            pairings.reset();
        });
        self.push
            .send(Event::PairingStatusChanged(&PairingStatus::idle()));
    }
}

impl SkakelClientRole {
    /// This role as an `Arc`, for a run that outlives the request that started it.
    fn myself(&self) -> Option<Arc<Self>> {
        self.me.upgrade()
    }
}

fn digits(fingerprint: &str) -> String {
    fingerprint
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

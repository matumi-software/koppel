//! The client role on skakel-client: the connection test, the servers found on the local network,
//! and pairing by the eight-digit code, as 0.1.x's `RemoteServer`, `ServerFinder` and
//! `PairingClient` did them. The link itself is koppel-usb's client over [`super::links`].

use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use koppel_config::ConfigFile;
use koppel_config::files::{ClientConnection, KoppelSettings, Role};
use skakel_client::{
    Browser, ClientOptions, Credentials, FailureKind, PairingClient, PairingError, Pins, Url,
};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;

use crate::api::client::ConnectionUpdate;
use crate::push::{self, Push};
use crate::roles::{ClientRole, ConnectionTest, DiscoveredServer, Fut, PairingStatus};

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
    pairing: Mutex<Pairings>,
    me: Weak<Self>,
}

#[derive(Default)]
struct Pairings {
    run: Option<Arc<Run>>,
    status: Option<PairingStatus>,
    next: u64,
}

/// One pairing attempt. Its identity decides who may set the status; what the server approved
/// waits here for the person to confirm.
struct Run {
    id: u64,
    server_url: String,
    fingerprint: String,
    server_name: String,
    stop: CancellationToken,
    approved: Mutex<Option<ConnectionUpdate>>,
    confirming: Mutex<bool>,
}

impl SkakelClientRole {
    pub(crate) fn new(
        connection: ConfigFile<ClientConnection>,
        push: Push,
        browser: Option<Browser>,
        own: Arc<dyn Fn() -> String + Send + Sync>,
        machine: String,
    ) -> Arc<Self> {
        let role = Arc::new_cyclic(|me| Self {
            connection: connection.clone(),
            push,
            browser,
            own,
            machine,
            pairing: Mutex::new(Pairings::default()),
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
    pub(crate) async fn follow(self: &Arc<Self>, settings: &ConfigFile<KoppelSettings>) {
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
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(role) = weak.upgrade() else {
                    return;
                };
                role.push.send(push::SERVERS_DISCOVERED, &role.discovered());
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Pairings> {
        self.pairing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the status for a run, unless another run or a cancel has taken over since.
    fn set(&self, run: &Run, status: PairingStatus) {
        {
            let mut pairings = self.lock();
            if pairings.run.as_ref().is_none_or(|r| r.id != run.id) {
                return;
            }
            pairings.status = Some(status.clone());
        }
        self.push.send(push::PAIRING_STATUS_CHANGED, &status);
    }

    fn status(run: &Run, state: &str) -> PairingStatus {
        PairingStatus {
            state: state.to_owned(),
            server_name: Some(run.server_name.clone()),
            server_url: Some(run.server_url.clone()),
            code: None,
            message: None,
            expires_at: None,
            server_fingerprint: Some(run.fingerprint.clone()),
        }
    }

    async fn run(self: Arc<Self>, run: Arc<Run>) {
        let mut status = Self::status(&run, "requesting");
        self.set(&run, status.clone());
        let address = match Url::parse(&run.server_url) {
            Ok(address) => address,
            Err(e) => {
                status.state = "failed".into();
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
                status.state = "failed".into();
                status.message = Some(format!(
                    "Could not pair with {}: this machine's name is not one a server takes.",
                    run.server_name
                ));
                self.set(&run, status);
                return;
            }
            Err(PairingError::Closed) => {
                status.state = "failed".into();
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
                status.state = "failed".into();
                status.message = Some(format!("Could not pair with {}: {error}", run.server_name));
                self.set(&run, status);
                return;
            }
        };
        status.state = "waiting".into();
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
        if self.lock().run.as_ref().is_none_or(|r| r.id != run.id) {
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
            *run.approved.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
            tracing::info!(
                "{} approved; waiting for the person to confirm replacing the connection",
                run.server_name
            );
            status.state = "confirm".into();
            status.message = Some(format!(
                "{} approved this machine. Saving replaces your connection to {}.",
                run.server_name,
                existing.server_url.as_deref().unwrap_or_default()
            ));
            self.set(run, status);
            return;
        }
        if let Err(e) = self.connection.update(|c| update.apply(c)).await {
            tracing::warn!("Pairing with {} failed: {e}", run.server_name);
            status.state = "failed".into();
            status.message = Some(format!(
                "Could not save the connection to {}: {e}",
                run.server_name
            ));
            self.set(run, status);
            return;
        }
        *run.approved.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
        tracing::info!("Paired with {}", run.server_name);
        self.set(run, Self::status(run, "paired"));
    }

    /// "Paired" holds only while the saved connection is the one it made.
    fn on_connection_changed(&self, saved: &ClientConnection) {
        {
            let mut pairings = self.lock();
            let Some(run) = pairings.run.clone() else {
                return;
            };
            let paired = pairings
                .status
                .as_ref()
                .is_some_and(|s| s.state == "paired");
            let made = run
                .approved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let still = made.is_some_and(|made| made.matches(saved));
            if !paired || still {
                return;
            }
            pairings.run = None;
            pairings.status = None;
        }
        self.push
            .send(push::PAIRING_STATUS_CHANGED, &PairingStatus::idle());
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
        FailureKind::PairingDenied => ("denied", format!("{name} refused the request.")),
        FailureKind::PairingExpired => ("expired", error.failure().message().to_owned()),
        FailureKind::TooManyRequests => (
            "failed",
            format!(
                "{name} is taking no more pairing requests for now. Try again in a few minutes."
            ),
        ),
        FailureKind::PairingClosed => ("failed", closed_message(name)),
        // Skakel's words name both versions and say which end to update: a Koppel 0.2.0 server
        // speaks protocol 2, this one protocol 3, and they do not pair.
        FailureKind::VersionMismatch => (
            "failed",
            format!(
                "Could not pair with {name}: {} Koppel 0.2.0 speaks Skakel 2 and Koppel 0.2.1 speaks Skakel 3, so update both machines to the same release.",
                error.failure().message()
            ),
        ),
        // A machine that is a client only answers 404 to everything a pairing client asks.
        FailureKind::UnexpectedAnswer if answered_404(error) => (
            "failed",
            format!("{name} does not take pairing requests. It may be set up as a client only."),
        ),
        _ => (
            "failed",
            format!("Could not pair with {name}: {}", error.failure().message()),
        ),
    };
    status.state = state.to_owned();
    status.message = Some(message);
    status
}

/// What a server that is not taking pairing requests is told to do about it.
fn closed_message(name: &str) -> String {
    format!(
        "{name} is not taking pairing requests. On {name}, open the Clients section and choose Allow pairing, then try again."
    )
}

/// Whether the server answered 404. skakel-client keeps the status only in its error's cause,
/// whose words are its own: "The server answered 404."
fn answered_404(error: &skakel_client::Error) -> bool {
    let mut cause = std::error::Error::source(error);
    while let Some(e) = cause {
        if e.to_string() == "The server answered 404." {
            return true;
        }
        cause = e.source();
    }
    false
}

/// A connection test's outcome, in 0.1.x's fixed words.
fn outcome(outcome: &str, message: Option<String>) -> ConnectionTest {
    let words = match outcome {
        "ok" => "Connected.",
        "incomplete" => "Fill in the server address, fingerprint, client ID and secret first.",
        "unreachable" => {
            "Could not reach the server. Check the address and that the server is running."
        }
        "certificateMismatch" => {
            "The server's certificate does not match the fingerprint. Copy the fingerprint from the server's Clients section again."
        }
        "rejectedCredentials" => {
            "The server refused the client ID or secret. Check them, or ask for a new client."
        }
        "unexpectedAnswer" => {
            "The server did not answer like a Koppel server. Check the address and the port."
        }
        "tooManyRequests" => {
            "The server is turning requests away for now, from this address or, after too many wrong secrets, for this client. Try again in a few minutes."
        }
        _ => "The server accepted the token but no longer knows this client. Ask for a new client.",
    };
    ConnectionTest {
        ok: outcome == "ok",
        outcome: outcome.to_owned(),
        message: message.unwrap_or_else(|| words.to_owned()),
    }
}

/// Skakel's failure as one of the outcomes, as 0.1.x's `ConnectionTestResult.From`. A version
/// mismatch keeps Skakel's wording, which names both sides' versions; a revoked or rotated client
/// reads as one the server no longer knows; anything a test cannot run into counts as unreachable.
fn tested(failure: &skakel_client::Failure) -> ConnectionTest {
    match failure.kind() {
        FailureKind::CertificateMismatch => outcome("certificateMismatch", None),
        FailureKind::RejectedCredentials => outcome("rejectedCredentials", None),
        FailureKind::ClientUnknown | FailureKind::Revoked | FailureKind::Rotated => {
            outcome("clientUnknown", None)
        }
        FailureKind::VersionMismatch => {
            outcome("versionMismatch", Some(failure.message().to_owned()))
        }
        FailureKind::UnexpectedAnswer => outcome("unexpectedAnswer", None),
        FailureKind::TooManyRequests => outcome("tooManyRequests", None),
        _ => outcome("unreachable", None),
    }
}

impl ClientRole for SkakelClientRole {
    fn test(&self) -> Fut<'_, ConnectionTest> {
        Box::pin(async move {
            let saved = self.connection.current();
            if !saved.is_complete() {
                return outcome("incomplete", None);
            }
            let Some(address) = saved.server_url.as_deref().and_then(|u| Url::parse(u).ok()) else {
                return outcome("unreachable", None);
            };
            let Ok(pins) = Pins::with_next(
                saved.server_fingerprint.as_deref().unwrap_or_default(),
                saved.next_server_fingerprint.as_deref(),
            ) else {
                return outcome("certificateMismatch", None);
            };
            let credentials = Credentials::new(
                saved.client_id.clone().unwrap_or_default(),
                saved.client_secret.clone().unwrap_or_default(),
            );
            let options = ClientOptions::default().request_timeout(TEST_TIMEOUT);
            let link = match skakel_client::Link::new(&address, pins, credentials, options) {
                Ok(link) => link,
                Err(e) => return tested(e.failure()),
            };
            match link.hello().await {
                Ok(hello) => outcome(
                    "ok",
                    Some(format!(
                        "Connected to {} as {}.",
                        hello.server, hello.client_id
                    )),
                ),
                Err(e) => tested(e.failure()),
            }
        })
    }

    fn discovered(&self) -> Vec<DiscoveredServer> {
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

    fn pairing(&self) -> PairingStatus {
        self.lock()
            .status
            .clone()
            .unwrap_or_else(PairingStatus::idle)
    }

    fn begin_pairing<'a>(
        &'a self,
        server_url: &'a str,
        fingerprint: &'a str,
        server_name: Option<&'a str>,
    ) -> Fut<'a, ()> {
        Box::pin(async move {
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
            let run = {
                let mut pairings = self.lock();
                pairings.next += 1;
                let run = Arc::new(Run {
                    id: pairings.next,
                    server_url: server_url.to_owned(),
                    fingerprint: crate::api::client::normalize(fingerprint),
                    server_name,
                    stop: CancellationToken::new(),
                    approved: Mutex::new(None),
                    confirming: Mutex::new(false),
                });
                if let Some(previous) = pairings.run.replace(run.clone()) {
                    previous.stop.cancel();
                }
                run
            };
            tokio::spawn(me.run(run));
        })
    }

    fn confirm_pairing(&self) -> Fut<'_, bool> {
        Box::pin(async move {
            let (run, update) = {
                let pairings = self.lock();
                let Some(run) = pairings.run.clone() else {
                    return false;
                };
                if pairings
                    .status
                    .as_ref()
                    .is_none_or(|s| s.state != "confirm")
                {
                    return false;
                }
                let mut confirming = run
                    .confirming
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let approved = run
                    .approved
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                let (Some(update), false) = (approved, *confirming) else {
                    return false;
                };
                *confirming = true;
                drop(confirming);
                (run, update)
            };
            if let Err(e) = self.connection.update(|c| update.apply(c)).await {
                tracing::warn!("Pairing with {} failed: {e}", run.server_name);
                let mut status = Self::status(&run, "failed");
                status.message = Some(format!(
                    "Could not save the connection to {}: {e}",
                    run.server_name
                ));
                self.set(&run, status);
                return true;
            }
            tracing::info!("Paired with {}", run.server_name);
            self.set(&run, Self::status(&run, "paired"));
            true
        })
    }

    fn cancel_pairing(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            {
                let mut pairings = self.lock();
                if let Some(run) = pairings.run.take() {
                    run.stop.cancel();
                }
                pairings.status = None;
            }
            self.push
                .send(push::PAIRING_STATUS_CHANGED, &PairingStatus::idle());
        })
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

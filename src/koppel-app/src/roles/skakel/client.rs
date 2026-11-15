//! The client role on skakel-client: the servers found on the local network, and the connection test,
//! as 0.1.x's `ServerFinder` and `RemoteServer` did them. The servers a client keeps, and adding one
//! by code or key, are in [`crate::servers`]. The link itself is koppel-usb's client over [`super::links`].

use std::sync::{Arc, Weak};
use std::time::Duration;

use koppel_config::ConfigFile;
use koppel_config::files::{ClientConnection, KoppelSettings, Role};
use skakel_client::{Browser, ClientOptions, Credentials, FailureKind, Pins, Url};

use crate::push::{Event, Push};
use crate::roles::{ConnectionTest, DiscoveredServer, Outcome};
use crate::tasks::Tasks;

/// The client role. See the module docs.
pub struct SkakelClientRole {
    push: Push,
    browser: Option<Browser>,
    /// This machine's own server certificate, which discovery leaves out.
    own: Arc<dyn Fn() -> String + Send + Sync>,
    tasks: Tasks,
}

impl SkakelClientRole {
    pub fn new(
        push: Push,
        browser: Option<Browser>,
        own: Arc<dyn Fn() -> String + Send + Sync>,
        tasks: Tasks,
    ) -> Arc<Self> {
        Arc::new(Self {
            push,
            browser,
            own,
            tasks,
        })
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

/// Tests the connection a server's entry holds: the token request and the hello, within `timeout`.
pub async fn test_connection(saved: &ClientConnection, timeout: Duration) -> ConnectionTest {
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
    let options = ClientOptions::default().request_timeout(timeout);
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

impl SkakelClientRole {
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
}

fn digits(fingerprint: &str) -> String {
    fingerprint
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

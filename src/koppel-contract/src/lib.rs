//! Contract tests for Koppel: what a running Koppel answers over HTTPS, checked from the outside.
//!
//! The tests in `tests/` drive a real Koppel the way the browser and other Koppel machines do, so the
//! Rust build can be held to what 0.1.x does. They check status codes, JSON field names and shapes,
//! errors and behaviour, never incidental values such as times and generated ids. This library is the
//! harness they share. It talks HTTP only and never links Koppel's own crates.
//!
//! # Targets
//!
//! Each test needs one or more running Koppels, named by environment variables. A test whose target
//! is not set prints that it was skipped and passes, so `cargo test --workspace` stays green without
//! one. The tests change settings, passwords, clients and certificates: point them only at a Koppel
//! made for the run, never at an install someone uses.
//!
//! | Variable | The target | Started with |
//! |---|---|---|
//! | `KOPPEL_CONTRACT_URL` | The main one, a server. Most tests use it | `RateLimits__AuthPerMinute=1000`, since every test signs in |
//! | `KOPPEL_CONTRACT_CLIENT_URL` | A second one the tests make a client of, paired with the main one | The same, on a network shared with the main one |
//! | `KOPPEL_CONTRACT_PEER_URL` | The main one's address as the client reaches it, such as `https://koppel-contract-main:5240` | |
//! | `KOPPEL_CONTRACT_FRESH_URL` | One without a password, for the first-run setup and then the lockout | Defaults |
//! | `KOPPEL_CONTRACT_EXPIRED_URL` | One whose setup window closes while the tests wait | `Ui__SetupWindowMinutes=1` |
//! | `KOPPEL_CONTRACT_GUARDED_URL`, `KOPPEL_CONTRACT_GUARDED_NETWORK_URL` | One that turns the tests' address away from the UI, with the network surface on a port of its own | `Ui__AllowFrom=local`, `Ports__Network` set |
//! | `KOPPEL_CONTRACT_LIMITS_URL` | One nothing else touches, for the sign-in rate limit | Defaults (30 a minute) |
//! | `KOPPEL_CONTRACT_PASSWORD` | The UI password the tests set and use. Optional | |
//! | `KOPPEL_CONTRACT_PUSH` | How the target pushes to the UI: `signalr` (0.1.x) or `sse` (the Rust build, #84). Optional, `signalr` by default | |
//! | `KOPPEL_CONTRACT_SKAKEL` | `0` when the target has no Skakel: no token endpoint, `/skakel/info`, pairing, discovery or link, as the Rust build before #85. Optional, on by default | |
//! | `KOPPEL_CONTRACT_DISCOVERY` | `0` when the client target cannot hear the main one's discovery answers. Optional, on by default | |
//!
//! `run-against-0.1.2.sh` in this crate starts all of them from the published image in Docker, runs the
//! tests and removes what it started. `run-against-rust.sh` does the same with the Rust build, started
//! from a temporary data folder for each target. The tests take one lock, so they run one at a time
//! whatever the thread count: they share targets and change their settings.
//!
//! # Skakel and discovery
//!
//! 0.1.x speaks Skakel protocol version 1 and the Rust build version 2; the tests take the version
//! a target lists ([`skakel_path`]). With `KOPPEL_CONTRACT_SKAKEL=0`, for a build without Skakel, the
//! parts of a test that need its endpoints or a link say they were skipped ([`needs_skakel`]) and
//! the rest runs. With `KOPPEL_CONTRACT_DISCOVERY=0` the tests that need the client to find the main
//! target by broadcast say they were skipped ([`needs_discovery`]).
//!
//! # Certificates
//!
//! A target's certificate is self-signed. The first one it presents is pinned, and the run trusts that
//! one only; see [`tls`].
//!
//! # The live push
//!
//! 0.1.x pushes over SignalR, the Rust build over Server-Sent Events at `/api/events` (#84). [`push`]
//! lists the events, and `tests/push.rs` checks each against a target with `KOPPEL_CONTRACT_PUSH=sse`,
//! reading the stream with [`Session::events`]. Against 0.1.x those tests say they were skipped.

pub mod push;
pub mod shape;
pub mod tls;

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Method;
use reqwest::header::HeaderMap;
use serde_json::Value;
use tokio::sync::{Mutex, MutexGuard};

pub use serde_json::json;

/// The password the tests set on a target without one, unless `KOPPEL_CONTRACT_PASSWORD` says another.
pub const DEFAULT_PASSWORD: &str = "contract-password-1";

/// The header every change to the UI's API carries, as the UI's `uiFetch` adds it.
pub const UI_HEADER: &str = "X-Koppel-UI";

/// The UI's session cookie.
pub const COOKIE: &str = "koppel.ui";

/// The UI password for this run.
pub fn password() -> String {
    std::env::var("KOPPEL_CONTRACT_PASSWORD").unwrap_or_else(|_| DEFAULT_PASSWORD.to_owned())
}

/// How the target pushes to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// SignalR at `/api/hubs/ui`, as 0.1.x does.
    SignalR,
    /// Server-Sent Events, as the Rust build will (#84).
    Sse,
}

/// What `KOPPEL_CONTRACT_PUSH` says the target pushes with.
pub fn push_kind() -> Push {
    match std::env::var("KOPPEL_CONTRACT_PUSH").as_deref() {
        Ok("sse") => Push::Sse,
        _ => Push::SignalR,
    }
}

/// Whether the target has Skakel's endpoints and link (`KOPPEL_CONTRACT_SKAKEL`, on unless `0`).
pub fn skakel() -> bool {
    std::env::var("KOPPEL_CONTRACT_SKAKEL").as_deref() != Ok("0")
}

/// True when the target has Skakel; otherwise says that `what` was skipped, and why.
pub fn needs_skakel(what: &str) -> bool {
    if skakel() {
        return true;
    }
    eprintln!("skipped: {what} needs Skakel's endpoints and link, which this target does not have");
    false
}

/// True when the client target can hear the main one's discovery answers
/// (`KOPPEL_CONTRACT_DISCOVERY`, on unless `0`); otherwise says that `what` was skipped, and why.
/// Off where the targets cannot reach each other by broadcast, such as processes on Windows that
/// listen on loopback only.
pub fn needs_discovery(what: &str) -> bool {
    if !needs_skakel(what) {
        return false;
    }
    if std::env::var("KOPPEL_CONTRACT_DISCOVERY").as_deref() != Ok("0") {
        return true;
    }
    eprintln!(
        "skipped: {what} needs the targets to hear each other's broadcasts, which this run turned off"
    );
    false
}

/// The path of one of Skakel's HTTPS calls, such as `token`, in the highest protocol version the
/// target lists in `/skakel/info`: `/skakel/v2/token` for the Rust build, `/skakel/v1/token` for
/// 0.1.x.
pub async fn skakel_path(target: &Target, call: &str) -> String {
    let info = target
        .anonymous()
        .get("/skakel/info")
        .await
        .expect(200)
        .json();
    let version = info["versions"]
        .as_array()
        .and_then(|versions| versions.iter().filter_map(Value::as_u64).max())
        .expect("the target lists the Skakel versions it speaks");
    format!("/skakel/v{version}/{call}")
}

/// The path of the live push on a target that pushes with Server-Sent Events.
pub const EVENTS_PATH: &str = "/api/events";

/// A running Koppel at a base address such as `https://localhost:15240`.
#[derive(Clone)]
pub struct Target {
    base: String,
    http: reqwest::Client,
    pins: Arc<tls::Pins>,
}

impl Target {
    /// The target at `base`. Its pins are shared by every test in the process.
    pub fn new(base: &str) -> Target {
        let base = base.trim_end_matches('/').to_owned();
        let pins = tls::Pins::of(authority(&base));
        Target {
            http: tls::client(pins.clone()),
            pins,
            base,
        }
    }

    /// The target named by `variable`, or `None` after saying the test was skipped.
    pub fn from_env(variable: &str) -> Option<Target> {
        match std::env::var(variable) {
            Ok(base) if !base.trim().is_empty() => Some(Target::new(base.trim())),
            _ => {
                skipped(variable);
                None
            }
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// `host:port` of the base address.
    pub fn authority(&self) -> &str {
        authority(&self.base)
    }

    /// No session: what anyone on the network gets.
    pub fn anonymous(&self) -> Session {
        Session {
            target: self.clone(),
            cookie: None,
        }
    }

    /// A session that sends this `koppel.ui` cookie value.
    pub fn with_cookie(&self, cookie: &str) -> Session {
        Session {
            target: self.clone(),
            cookie: Some(cookie.to_owned()),
        }
    }

    /// Trusts another certificate of this target, such as the next one before a switch.
    pub fn trust(&self, fingerprint: &str) {
        self.pins.trust(fingerprint);
    }

    /// The certificate this target presented in its last handshake.
    pub fn presented(&self) -> Option<String> {
        self.pins.last()
    }

    /// The fingerprint of the certificate the target presents now, on a new connection. It must be
    /// one the run trusts.
    pub async fn current_certificate(&self) -> String {
        let fresh = Target::new(&self.base);
        fresh.anonymous().get("/healthz").await;
        fresh
            .presented()
            .expect("the handshake presented a certificate")
    }

    /// Sets the run's password when the target has none, and says so when it has another.
    pub async fn ensure_password(&self) {
        let state = self
            .anonymous()
            .get("/api/auth/state")
            .await
            .expect(200)
            .json();
        if state["passwordSet"] == Value::Bool(false) {
            self.anonymous()
                .post("/api/auth/setup", json!({ "password": password() }))
                .await
                .expect(204);
        }
    }

    /// Signs in with the run's password, setting it first when the target has none.
    pub async fn sign_in(&self) -> Session {
        self.ensure_password().await;
        let answer = self
            .anonymous()
            .post("/api/auth/login", json!({ "password": password() }))
            .await;
        assert_eq!(
            answer.status,
            204,
            "signing in to {} failed: {} {}. A target with another password needs KOPPEL_CONTRACT_PASSWORD.",
            self.base,
            answer.status,
            answer.text()
        );
        let cookie = answer
            .session_cookie()
            .expect("a sign-in sets the session cookie");
        self.with_cookie(&cookie)
    }
}

fn authority(base: &str) -> &str {
    let rest = base.split_once("://").map_or(base, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// Says why a test did nothing. The test then passes.
pub fn skipped(variable: &str) {
    eprintln!(
        "skipped: {variable} is not set, so this test has no Koppel to run against. See the koppel-contract docs."
    );
}

/// A request body.
pub enum Body {
    None,
    Json(Value),
    /// Raw bytes with a content type, for bodies that are not valid JSON on purpose.
    Raw(&'static str, String),
}

/// One caller of a target, with or without a session cookie.
#[derive(Clone)]
pub struct Session {
    target: Target,
    cookie: Option<String>,
}

impl Session {
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The `koppel.ui` cookie value this session sends, if any.
    pub fn cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    /// Sends a request exactly as given: no `X-Koppel-UI` header unless `headers` has it.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Body,
        headers: &[(&str, &str)],
    ) -> Answer {
        let what = format!("{method} {path}");
        let mut request = self
            .target
            .http
            .request(method, format!("{}{path}", self.target.base));
        if let Some(cookie) = &self.cookie {
            request = request.header("Cookie", format!("{COOKIE}={cookie}"));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request = match body {
            Body::None => request,
            Body::Json(value) => request.json(&value),
            Body::Raw(content_type, text) => {
                request.header("Content-Type", content_type).body(text)
            }
        };
        let response = request
            .send()
            .await
            .unwrap_or_else(|e| panic!("{what} to {} failed: {e:?}", self.target.base));
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .unwrap_or_else(|e| panic!("reading the answer to {what} failed: {e:?}"))
            .to_vec();
        Answer {
            what,
            status,
            headers,
            body,
        }
    }

    /// A read, as the UI makes it.
    pub async fn get(&self, path: &str) -> Answer {
        self.send(Method::GET, path, Body::None, &[]).await
    }

    /// A change with a JSON body, with the header the UI adds.
    pub async fn post(&self, path: &str, body: Value) -> Answer {
        self.change(Method::POST, path, Body::Json(body)).await
    }

    /// A change without a body, with the header the UI adds.
    pub async fn post_empty(&self, path: &str) -> Answer {
        self.change(Method::POST, path, Body::None).await
    }

    pub async fn put(&self, path: &str, body: Value) -> Answer {
        self.change(Method::PUT, path, Body::Json(body)).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Answer {
        self.change(Method::PATCH, path, Body::Json(body)).await
    }

    pub async fn delete(&self, path: &str) -> Answer {
        self.change(Method::DELETE, path, Body::None).await
    }

    /// Any change, with the header the UI adds.
    pub async fn change(&self, method: Method, path: &str, body: Body) -> Answer {
        self.send(method, path, body, &[(UI_HEADER, "1")]).await
    }

    /// The live push, opened with this session's cookie, as the UI's `EventSource` opens it.
    pub async fn events(&self) -> Events {
        let response = self
            .get_stream(EVENTS_PATH, &[("Accept", "text/event-stream")])
            .await;
        Events {
            status: response.status().as_u16(),
            content_type: response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
            response,
            buffer: String::new(),
        }
    }

    /// Opens a streaming GET and hands back the response unread, for a push test that reads events as
    /// they arrive.
    pub async fn get_stream(&self, path: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut request = self.target.http.get(format!("{}{path}", self.target.base));
        if let Some(cookie) = &self.cookie {
            request = request.header("Cookie", format!("{COOKIE}={cookie}"));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path} to {} failed: {e:?}", self.target.base))
    }
}

/// What reading the live push came to.
#[derive(Debug)]
pub enum Next {
    /// An event: its name and its data as JSON.
    Event(String, Value),
    /// The server ended the stream.
    Ended,
    /// Nothing came in time.
    TimedOut,
}

/// The live push as it comes, one Server-Sent Event at a time.
pub struct Events {
    pub status: u16,
    pub content_type: String,
    response: reqwest::Response,
    buffer: String,
}

impl Events {
    /// The next event, the end of the stream, or nothing within `timeout`. Comments, the keep-alive
    /// among them, are skipped.
    pub async fn next(&mut self, timeout: Duration) -> Next {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                let mut name = String::from("message");
                let mut data = Vec::new();
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("event:") {
                        name = value.trim_start().to_owned();
                    } else if let Some(value) = line.strip_prefix("data:") {
                        data.push(value.strip_prefix(' ').unwrap_or(value).to_owned());
                    }
                }
                if data.is_empty() {
                    continue;
                }
                let text = data.join("\n");
                let value = serde_json::from_str(&text)
                    .unwrap_or_else(|e| panic!("the event {name} carries no JSON ({e}): {text}"));
                return Next::Event(name, value);
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Next::TimedOut;
            };
            match tokio::time::timeout(left, self.response.chunk()).await {
                Err(_) => return Next::TimedOut,
                Ok(Ok(Some(chunk))) => self
                    .buffer
                    .push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n")),
                Ok(Ok(None)) | Ok(Err(_)) => return Next::Ended,
            }
        }
    }

    /// The first event named `name` whose data `accept` takes, skipping others. Fails the test when
    /// the stream ends or nothing comes within `timeout`.
    pub async fn wait_for(
        &mut self,
        name: &str,
        timeout: Duration,
        mut accept: impl FnMut(&Value) -> bool,
    ) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next(left).await {
                Next::Event(got, value) if got == name && accept(&value) => return value,
                Next::Event(..) => {}
                Next::Ended => panic!("the stream ended while waiting for {name}"),
                Next::TimedOut => panic!("no {name} came within {timeout:?}"),
            }
        }
    }

    /// True when the server ends the stream within `timeout`, whatever events come before.
    pub async fn ends_within(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next(left).await {
                Next::Event(..) => {}
                Next::Ended => return true,
                Next::TimedOut => return false,
            }
        }
    }
}

/// A response, read whole.
pub struct Answer {
    what: String,
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Answer {
    /// Fails the test unless the status is `status`, showing the body.
    #[track_caller]
    pub fn expect(self, status: u16) -> Answer {
        assert_eq!(
            self.status,
            status,
            "{} answered {}, not {status}: {}",
            self.what,
            self.status,
            self.text()
        );
        self
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON.
    #[track_caller]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "{} did not answer JSON ({e}): {} {}",
                self.what,
                self.status,
                self.text()
            )
        })
    }

    /// One header's value.
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap_or_default().to_owned())
    }

    /// The media type without parameters, lower case.
    pub fn content_type(&self) -> String {
        self.header("content-type")
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    }

    /// Every `Set-Cookie` header.
    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// The `Set-Cookie` header for the session cookie, if one was set.
    pub fn session_set_cookie(&self) -> Option<String> {
        self.set_cookies()
            .into_iter()
            .find(|c| c.starts_with(&format!("{COOKIE}=")))
    }

    /// The new session cookie's value, if this answer set a non-empty one.
    pub fn session_cookie(&self) -> Option<String> {
        let header = self.session_set_cookie()?;
        let value = header
            .split(';')
            .next()?
            .trim()
            .strip_prefix(&format!("{COOKIE}="))?;
        (!value.is_empty()).then(|| value.to_owned())
    }

    /// `{ error }`, the shape of the sign-in endpoints' refusals.
    #[track_caller]
    pub fn error(&self) -> String {
        let body = self.json();
        shape::fields(&body, &[("error", shape::Kind::Str)], &self.what);
        body["error"].as_str().unwrap_or_default().to_owned()
    }

    /// A validation problem (400, `application/problem+json`) whose `errors` name exactly `fields`.
    #[track_caller]
    pub fn problem(&self, fields: &[&str]) {
        assert_eq!(self.status, 400, "{}: {}", self.what, self.text());
        assert_eq!(
            self.content_type(),
            "application/problem+json",
            "{} answers a validation problem",
            self.what
        );
        let body = self.json();
        assert_eq!(body["status"], json!(400), "{}: {body}", self.what);
        assert!(body["title"].is_string(), "{}: {body}", self.what);
        let errors = body["errors"]
            .as_object()
            .unwrap_or_else(|| panic!("{} has no errors object: {body}", self.what));
        let mut names: Vec<&str> = errors.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut want = fields.to_vec();
        want.sort_unstable();
        assert_eq!(names, want, "{}: {body}", self.what);
        for (name, messages) in errors {
            let messages = messages
                .as_array()
                .unwrap_or_else(|| panic!("{}: errors.{name} is not a list", self.what));
            assert!(
                !messages.is_empty() && messages.iter().all(Value::is_string),
                "{}: errors.{name} holds messages",
                self.what
            );
        }
    }

    /// An empty body, as a 202 or 204 has.
    #[track_caller]
    pub fn empty(&self) {
        assert!(
            self.body.is_empty(),
            "{} answered with a body: {}",
            self.what,
            self.text()
        );
    }
}

/// The one lock every test holds, because the tests share targets and change their state.
pub async fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::const_new(());
    LOCK.lock().await
}

/// The main target, signed in and back at the settings the tests start from: a server, no certificate
/// prepared. Holds the lock while it lives.
pub struct Main {
    pub target: Target,
    pub admin: Session,
    _lock: MutexGuard<'static, ()>,
}

impl Main {
    /// The main target, or `None` (skipped) when `KOPPEL_CONTRACT_URL` is not set.
    pub async fn get() -> Option<Main> {
        let lock = lock().await;
        let target = Target::from_env("KOPPEL_CONTRACT_URL")?;
        let admin = target.sign_in().await;
        baseline_server(&admin).await;
        Some(Main {
            target,
            admin,
            _lock: lock,
        })
    }
}

/// The settings a server starts each test with, whatever the last test left behind.
async fn baseline_server(admin: &Session) {
    admin
        .patch(
            "/api/settings",
            json!({ "role": "server", "shareOnStartup": true, "notifyPairing": true, "notifyLink": true, "notifyAttachFailures": true }),
        )
        .await
        .expect(202);
    let keys = admin.get("/api/server/keys").await.expect(200).json();
    if !keys["nextFingerprint"].is_null() {
        admin
            .delete("/api/server/keys/next-certificate")
            .await
            .expect(202);
    }
}

/// The main target as a server and the client target as its client, both signed in. Holds the lock.
pub struct Pair {
    pub server: Target,
    pub server_admin: Session,
    pub client: Target,
    pub client_admin: Session,
    /// The server's address as the client reaches it.
    pub peer_url: String,
    _lock: MutexGuard<'static, ()>,
}

impl Pair {
    /// Both targets, or `None` (skipped) when any of their variables is not set.
    pub async fn get() -> Option<Pair> {
        let lock = lock().await;
        let server = Target::from_env("KOPPEL_CONTRACT_URL")?;
        let client = Target::from_env("KOPPEL_CONTRACT_CLIENT_URL")?;
        let Ok(peer_url) = std::env::var("KOPPEL_CONTRACT_PEER_URL") else {
            skipped("KOPPEL_CONTRACT_PEER_URL");
            return None;
        };
        let server_admin = server.sign_in().await;
        baseline_server(&server_admin).await;
        let client_admin = client.sign_in().await;
        client_admin
            .patch("/api/settings", json!({ "role": "client" }))
            .await
            .expect(202);
        client_admin.delete("/api/client/pairing").await.expect(202);
        Some(Pair {
            server,
            server_admin,
            client,
            client_admin,
            peer_url: peer_url.trim_end_matches('/').to_owned(),
            _lock: lock,
        })
    }
}

/// A target only one test uses, with the lock held.
pub struct Single {
    pub target: Target,
    _lock: MutexGuard<'static, ()>,
}

impl Single {
    /// The target named by `variable`, or `None` (skipped).
    pub async fn get(variable: &str) -> Option<Single> {
        let lock = lock().await;
        let target = Target::from_env(variable)?;
        Some(Single {
            target,
            _lock: lock,
        })
    }
}

/// Asks `check` every quarter second until it gives a value, and fails the test after `timeout`.
pub async fn eventually<T, F, Fut>(what: &str, timeout: Duration, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let start = Instant::now();
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(
            start.elapsed() < timeout,
            "gave up after {timeout:?} waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The activity entries with this reason, newest first.
pub async fn activity_with(admin: &Session, reason: &str) -> Vec<Value> {
    admin
        .get("/api/activity?max=500")
        .await
        .expect(200)
        .json()
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e["reason"] == reason)
        .collect()
}

/// A name no other run uses, for clients the tests add.
pub fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos:x}")
}

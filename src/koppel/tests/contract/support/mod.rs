//! Contract tests for Koppel: what a running Koppel answers over HTTPS, checked from the outside.
//!
//! The tests in `tests/contract/` start the real `koppel` program (`CARGO_BIN_EXE_koppel`) the way a
//! person or another Koppel machine meets it, and drive it over HTTPS. They check status codes, JSON
//! field names and types, errors and behaviour, never incidental values such as times and generated
//! ids. This module is the harness they share. It talks HTTP only and never links Koppel's own crates,
//! so a test sees what a browser or another machine sees.
//!
//! # Instances
//!
//! Each test starts the instances it needs ([`Main`], [`Pair`], [`Single`] or [`Instance`] itself) on
//! a free port, with a temporary config and keys folder, and a guard that stops the program and removes
//! the folders when the test ends, whether it passed or not. Nothing is shared between tests, so they
//! run in parallel like any other, and `cargo test --workspace` runs them for real. Because they start
//! the program, they need no environment variables and no Docker.
//!
//! # Skakel and discovery
//!
//! The program speaks Skakel protocol version 3; the tests take the version it lists
//! ([`skakel_path`]). Discovery works by broadcast, which a container or a Linux machine allows and
//! the Windows runs here turn off (see [`Options`]), so the two tests that need it only build on Linux.
//!
//! # Certificates
//!
//! An instance's certificate is self-signed. The first one it presents is pinned, and the test trusts
//! that one only; see [`tls`].
//!
//! # The live push
//!
//! The program pushes to the UI over Server-Sent Events at `/api/v1/events`. [`push`] lists the events,
//! and `push.rs` checks each, reading the stream with [`Session::events`].

mod process;
pub mod push;
pub mod shape;
pub mod tls;

#[cfg(target_os = "linux")]
pub use process::own_address;
pub use process::{Instance, Options};

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Method;
use reqwest::header::HeaderMap;
use serde_json::Value;

pub use serde_json::json;

/// The password the tests set on an instance without one.
pub const DEFAULT_PASSWORD: &str = "contract-password-1";

/// The header every change to the UI's API carries, as the UI's `uiFetch` adds it.
pub const UI_HEADER: &str = "X-Koppel-UI";

/// The UI's session cookie.
pub const COOKIE: &str = "__Host-koppel-session";

/// The UI password the tests set and use.
pub fn password() -> String {
    DEFAULT_PASSWORD.to_owned()
}

/// The path of one of Skakel's HTTPS calls, such as `token`, in the highest protocol version the
/// target lists in `/skakel/info`, such as `/skakel/v3/token`.
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
pub const EVENTS_PATH: &str = "/api/v1/events";

/// A running Koppel at a base address such as `https://localhost:15240`.
#[derive(Clone)]
pub struct Target {
    base: String,
    http: reqwest::Client,
    pins: Arc<tls::Pins>,
}

impl Target {
    /// The target at `base`, with pins of its own: the first certificate it presents is the one
    /// this target and its clones trust.
    pub fn new(base: &str) -> Target {
        let base = base.trim_end_matches('/').to_owned();
        let pins = Arc::new(tls::Pins::default());
        Target {
            http: tls::client(pins.clone()),
            pins,
            base,
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

    /// A session that sends this session cookie value.
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
        let fresh = Target {
            http: tls::client(self.pins.clone()),
            pins: self.pins.clone(),
            base: self.base.clone(),
        };
        fresh.anonymous().get("/healthz").await;
        fresh
            .presented()
            .expect("the handshake presented a certificate")
    }

    /// Sets the run's password when the target has none, and says so when it has another.
    pub async fn ensure_password(&self) {
        let state = self
            .anonymous()
            .get("/api/v1/auth/state")
            .await
            .expect(200)
            .json();
        if state["passwordSet"] == Value::Bool(false) {
            self.anonymous()
                .post("/api/v1/auth/setup", json!({ "password": password() }))
                .await
                .expect(204);
        }
    }

    /// Signs in with the run's password, setting it first when the target has none.
    pub async fn sign_in(&self) -> Session {
        self.ensure_password().await;
        let answer = self
            .anonymous()
            .post("/api/v1/auth/login", json!({ "password": password() }))
            .await;
        assert_eq!(
            answer.status,
            204,
            "signing in to {} failed: {} {}",
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
    /// The session cookie value this session sends, if any.
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

    /// A change without a body, with the header the UI adds.
    pub async fn put_empty(&self, path: &str) -> Answer {
        self.change(Method::PUT, path, Body::None).await
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

    /// A refusal as problem JSON (RFC 9457): the media type, `status` matching the answer's, and a
    /// `code` and a `detail` that are not empty. Returns the sentence to show: the first message of
    /// `errors` when there is one, else the `detail`.
    #[track_caller]
    pub fn error(&self) -> String {
        assert_eq!(
            self.content_type(),
            "application/problem+json",
            "{} answers a problem: {}",
            self.what,
            self.text()
        );
        let body = self.json();
        shape::fields(
            &body,
            &[
                ("type", shape::Kind::Str),
                ("title", shape::Kind::Str),
                ("status", shape::Kind::Int),
                ("code", shape::Kind::Str),
                ("detail", shape::Kind::Str),
            ],
            &self.what,
        );
        assert_eq!(body["status"], json!(self.status), "{}: {body}", self.what);
        assert!(
            !body["code"].as_str().unwrap_or_default().is_empty(),
            "{body}"
        );
        let first = body["errors"]
            .as_object()
            .and_then(|errors| errors.values().next())
            .and_then(|messages| messages.get(0))
            .and_then(Value::as_str);
        first
            .or_else(|| body["detail"].as_str())
            .unwrap_or_default()
            .to_owned()
    }

    /// The `code` of a problem answer, after `error()` has checked its shape.
    #[track_caller]
    pub fn code(&self) -> String {
        self.error();
        self.json()["code"].as_str().unwrap_or_default().to_owned()
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
        assert_eq!(body["code"], json!("validation"), "{}: {body}", self.what);
        assert!(body["title"].is_string(), "{}: {body}", self.what);
        assert!(body["detail"].is_string(), "{}: {body}", self.what);
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

/// A server started for one test, signed in and at the settings the tests start from. Stops with the
/// test.
pub struct Main {
    pub target: Target,
    pub admin: Session,
    pub instance: Instance,
}

impl Main {
    /// Starts the program as a server and signs in.
    pub async fn get() -> Main {
        Main::with(Options::default()).await
    }

    /// The same, started with these options.
    pub async fn with(options: Options) -> Main {
        let instance = Instance::start(options).await;
        let target = instance.target.clone();
        let admin = target.sign_in().await;
        baseline_server(&admin).await;
        Main {
            target,
            admin,
            instance,
        }
    }
}

/// The settings a server starts each test with.
async fn baseline_server(admin: &Session) {
    admin
        .patch(
            "/api/v1/settings",
            json!({ "role": "server", "shareOnStartup": true, "notifyPairing": true, "notifyLink": true, "notifyAttachFailures": true }),
        )
        .await
        .expect(202);
}

/// A server and a second instance as its client, both signed in, each started for one test.
pub struct Pair {
    pub server_admin: Session,
    pub client: Target,
    pub client_admin: Session,
    /// The server's address as the client reaches it.
    pub peer_url: String,
    pub _server_instance: Instance,
    pub client_instance: Instance,
}

impl Pair {
    /// Starts both. The client looks for servers on the server's port, so it finds the server by
    /// broadcast where broadcast works.
    pub async fn get() -> Pair {
        Pair::with(Options::default()).await
    }

    /// The same, with the server started with these options.
    pub async fn with(server: Options) -> Pair {
        let server_instance = Instance::start(server).await;
        let server = server_instance.target.clone();
        let client_instance = Instance::start(
            Options::default().setting("KOPPEL_DISCOVERY_PORT", server_instance.port().to_string()),
        )
        .await;
        let client = client_instance.target.clone();
        let server_admin = server.sign_in().await;
        baseline_server(&server_admin).await;
        let client_admin = client.sign_in().await;
        client_admin
            .patch("/api/v1/settings", json!({ "role": "client" }))
            .await
            .expect(202);
        let peer_url = server.base().to_owned();
        Pair {
            server_admin,
            client,
            client_admin,
            peer_url,
            _server_instance: server_instance,
            client_instance,
        }
    }
}

impl Pair {
    /// Registers a client on the server, saves its credentials on the client and waits until the
    /// client's link is up. Returns the new client's id and secret.
    pub async fn connect(&self) -> (String, String) {
        let created = self
            .server_admin
            .post(
                "/api/v1/server/clients",
                json!({ "name": unique("contract-client") }),
            )
            .await
            .expect(201)
            .json();
        let fingerprint = self
            .server_admin
            .get("/api/v1/server/identity")
            .await
            .expect(200)
            .json()["fingerprint"]
            .clone();
        self.client_admin
            .put(
                "/api/v1/client/connection",
                json!({
                    "serverUrl": self.peer_url,
                    "serverFingerprint": fingerprint,
                    "clientId": created["clientId"],
                    "clientSecret": created["clientSecret"],
                }),
            )
            .await
            .expect(202);
        eventually("the client to link", Duration::from_secs(60), || async {
            let devices = self
                .client_admin
                .get("/api/v1/client/devices")
                .await
                .expect(200)
                .json();
            (devices["link"] == json!("connected")).then_some(())
        })
        .await;
        (
            created["clientId"].as_str().unwrap_or_default().to_owned(),
            created["clientSecret"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    }
}

/// An instance only one test uses, started with its own settings.
pub struct Single {
    pub target: Target,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub instance: Instance,
}

impl Single {
    pub async fn start(options: Options) -> Single {
        let instance = Instance::start(options).await;
        Single {
            target: instance.target.clone(),
            instance,
        }
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
        .get("/api/v1/activity?max=500")
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

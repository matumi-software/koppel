//! The guards in front of every route, in 0.1.x's order:
//!
//! 1. Security headers on every answer, whatever answers.
//! 2. Port and role: a path whose surface is not on the port the connection came in on, or that the
//!    current role does not have, gets 404. The port is the local one, never the `Host` header.
//! 3. `Ui:AllowFrom`: a UI path from an address it does not take gets 403 and a line saying so.
//! 4. Origin and header: a UI API call from another origin gets 403, and so does a change (anything
//!    but GET or HEAD) without `X-Koppel-UI: 1`.
//! 5. The rate limit on setup, login and password change, per address, after the port and role
//!    check so a request for the wrong port does not use up the allowance.
//! 6. The session: every UI API path but the anonymous ones needs a good cookie, else 401.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST, HeaderName, HeaderValue, ORIGIN, REFERRER_POLICY,
    SET_COOKIE, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::access::{UiReach, address_key};
use crate::app::App;
use crate::auth::{Rejected, ticket};
use crate::surface::{Listener, Surface, requires_sign_in, starts_with_segments};

/// The header every change to the UI's API carries.
pub(crate) const UI_HEADER: &str = "x-koppel-ui";
const FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
/// The paths the sign-in rate limit counts. State and logout are left out: the UI polls the state.
const AUTH_PATHS: [&str; 3] = ["/api/auth/setup", "/api/auth/login", "/api/auth/password"];

/// The local address a connection came in on, which says which port, so which surface, it is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalAddr(pub SocketAddr);

pub(crate) async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let mut response = check(&app, request, next).await;
    let headers = response.headers_mut();
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    // Only frame-ancestors, so it cannot break the app or the Vite dev proxy.
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

async fn check(app: &App, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let remote = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0)
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 0)));
    let local_port = request
        .extensions()
        .get::<LocalAddr>()
        .map_or(app.host.ui_port, |local| local.0.port());

    // Skakel's limits on its token and pairing calls come first, as its middleware came ahead of
    // everything Koppel added in 0.1.x, so a call to the wrong port or role counts too.
    if !app.skakel_limits.allow(&path, remote.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("application/problem+json"),
            )],
            r#"{"type":"https://tools.ietf.org/html/rfc6585#section-4","title":"Too Many Requests","status":429,"detail":"Too many requests from this address. Wait a minute and try again."}"#,
        )
            .into_response();
    }

    // A gRPC call is Skakel's, or Koppel's own service beside it: other machines, a server's.
    let surface = if crate::roles::skakel::front::is_grpc(&request) {
        Surface::GRPC
    } else {
        Surface::of(&path)
    };
    let port = match surface.listener {
        Listener::Ui => app.host.ui_port,
        Listener::Network => app.host.network_port,
    };
    if local_port != port || !surface.served_to(app.settings.current().role) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if surface.listener == Listener::Ui && !app.access.allows(remote.ip()) {
        return (
            StatusCode::FORBIDDEN,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            )],
            format!(
                "Koppel's UI does not answer this address. {} decides who may use it.",
                UiReach::SETTING
            ),
        )
            .into_response();
    }
    if surface.listener == Listener::Ui
        && starts_with_segments(&path, "/api")
        && !from_the_ui(&request)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if AUTH_PATHS
        .iter()
        .any(|prefix| starts_with_segments(&path, prefix))
        && !app.auth_limit.allow(&address_key(remote.ip()))
    {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    // Anonymous routes see the session too, when there is one: the state says whether it is good.
    let mut clear = false;
    let mut renewed = None;
    if surface.listener == Listener::Ui
        && let Some(value) = ticket::from_request(request.headers())
    {
        match app.auth.check(&value) {
            Ok(session) => {
                renewed = session.renewed.clone();
                request.extensions_mut().insert(session);
            }
            Err(Rejected::Ended) => clear = true,
            Err(Rejected::NotOurs) => {}
        }
    }
    let signed_in = request
        .extensions()
        .get::<crate::auth::SignedIn>()
        .is_some();
    let mut response = if requires_sign_in(&path) && !signed_in {
        StatusCode::UNAUTHORIZED.into_response()
    } else {
        next.run(request).await
    };
    // A session that ended loses its cookie, and one past its half gets a later expiry, unless the
    // route set a cookie of its own (a sign-in, a sign-out, a password change).
    let cookie = if clear {
        Some(ticket::clear_cookie())
    } else {
        renewed
    };
    if let Some(cookie) = cookie
        && !response.headers().contains_key(SET_COOKIE)
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    response
}

/// The Origin, when there is one, is this request's own, and a change carries `X-Koppel-UI: 1`, a
/// header a page on another site cannot add without a CORS preflight this server never grants.
fn from_the_ui(request: &Request) -> bool {
    let headers = request.headers();
    if let Some(origin) = headers.get(ORIGIN)
        && !same_origin(request, origin.to_str().unwrap_or_default())
    {
        return false;
    }
    let change = request.method() != Method::GET && request.method() != Method::HEAD;
    !change || headers.get(UI_HEADER).is_some_and(|v| v.as_bytes() == b"1")
}

/// The origin's scheme, host and port against this request's own. Behind a TLS proxy, such as the
/// Vite dev server, the scheme comes from `X-Forwarded-Proto`; a browser cannot set that on a
/// cross-site request without a preflight, so it is no way around the check.
fn same_origin(request: &Request, origin: &str) -> bool {
    let Some((origin_scheme, origin_host, origin_port)) = split_origin(origin) else {
        return false;
    };
    let scheme = request
        .headers()
        .get(FORWARDED_PROTO)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("https")
        .to_ascii_lowercase();
    let authority = request
        .uri()
        .authority()
        .map(|a| a.as_str().to_owned())
        .or_else(|| {
            request
                .headers()
                .get(HOST)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let Some((host, port)) = split_host(&authority) else {
        return false;
    };
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    origin_scheme == scheme && origin_host.eq_ignore_ascii_case(&host) && origin_port == port
}

/// `scheme://host[:port]` as its parts, the port filled in from the scheme. `None` for anything
/// else, such as `null`.
fn split_origin(origin: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = origin.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains(['/', '?', '#', '@']) {
        return None;
    }
    let (host, port) = split_host(rest)?;
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some((scheme, host, port))
}

/// `host[:port]`, with an IPv6 host in brackets.
fn split_host(authority: &str) -> Option<(String, Option<u16>)> {
    if authority.is_empty() {
        return None;
    }
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host = &authority[..=end];
        let port = match &authority[end + 1..] {
            "" => None,
            rest => Some(rest.strip_prefix(':')?.parse().ok()?),
        };
        return Some((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((host.to_owned(), Some(port.parse().ok()?))),
        Some(_) => None,
        None => Some((authority.to_owned(), None)),
    }
}

/// A fixed one-minute window per caller, as ASP.NET's fixed window limiter: so many requests, then
/// 429 until the window is over.
pub(crate) struct RateLimit {
    permits: u32,
    window: Duration,
    callers: Mutex<HashMap<String, (Instant, u32)>>,
}

/// Callers the limit keeps track of before it forgets the ones whose window is over.
const MAX_CALLERS: usize = 4096;

impl RateLimit {
    pub(crate) fn per_minute(permits: u32) -> Self {
        Self {
            permits,
            window: Duration::from_secs(60),
            callers: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn allow(&self, caller: &str) -> bool {
        let now = Instant::now();
        let mut callers = self.callers.lock().unwrap_or_else(PoisonError::into_inner);
        if callers.len() >= MAX_CALLERS {
            let window = self.window;
            callers.retain(|_, (start, _)| now.duration_since(*start) < window);
        }
        let entry = callers.entry(caller.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        if entry.1 >= self.permits {
            return false;
        }
        entry.1 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_split_into_their_parts() {
        assert_eq!(
            split_origin("https://localhost:15240"),
            Some(("https".into(), "localhost".into(), 15240))
        );
        assert_eq!(
            split_origin("https://elsewhere.example"),
            Some(("https".into(), "elsewhere.example".into(), 443))
        );
        assert_eq!(
            split_origin("http://[::1]:80/"),
            Some(("http".into(), "[::1]".into(), 80))
        );
        assert_eq!(split_origin("null"), None);
        assert_eq!(split_origin("https://a/b"), None);
    }

    #[test]
    fn the_limit_counts_per_caller_in_a_window() {
        let limit = RateLimit::per_minute(2);
        assert!(limit.allow("a") && limit.allow("a"));
        assert!(!limit.allow("a"));
        assert!(limit.allow("b"));
    }
}

//! The guards, as layers on the routers that need them (see `routes.rs` for where each sits). A new
//! route is safe by construction: it joins a router and gets that router's layers.
//!
//! | Layer | On | Answers |
//! |---|---|---|
//! | [`security_headers`] | every router, outermost | adds the headers to every answer, whatever answers |
//! | [`connection_info`] | every router | 500 when the request has no caller address: a request must come through the accept loop (or a probe that adds one) |
//! | [`network_paths_are_not_here`] | the UI router | 404 for a path the network surface serves, before anything else looks at it |
//! | [`allow_from`] | the UI router | 403 from an address `Ui:AllowFrom` does not take |
//! | [`ui_origin`] | the UI's `/api` | 403 for another origin, and for a change without `X-Koppel-UI: 1` |
//! | [`auth_limit`] | the sign-in routes | 429 over the per-address limit |
//! | [`session`] | the UI's `/api` | puts the session on the request when the cookie is good, and renews or clears the cookie |
//! | [`role_gate`] | `/api/server`, `/api/client` and the network router | 404 when this machine's role does not have it |
//! | [`require_session`] | the protected routes | 401 without a session |
//!
//! Skakel's limits on its token and pairing calls are Skakel's own, behind these guards, per
//! caller's address (see `roles::skakel`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST, HeaderName, HeaderValue, ORIGIN, REFERRER_POLICY,
    SET_COOKIE, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use koppel_config::ConfigFile;
use koppel_config::files::KoppelSettings;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::access::{UiAccess, UiReach, address_key};
use crate::auth::{Rejected, SignedIn, UiAuth, ticket};
use crate::surface::{RoleScope, is_network_path};

/// The header every change to the UI's API carries.
pub(crate) const UI_HEADER: &str = "x-koppel-ui";
const FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// The four security headers on every answer. Outermost, so they go on whatever answers: a route, a
/// guard's refusal, a panic's 500.
pub(crate) fn security_headers<S: Clone + Send + Sync + 'static>(router: Router<S>) -> Router<S> {
    router
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        // Only frame-ancestors, so it cannot break the app or the Vite dev proxy.
        .layer(SetResponseHeaderLayer::overriding(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'none'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
}

/// Refuses a request that carries no `ConnectInfo`. Every guard that looks at the caller reads it,
/// and a guess for a missing one (loopback, say) would open the UI to whoever lacked it. The accept
/// loop adds it to each request, and so does a probe that calls the router in process.
pub(crate) async fn connection_info(request: Request, next: Next) -> Response {
    if request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none()
    {
        tracing::error!(
            "A request for {} reached the router without the caller's address and was refused",
            request.uri().path()
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    next.run(request).await
}

fn caller(request: &Request) -> Option<SocketAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0)
}

/// On the UI router: a path the network surface serves is not here. This answers before
/// `Ui:AllowFrom`, so a caller the UI turns away learns nothing of the other surface from it.
pub(crate) async fn network_paths_are_not_here(request: Request, next: Next) -> Response {
    if is_network_path(request.uri().path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

/// `Ui:AllowFrom`: the UI from an address it does not take gets 403 and a line saying so.
pub(crate) async fn allow_from(
    State(access): State<Arc<UiAccess>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(remote) = caller(&request) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if !access.allows(remote.ip()) {
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
    next.run(request).await
}

/// On the UI's `/api`: the Origin, when there is one, is this request's own, and a change carries
/// `X-Koppel-UI: 1`, a header a page on another site cannot add without a CORS preflight this server
/// never grants.
pub(crate) async fn ui_origin(request: Request, next: Next) -> Response {
    if !from_the_ui(&request) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

/// The sign-in rate limit on setup, login and password change, per address. It sits on those routes
/// alone: state and logout are left out, since the UI polls the state.
pub(crate) async fn auth_limit(
    State(limit): State<Arc<RateLimit>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(remote) = caller(&request) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if !limit.allow(&address_key(remote.ip())) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    next.run(request).await
}

/// Anonymous routes see the session too, when there is one: the state says whether it is good. A
/// session that ended loses its cookie, and one past its half gets a later expiry, unless the route
/// set a cookie of its own (a sign-in, a sign-out, a password change).
pub(crate) async fn session(
    State(auth): State<Arc<UiAuth>>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut clear = false;
    let mut renewed = None;
    if let Some(value) = ticket::from_request(request.headers()) {
        match auth.check(&value) {
            Ok(session) => {
                renewed = session.renewed.clone();
                request.extensions_mut().insert(session);
            }
            Err(Rejected::Ended) => clear = true,
            Err(Rejected::NotOurs) => {}
        }
    }
    let mut response = next.run(request).await;
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

/// On the protected routes, inside [`session`]: a request with no good session gets 401.
pub(crate) async fn require_session(request: Request, next: Next) -> Response {
    if request.extensions().get::<SignedIn>().is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

/// What [`role_gate`] reads: the settings, whose role is read on every request, so a role change
/// takes effect at once, and the scope the routes belong to.
#[derive(Clone)]
pub(crate) struct RoleGate {
    pub settings: ConfigFile<KoppelSettings>,
    pub scope: RoleScope,
}

/// A route the current role does not have is not found.
pub(crate) async fn role_gate(
    State(gate): State<RoleGate>,
    request: Request,
    next: Next,
) -> Response {
    if !gate.scope.served_to(gate.settings.current().role) {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

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

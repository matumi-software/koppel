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
//! | [`no_store`] | the UI's `/api` | adds `Cache-Control: no-store` to every answer |
//! | [`auth_limit`] | the sign-in routes | 429 over the per-address limit |
//! | [`session`] | the UI's `/api` | puts the session on the request when the cookie is good, and renews or clears the cookie |
//! | [`role_gate`] | `/api/v1/server`, `/api/v1/client` and the network router | 404 when this machine's role does not have it |
//! | [`require_session`] | the protected routes | 401 without a session |
//!
//! Skakel's limits on its token and pairing calls are Skakel's own, behind these guards, per
//! caller's address (see `roles::skakel`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST, HeaderName, HeaderValue, ORIGIN,
    REFERRER_POLICY, SET_COOKIE, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::uri::Authority;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use koppel_config::ConfigFile;
use koppel_config::files::KoppelSettings;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::access::{UiAccess, UiReach, address_key};
use crate::api::ApiError;
use crate::auth::session::{clear_cookie, from_request};
use crate::auth::{SignedIn, UiAuth};
use crate::surface::{RoleScope, is_network_path};
use crate::sync::Unpoisoned;

/// The header every change to the UI's API carries.
pub(crate) const UI_HEADER: &str = "x-koppel-ui";
const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

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

/// `Cache-Control: no-store` on every answer of the router it wraps, the UI's `/api`: the client
/// secrets, the settings and the session state must not stay in a browser or proxy cache.
pub(crate) fn no_store<S: Clone + Send + Sync + 'static>(router: Router<S>) -> Router<S> {
    router.layer(SetResponseHeaderLayer::overriding(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
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

/// On the UI's `/api`: the request comes from this origin (the browser's `Sec-Fetch-Site`, else the
/// `Origin` against the host), and a change carries
/// `X-Koppel-UI: 1`, a header a page on another site cannot add without a CORS preflight this server
/// never grants.
pub(crate) async fn ui_origin(request: Request, next: Next) -> Response {
    if !from_the_ui(&request) {
        return ApiError::refused(
            StatusCode::FORBIDDEN,
            "forbidden",
            "This request did not come from Koppel's own UI.",
        )
        .into_response();
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
        return ApiError::too_many(
            "rate_limited",
            "Too many sign-in requests from this address. Wait a minute and try again.",
            60,
        )
        .into_response();
    }
    next.run(request).await
}

/// Anonymous routes see the session too, when there is one: the state says whether it is good. A
/// cookie that names no session (ended, run out, or never one of ours) is removed from the browser,
/// unless the route set a cookie of its own (a sign-in, a sign-out).
pub(crate) async fn session(
    State(auth): State<Arc<UiAuth>>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut clear = false;
    if let Some(value) = from_request(request.headers()) {
        match auth.check(&value) {
            Some(session) => {
                request.extensions_mut().insert(session);
            }
            None => clear = true,
        }
    }
    let mut response = next.run(request).await;
    if clear
        && !response.headers().contains_key(SET_COOKIE)
        && let Ok(value) = HeaderValue::from_str(&clear_cookie())
    {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    response
}

/// On the protected routes, inside [`session`]: a request with no good session gets 401.
pub(crate) async fn require_session(request: Request, next: Next) -> Response {
    if request.extensions().get::<SignedIn>().is_none() {
        return ApiError::refused(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Sign in to use Koppel.",
        )
        .into_response();
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
    let here = match headers.get(SEC_FETCH_SITE) {
        // A browser says where the request comes from, and a page cannot change it. `none` is the
        // person's own navigation, such as a typed address; a change still needs the header below.
        Some(site) => matches!(site.as_bytes(), b"same-origin" | b"none"),
        // A browser or a proxy that sends no Sec-Fetch-Site: the Origin, when there is one, must be
        // this request's own.
        None => headers
            .get(ORIGIN)
            .is_none_or(|origin| same_origin(request, origin)),
    };
    let change = request.method() != Method::GET && request.method() != Method::HEAD;
    here && (!change || headers.get(UI_HEADER).is_some_and(|v| v.as_bytes() == b"1"))
}

/// The origin's host and port against this request's own. The UI is HTTPS only, so an origin that
/// is not `https` is another origin; a proxy's `X-Forwarded-Proto` is not read.
fn same_origin(request: &Request, origin: &HeaderValue) -> bool {
    let Some(origin) = origin
        .to_str()
        .ok()
        .and_then(|origin| origin.strip_prefix("https://"))
        .filter(|rest| !rest.contains('@'))
        .and_then(|rest| rest.parse::<Authority>().ok())
    else {
        return false;
    };
    let own = request.uri().authority().cloned().or_else(|| {
        request
            .headers()
            .get(HOST)
            .and_then(|host| host.to_str().ok())
            .and_then(|host| host.parse::<Authority>().ok())
    });
    own.is_some_and(|own| {
        own.host().eq_ignore_ascii_case(origin.host())
            && own.port_u16().unwrap_or(443) == origin.port_u16().unwrap_or(443)
    })
}

/// A fixed one-minute window per caller, as ASP.NET's fixed window limiter: so many requests, then
/// 429 until the window is over.
pub(crate) struct RateLimit {
    permits: u32,
    window: Duration,
    callers: Mutex<Callers>,
}

#[derive(Default)]
struct Callers {
    windows: HashMap<String, (Instant, u32)>,
    /// When the windows that are over were last dropped.
    swept: Option<Instant>,
}

/// The most callers the limit keeps. A flood of source addresses cannot grow it past this: the
/// caller whose window began first is forgotten, and starts a fresh window if it comes back.
const MAX_CALLERS: usize = 4096;

impl RateLimit {
    pub(crate) fn per_minute(permits: u32) -> Self {
        Self {
            permits,
            window: Duration::from_secs(60),
            callers: Mutex::new(Callers::default()),
        }
    }

    pub(crate) fn allow(&self, caller: &str) -> bool {
        self.allow_at(caller, Instant::now())
    }

    fn allow_at(&self, caller: &str, now: Instant) -> bool {
        let mut callers = self.callers.locked();
        let Callers { windows, swept } = &mut *callers;
        // Windows that are over go once per window, so the map holds only callers of the last minute.
        if swept.is_none_or(|at| now.duration_since(at) >= self.window) {
            windows.retain(|_, (start, _)| now.duration_since(*start) < self.window);
            *swept = Some(now);
        }
        if windows.len() >= MAX_CALLERS && !windows.contains_key(caller) {
            let oldest = windows
                .iter()
                .min_by_key(|(_, (start, _))| *start)
                .map(|(caller, _)| caller.clone());
            if let Some(oldest) = oldest {
                windows.remove(&oldest);
            }
        }
        let entry = windows.entry(caller.to_owned()).or_insert((now, 0));
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

    fn request(host: &str, origin: Option<&str>, site: Option<&str>) -> Request {
        let mut builder = Request::builder()
            .uri("/api/v1/settings")
            .header(HOST, host);
        if let Some(origin) = origin {
            builder = builder.header(ORIGIN, origin);
        }
        if let Some(site) = site {
            builder = builder.header(SEC_FETCH_SITE, site);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn sec_fetch_site_decides_when_the_browser_sends_it() {
        for (site, ok) in [
            ("same-origin", true),
            ("none", true),
            ("same-site", false),
            ("cross-site", false),
            ("", false),
        ] {
            // The Origin is not looked at: the browser's own word is.
            let call = request(
                "koppel.lan:5240",
                Some("https://elsewhere.example"),
                Some(site),
            );
            assert_eq!(from_the_ui(&call), ok, "Sec-Fetch-Site {site:?}");
        }
    }

    #[test]
    fn without_sec_fetch_site_the_origin_is_compared_to_the_host() {
        let own = |origin: Option<&str>| from_the_ui(&request("koppel.lan:5240", origin, None));
        assert!(own(None));
        assert!(own(Some("https://koppel.lan:5240")));
        assert!(own(Some("https://KOPPEL.lan:5240")));
        assert!(!own(Some("https://koppel.lan")));
        assert!(!own(Some("https://koppel.lan:5241")));
        assert!(!own(Some("http://koppel.lan:5240")));
        assert!(!own(Some("https://elsewhere.example")));
        assert!(!own(Some("https://koppel.lan:5240/path")));
        assert!(!own(Some("https://koppel.lan:5240@elsewhere.example")));
        assert!(!own(Some("null")));
        let default_port = |origin| from_the_ui(&request("koppel.lan", Some(origin), None));
        assert!(default_port("https://koppel.lan"));
        assert!(default_port("https://koppel.lan:443"));
        let v6 = |origin| from_the_ui(&request("[::1]:5240", Some(origin), None));
        assert!(v6("https://[::1]:5240"));
        assert!(!v6("https://[::2]:5240"));
    }

    #[test]
    fn a_forwarded_scheme_is_not_read() {
        let mut call = request("koppel.lan:5240", Some("http://koppel.lan:5240"), None);
        call.headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("http"));
        assert!(!from_the_ui(&call));
    }

    #[test]
    fn the_limit_counts_per_caller_in_a_window() {
        let limit = RateLimit::per_minute(2);
        assert!(limit.allow("a") && limit.allow("a"));
        assert!(!limit.allow("a"));
        assert!(limit.allow("b"));
    }

    #[test]
    fn windows_that_are_over_are_dropped_and_the_map_stays_bounded() {
        let limit = RateLimit::per_minute(1);
        let start = Instant::now();
        for i in 0..100 {
            assert!(limit.allow_at(&format!("old{i}"), start));
        }
        let later = start + Duration::from_secs(61);
        assert!(limit.allow_at("new", later));
        assert_eq!(
            limit.callers.lock().unwrap().windows.len(),
            1,
            "the finished windows went"
        );
        for i in 0..MAX_CALLERS * 2 {
            limit.allow_at(&format!("flood{i}"), later);
        }
        assert!(limit.callers.lock().unwrap().windows.len() <= MAX_CALLERS);
    }
}

//! The guards in front of the routes, one request at a time through the routers with the address the
//! accept loop would give it, so each guard is tested alone, with `tower::ServiceExt::oneshot`:
//!
//! - which surface a request reaches follows from the router it is given, so a path of the other
//!   surface is not found;
//! - `Ui:AllowFrom`, the origin and `X-Koppel-UI`, the session and the role each answer as they say;
//! - a request cannot choose its guards by what it says about itself (a gRPC content type);
//! - a request without a caller address is refused.

mod common;

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::{CONTENT_TYPE, COOKIE, HOST, ORIGIN};
use axum::http::{Method, Request, Response, StatusCode};
use common::{FakeKernel, Node, TempDir};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// An address the UI does not answer (`Ui:AllowFrom=local` in the tests): not this machine's.
const STRANGER: [u8; 4] = [203, 0, 113, 9];
const LOCAL: [u8; 4] = [127, 0, 0, 1];

/// A request as the accept loop would hand it on: with the caller's address.
struct Call {
    request: Request<Body>,
}

impl Call {
    fn new(method: Method, path: &str, from: [u8; 4]) -> Self {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((from, 50_000))));
        Self { request }
    }

    fn get(path: &str, from: [u8; 4]) -> Self {
        Self::new(Method::GET, path, from)
    }

    fn post(path: &str, from: [u8; 4]) -> Self {
        Self::new(Method::POST, path, from)
    }

    fn header(mut self, name: impl TryInto<axum::http::HeaderName>, value: &str) -> Self {
        let name = name.try_into().ok().expect("a header name");
        self.request
            .headers_mut()
            .insert(name, value.parse().unwrap());
        self
    }

    fn json(mut self, body: &Value) -> Self {
        *self.request.body_mut() = Body::from(body.to_string());
        self.header(CONTENT_TYPE, "application/json")
    }

    fn cookie(self, cookie: &str) -> Self {
        self.header(COOKIE, cookie)
    }

    /// As the UI sends a change: the header the server requires.
    fn with_the_ui_header(self) -> Self {
        self.header("x-koppel-ui", "1")
    }

    fn without_the_caller(mut self) -> Self {
        self.request
            .extensions_mut()
            .remove::<ConnectInfo<SocketAddr>>();
        self
    }

    async fn through(self, router: &Router) -> Response<Body> {
        router.clone().oneshot(self.request).await.unwrap()
    }

    async fn status(self, router: &Router) -> StatusCode {
        self.through(router).await.status()
    }
}

async fn text(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

const CONTENT_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

fn security_headers(response: &Response<Body>, what: &str) {
    for (name, value) in [
        ("x-frame-options", "DENY"),
        ("content-security-policy", CONTENT_POLICY),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ] {
        assert_eq!(
            response.headers().get(name).and_then(|v| v.to_str().ok()),
            Some(value),
            "{name} on {what}"
        );
    }
}

async fn start(role: &str) -> (TempDir, Node) {
    let dir = TempDir::new(role);
    let node = Node::start(dir.path(), role, None, FakeKernel::new()).await;
    (dir, node)
}

async fn set_role(node: &Node, role: &str) {
    let (status, body) = node
        .call(
            Method::PATCH,
            "/api/v1/settings",
            Some(json!({ "role": role })),
        )
        .await;
    assert_eq!(status, 202, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn each_surface_is_its_routers_and_a_path_of_the_other_is_not_found() {
    let (_dir, node) = start("server").await;
    let routers = node.app.routers();

    // The UI router, from this machine: its own paths, and nothing of the network surface.
    for path in ["/healthz", "/", "/some/spa/route", "/api/v1/auth/state"] {
        assert_eq!(
            Call::get(path, LOCAL).status(&routers.ui).await,
            StatusCode::OK,
            "{path} on the UI router"
        );
    }
    for path in [
        "/skakel/info",
        "/skakel/v3/token",
        "/api/remote/devices",
        "/skakel.v3.ControlService/Connect",
        "/koppel.remote.v1.RemoteDevices/Watch",
    ] {
        assert_eq!(
            Call::get(path, LOCAL).status(&routers.ui).await,
            StatusCode::NOT_FOUND,
            "{path} on the UI router"
        );
    }

    // The network router: the network surface, to anyone, and nothing of the UI.
    assert_eq!(
        Call::get("/skakel/info", STRANGER)
            .status(&routers.network)
            .await,
        StatusCode::OK
    );
    assert_eq!(
        Call::get("/api/remote/devices", STRANGER)
            .status(&routers.network)
            .await,
        StatusCode::UNAUTHORIZED
    );
    for path in [
        "/healthz",
        "/",
        "/api/v1/auth/state",
        "/api/v1/settings",
        "/skakel.v3.Other",
    ] {
        assert_eq!(
            Call::get(path, LOCAL).status(&routers.network).await,
            StatusCode::NOT_FOUND,
            "{path} on the network router"
        );
    }

    // On one port the two go behind one router, which hands each path to its surface.
    let merged = node.app.router();
    assert_eq!(
        Call::get("/skakel/info", STRANGER).status(&merged).await,
        StatusCode::OK
    );
    assert_eq!(
        Call::get("/healthz", LOCAL).status(&merged).await,
        StatusCode::OK
    );
    assert_eq!(
        Call::get("/healthz", STRANGER).status(&merged).await,
        StatusCode::FORBIDDEN,
        "the UI's guards still apply on a port the network shares"
    );

    // The paths are matched as they are written: another case is the UI's, not the network's.
    assert_eq!(
        Call::get("/SKAKEL/info", STRANGER).status(&merged).await,
        StatusCode::FORBIDDEN
    );

    drop(routers);
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn allow_from_turns_an_address_away_from_everything_of_the_ui() {
    let (_dir, node) = start("server").await;
    let routers = node.app.routers();

    for path in [
        "/",
        "/healthz",
        "/api/v1/auth/state",
        "/api/v1/settings",
        "/no/such/path",
    ] {
        let response = Call::get(path, STRANGER).through(&routers.ui).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        security_headers(&response, path);
        assert!(
            text(response).await.contains("Ui:AllowFrom"),
            "{path} says which setting decides"
        );
    }
    assert_eq!(
        Call::post("/api/v1/auth/login", STRANGER)
            .with_the_ui_header()
            .json(&json!({ "password": "x" }))
            .status(&routers.ui)
            .await,
        StatusCode::FORBIDDEN
    );

    // This machine is let in, and so is a neighbour the setting would take.
    assert_eq!(
        Call::get("/healthz", LOCAL).status(&routers.ui).await,
        StatusCode::OK
    );
    drop(routers);
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_origin_and_the_ui_header_guard_the_api_and_nothing_else() {
    let (_dir, node) = start("server").await;
    let ui = node.app.routers().ui;
    let authority = format!("127.0.0.1:{}", node.port);
    let own = format!("https://{authority}");
    let get = |path: &str, origin: Option<&str>| {
        let mut call = Call::get(path, LOCAL).header(HOST, &authority);
        if let Some(origin) = origin {
            call = call.header(ORIGIN, origin);
        }
        call
    };

    // A request from another site is refused on the anonymous routes and the protected ones alike,
    // before the session is looked at.
    for origin in [
        "https://elsewhere.example",
        "null",
        "http://127.0.0.1",
        &format!("https://127.0.0.1:{}", node.port.wrapping_add(1)),
    ] {
        for path in [
            "/api/v1/auth/state",
            "/api/v1/settings",
            "/api/v1/server/identity",
        ] {
            assert_eq!(
                get(path, Some(origin)).status(&ui).await,
                StatusCode::FORBIDDEN,
                "{path} from {origin}"
            );
        }
    }
    assert_eq!(
        get("/api/v1/auth/state", Some(&own)).status(&ui).await,
        StatusCode::OK
    );
    assert_eq!(
        get("/api/v1/auth/state", None).status(&ui).await,
        StatusCode::OK
    );
    // Not the API: the UI's files and /healthz take any origin.
    assert_eq!(
        get("/", Some("https://elsewhere.example"))
            .status(&ui)
            .await,
        StatusCode::OK
    );

    // A change needs the UI header, whoever it is from, and a read does not.
    for path in [
        "/api/v1/auth/login",
        "/api/v1/auth/setup",
        "/api/v1/settings",
    ] {
        for method in [Method::POST, Method::PATCH] {
            assert_eq!(
                Call::new(method.clone(), path, LOCAL)
                    .header(HOST, &authority)
                    .json(&json!({}))
                    .status(&ui)
                    .await,
                StatusCode::FORBIDDEN,
                "{method} {path} without the header"
            );
            assert_eq!(
                Call::new(method.clone(), path, LOCAL)
                    .header(HOST, &authority)
                    .header("x-koppel-ui", "yes")
                    .json(&json!({}))
                    .status(&ui)
                    .await,
                StatusCode::FORBIDDEN,
                "{method} {path} with the wrong header"
            );
        }
    }
    // With it, the request reaches what answers: a wrong password, not a refusal of the request.
    let response = Call::post("/api/v1/auth/login", LOCAL)
        .header(HOST, &authority)
        .with_the_ui_header()
        .json(&json!({ "password": "not the password" }))
        .status(&ui)
        .await;
    assert_eq!(response, StatusCode::UNAUTHORIZED);

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sec_fetch_site_decides_before_the_origin_is_compared() {
    let (_dir, node) = start("server").await;
    let ui = node.app.routers().ui;
    let authority = format!("127.0.0.1:{}", node.port);
    let get = |site: &str| {
        Call::get("/api/v1/auth/state", LOCAL)
            .header(HOST, &authority)
            .header("sec-fetch-site", site)
    };

    for site in ["same-origin", "none"] {
        assert_eq!(get(site).status(&ui).await, StatusCode::OK, "{site}");
    }
    for site in ["cross-site", "same-site", "other"] {
        assert_eq!(
            get(site).status(&ui).await,
            StatusCode::FORBIDDEN,
            "{site}, even with an Origin that matches"
        );
        assert_eq!(
            get(site)
                .header(ORIGIN, &format!("https://{authority}"))
                .status(&ui)
                .await,
            StatusCode::FORBIDDEN,
            "{site} with an Origin that matches"
        );
    }
    // A change that is same-origin still needs the UI header.
    assert_eq!(
        Call::post("/api/v1/auth/login", LOCAL)
            .header(HOST, &authority)
            .header("sec-fetch-site", "same-origin")
            .json(&json!({}))
            .status(&ui)
            .await,
        StatusCode::FORBIDDEN
    );
    // No Sec-Fetch-Site: the Origin against the host, over HTTPS only, and a forwarded scheme does not help.
    assert_eq!(
        get("same-origin").status(&ui).await,
        StatusCode::OK,
        "the helper sends the header"
    );
    for origin in [
        format!("http://{authority}"),
        "https://elsewhere.example".to_owned(),
    ] {
        assert_eq!(
            Call::get("/api/v1/auth/state", LOCAL)
                .header(HOST, &authority)
                .header(ORIGIN, &origin)
                .header("x-forwarded-proto", "https")
                .status(&ui)
                .await,
            StatusCode::FORBIDDEN,
            "{origin}"
        );
    }

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_answer_of_the_api_says_no_store_and_the_ui_files_keep_their_own_caching() {
    let (_dir, node) = start("server").await;
    let ui = node.app.routers().ui;
    let authority = format!("127.0.0.1:{}", node.port);
    let cache = |response: &Response<Body>| {
        response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };

    for (path, signed_in, origin) in [
        ("/api/v1/auth/state", false, None),
        ("/api/v1/settings", true, None),
        ("/api/v1/settings", false, None),
        ("/api/v1/server/identity", true, None),
        ("/api/v1/nothing/here", true, None),
        ("/api/v1/events", true, None),
        ("/api/v1/settings", true, Some("https://elsewhere.example")),
    ] {
        let mut call = Call::get(path, LOCAL).header(HOST, &authority);
        if signed_in {
            call = call.cookie(node.cookie());
        }
        if let Some(origin) = origin {
            call = call.header(ORIGIN, origin);
        }
        let response = call.through(&ui).await;
        assert_eq!(
            cache(&response).as_deref(),
            Some("no-store"),
            "{path} ({})",
            response.status()
        );
    }
    let secret = Call::get("/api/v1/server/clients/none/secret", LOCAL)
        .header(HOST, &authority)
        .cookie(node.cookie())
        .through(&ui)
        .await;
    assert_eq!(cache(&secret).as_deref(), Some("no-store"));

    // Not the API: the page keeps what the file server says about it.
    let page = Call::get("/", LOCAL).through(&ui).await;
    assert_ne!(cache(&page).as_deref(), Some("no-store"));

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_guards_every_api_route_that_is_not_anonymous() {
    let (_dir, node) = start("both").await;
    let ui = node.app.routers().ui;
    let cookie = node.cookie().to_owned();

    // Signed out: 401 on the protected routes, and on a path no route has.
    for (method, path) in [
        (Method::GET, "/api/v1/settings"),
        (Method::GET, "/api/v1/activity"),
        (Method::GET, "/api/v1/events"),
        (Method::GET, "/api/v1/system/info"),
        (Method::POST, "/api/v1/auth/logout"),
        (Method::POST, "/api/v1/auth/password"),
        (Method::GET, "/api/v1/server/identity"),
        (Method::GET, "/api/v1/client/connection"),
        (Method::GET, "/api/v1/not-a-route-yet"),
        (Method::DELETE, "/api/v1/not/a/route/either"),
    ] {
        assert_eq!(
            Call::new(method.clone(), path, LOCAL)
                .with_the_ui_header()
                .json(&json!({}))
                .status(&ui)
                .await,
            StatusCode::UNAUTHORIZED,
            "{method} {path} signed out"
        );
    }
    // A cookie that is not a session is none.
    for forged in ["koppel.ui=nope", "koppel.ui=", "other=1"] {
        assert_eq!(
            Call::get("/api/v1/settings", LOCAL)
                .cookie(forged)
                .status(&ui)
                .await,
            StatusCode::UNAUTHORIZED,
            "{forged}"
        );
    }

    // Anonymous routes answer either way, and the state says which.
    let state = |call: Call| async {
        let response = call.through(&ui).await;
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_str::<Value>(&text(response).await).unwrap()
    };
    assert_eq!(
        state(Call::get("/api/v1/auth/state", LOCAL)).await["signedIn"],
        json!(false)
    );
    assert_eq!(
        state(Call::get("/api/v1/auth/state", LOCAL).cookie(&cookie)).await["signedIn"],
        json!(true)
    );

    // Signed in: the routes answer, and a path no route has is 404, not 401.
    for path in [
        "/api/v1/settings",
        "/api/v1/activity",
        "/api/v1/system/info",
    ] {
        assert_eq!(
            Call::get(path, LOCAL).cookie(&cookie).status(&ui).await,
            StatusCode::OK,
            "{path}"
        );
    }
    assert_eq!(
        Call::get("/api/v1/not-a-route-yet", LOCAL)
            .cookie(&cookie)
            .status(&ui)
            .await,
        StatusCode::NOT_FOUND
    );

    // The path as written is the path routed: another case, an encoded letter or an extra slash is
    // no way round the session. Signed out, each is refused or is the UI's page, never the data.
    for path in [
        "/API/settings",
        "/api/v1/%73ettings",
        "/api/v1/settings/",
        "//api/v1/settings",
    ] {
        let response = Call::get(path, LOCAL).through(&ui).await;
        let status = response.status();
        let body = text(response).await;
        assert!(
            !body.contains("shareOnStartup"),
            "{path} signed out answered {status} with the settings"
        );
    }

    // A session that ended loses its cookie: here one that is not ours is left alone.
    let response = Call::get("/api/v1/auth/state", LOCAL)
        .cookie("koppel.ui=nope")
        .through(&ui)
        .await;
    assert!(response.headers().get("set-cookie").is_none());

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_role_gate_answers_before_the_session_and_follows_the_role() {
    let (_dir, node) = start("server").await;
    let routers = node.app.routers();
    let (ui, network) = (routers.ui, routers.network);
    let cookie = node.cookie().to_owned();

    // A server: its own routes need the session, the client's are not found, signed in or not.
    let identity = "/api/v1/server/identity";
    let connection = "/api/v1/client/connection";
    assert_eq!(
        Call::get(identity, LOCAL).status(&ui).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        Call::get(identity, LOCAL).cookie(&cookie).status(&ui).await,
        StatusCode::OK
    );
    for cookie in [None, Some(cookie.as_str())] {
        let mut call = Call::get(connection, LOCAL);
        if let Some(cookie) = cookie {
            call = call.cookie(cookie);
        }
        assert_eq!(call.status(&ui).await, StatusCode::NOT_FOUND, "{cookie:?}");
    }
    assert_eq!(
        Call::get("/skakel/info", STRANGER).status(&network).await,
        StatusCode::OK
    );

    // A client: the other way round, and the network surface is gone, from the next request on.
    set_role(&node, "client").await;
    assert_eq!(
        Call::get(identity, LOCAL).status(&ui).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        Call::get(identity, LOCAL).cookie(&cookie).status(&ui).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        Call::get(connection, LOCAL).status(&ui).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        Call::get(connection, LOCAL)
            .cookie(&cookie)
            .status(&ui)
            .await,
        StatusCode::OK
    );
    for path in [
        "/skakel/info",
        "/api/remote/devices",
        "/skakel.v3.ControlService/Connect",
    ] {
        assert_eq!(
            Call::get(path, STRANGER).status(&network).await,
            StatusCode::NOT_FOUND,
            "{path} on a client"
        );
    }

    // Both: everything.
    set_role(&node, "both").await;
    for path in [identity, connection] {
        assert_eq!(
            Call::get(path, LOCAL).cookie(&cookie).status(&ui).await,
            StatusCode::OK,
            "{path} on both"
        );
    }
    assert_eq!(
        Call::get("/skakel/info", STRANGER).status(&network).await,
        StatusCode::OK
    );

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_without_the_callers_address_is_refused() {
    let (_dir, node) = start("server").await;
    let routers = node.app.routers();
    let merged = node.app.router();
    let (ui, network) = (routers.ui, routers.network);

    for (router, path) in [
        (&ui, "/healthz"),
        (&ui, "/api/v1/auth/state"),
        (&network, "/skakel/info"),
        (&merged, "/healthz"),
        (&merged, "/skakel/info"),
    ] {
        let response = Call::get(path, LOCAL)
            .without_the_caller()
            .through(router)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{path}"
        );
        security_headers(&response, path);
    }

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_answer_carries_the_security_headers() {
    let (_dir, node) = start("server").await;
    let routers = node.app.routers();
    let cookie = node.cookie().to_owned();

    for (router, call) in [
        (&routers.ui, Call::get("/healthz", LOCAL)),
        (&routers.ui, Call::get("/", LOCAL)),
        (&routers.ui, Call::get("/api/v1/settings", LOCAL)),
        (
            &routers.ui,
            Call::get("/api/v1/settings", LOCAL).cookie(&cookie),
        ),
        (&routers.ui, Call::get("/api/v1/client/connection", LOCAL)),
        (&routers.ui, Call::get("/healthz", STRANGER)),
        (&routers.ui, Call::get("/skakel/info", LOCAL)),
        (&routers.network, Call::get("/skakel/info", STRANGER)),
        (&routers.network, Call::get("/api/remote/devices", STRANGER)),
        (&routers.network, Call::get("/healthz", STRANGER)),
    ] {
        let path = call.request.uri().path().to_owned();
        let response = call.through(router).await;
        security_headers(&response, &format!("{path} ({})", response.status()));
    }

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_grpc_content_type_does_not_skip_the_ui_guards() {
    let (_dir, node) = start("server").await;
    let merged = node.app.router();

    // From an address Ui:AllowFrom does not take, whatever the content type says.
    for content_type in [
        "application/json",
        "application/grpc",
        "application/grpc+json",
        "application/grpc+proto",
        "Application/GRPC+JSON; charset=utf-8",
    ] {
        for path in [
            "/api/v1/auth/login",
            "/api/v1/auth/setup",
            "/api/v1/settings",
        ] {
            assert_eq!(
                Call::post(path, STRANGER)
                    .header(CONTENT_TYPE, content_type)
                    .status(&merged)
                    .await,
                StatusCode::FORBIDDEN,
                "{path} as {content_type} from a stranger"
            );
        }
    }

    // From this machine, a change still needs the header the UI adds.
    for content_type in ["application/grpc+json", "application/grpc"] {
        for path in ["/api/v1/auth/login", "/api/v1/auth/setup"] {
            assert_eq!(
                Call::post(path, LOCAL)
                    .header(CONTENT_TYPE, content_type)
                    .status(&merged)
                    .await,
                StatusCode::FORBIDDEN,
                "{path} as {content_type} without X-Koppel-UI"
            );
        }
    }

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_grpc_call_reaches_skakel_on_the_network_surface() {
    let (_dir, node) = start("server").await;
    let merged = node.app.router();

    // From the same stranger the UI turns away: Skakel's and Koppel's own services answer anyone,
    // and say the call has no token.
    for path in [
        "/skakel.v3.ControlService/Connect",
        "/skakel.v3.StreamService/Open",
        "/koppel.remote.v1.RemoteDevices/Watch",
    ] {
        let response = Call::post(path, STRANGER)
            .header(CONTENT_TYPE, "application/grpc")
            .through(&merged)
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let grpc_status = response
            .headers()
            .get("grpc-status")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        assert_eq!(
            grpc_status.as_deref(),
            Some("16"),
            "{path} answers UNAUTHENTICATED, as Skakel's own"
        );
    }

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_only_machine_serves_no_grpc() {
    let (_dir, node) = start("client").await;
    let merged = node.app.router();

    for path in [
        "/skakel.v3.ControlService/Connect",
        "/koppel.remote.v1.RemoteDevices/Watch",
    ] {
        assert_eq!(
            Call::post(path, STRANGER)
                .header(CONTENT_TYPE, "application/grpc")
                .status(&merged)
                .await,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    node.stop().await;
}

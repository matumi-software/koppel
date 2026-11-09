//! How Skakel's requests reach Skakel's server. skakel-server 0.2.0 serves only on a TCP listener of
//! its own, so it runs on a loopback port, and Koppel's web ports, which serve the UI, the plain HTTP
//! redirect and the port and role rules on the same port as 0.1.x did, hand it what is Skakel's: the
//! gRPC calls (by their content type), `/skakel/*` and `/api/remote/*`.
//!
//! Each connection to a web port gets one HTTP/2 connection of its own to the loopback port, made
//! when its first Skakel request comes and closed with it. So a client's streams keep a connection
//! each, as the protocol asks, and when a client goes away (or stops answering pings) its calls on
//! Skakel's side end with it.
//!
//! What Skakel's server sees as the caller's address is loopback. Its per-address limits on tokens
//! and pairing calls are kept here instead, per real address (`RateLimits:TokenPerMinute` and
//! `RateLimits:PairPerMinute`), and the address a pairing request came from is read off the answer
//! that opened it, for the server's UI. Its other per-address pairing limits (5 opens in 5 minutes,
//! 3 open at once) count every caller together. A Skakel release that lets an app serve on its own
//! listener removes this module.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONNECTION, CONTENT_TYPE, HOST, HeaderName, TE};
use axum::http::{HeaderValue, Method, StatusCode, Uri, Version};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use hyper::client::conn::http2::SendRequest;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::access::address_key;

/// How long making the loopback connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest pairing answer read to learn the request's id. Skakel's are far smaller.
const MAX_PAIR_ANSWER: usize = 64 * 1024;

/// Pairing requests whose address is remembered at most, so callers cannot grow it for ever.
const MAX_REMEMBERED: usize = 1024;

/// Skakel's pairing path, where an answer to a POST opens a request.
const PAIR_PATH: &str = "/skakel/v2/pair";

/// True for a request that is Skakel's: a gRPC call, by its content type.
pub(crate) fn is_grpc(request: &Request) -> bool {
    request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(skakel_protocol::is_grpc_content_type)
}

/// Hands requests to Skakel's server on its loopback port. One per process; the port moves when
/// the server starts again.
pub(crate) struct Front {
    target: Mutex<SocketAddr>,
    connector: TlsConnector,
    /// The caller's address of each pairing request that is still open, by its id.
    pair_from: Mutex<HashMap<String, String>>,
}

/// The loopback connection of one connection to a web port. Every request of that connection
/// carries a handle; the connection to Skakel's server closes when the last one is gone.
#[derive(Default)]
pub struct FrontConnection {
    sender: tokio::sync::Mutex<Option<(SocketAddr, SendRequest<Body>)>>,
}

impl Front {
    pub(crate) fn new(target: SocketAddr) -> Arc<Self> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("the ring provider speaks the default protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(OwnServer(provider)))
            .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec()];
        Arc::new(Self {
            target: Mutex::new(target),
            connector: TlsConnector::from(Arc::new(config)),
            pair_from: Mutex::new(HashMap::new()),
        })
    }

    /// Points new loopback connections at a server that started again on another port.
    pub(crate) fn retarget(&self, target: SocketAddr) {
        *self.target.lock().unwrap_or_else(PoisonError::into_inner) = target;
    }

    fn target(&self) -> SocketAddr {
        *self.target.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The address a pairing request came from, when this front saw it open.
    pub(crate) fn pair_from(&self, request_id: &str) -> Option<String> {
        self.pair_from
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(request_id)
            .cloned()
    }

    /// Forgets the addresses of requests that are no longer open.
    pub(crate) fn keep_pair_from(&self, open: &[String]) {
        self.pair_from
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id, _| open.contains(id));
    }

    /// Sends `request` from `remote` to Skakel's server over `connection`'s loopback connection,
    /// and gives back its answer as it comes, trailers included.
    pub(crate) async fn forward(
        &self,
        connection: &FrontConnection,
        remote: SocketAddr,
        request: Request,
    ) -> Response {
        let grpc = is_grpc(&request);
        let opens_pairing = request.method() == Method::POST
            && request.uri().path().eq_ignore_ascii_case(PAIR_PATH);
        let mut sender = match self.sender(connection).await {
            Ok(sender) => sender,
            Err(e) => {
                tracing::warn!("Could not reach Skakel's server on loopback: {e}");
                return unavailable(grpc);
            }
        };
        let request = match self.outbound(request) {
            Ok(request) => request,
            Err(e) => {
                tracing::debug!("A request could not be handed to Skakel's server: {e}");
                return StatusCode::BAD_REQUEST.into_response();
            }
        };
        let response = match sender.send_request(request).await {
            Ok(response) => response,
            Err(e) => {
                tracing::debug!("Skakel's server did not answer a request: {e}");
                *connection.sender.lock().await = None;
                return unavailable(grpc);
            }
        };
        let (parts, body) = response.into_parts();
        if opens_pairing && parts.status == StatusCode::OK {
            let Ok(collected) = http_body_util::Limited::new(body, MAX_PAIR_ANSWER)
                .collect()
                .await
            else {
                return StatusCode::BAD_GATEWAY.into_response();
            };
            let bytes = collected.to_bytes();
            if let Some(id) = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v["requestId"].as_str().map(str::to_owned))
            {
                let mut known = self
                    .pair_from
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if known.len() >= MAX_REMEMBERED {
                    known.clear();
                }
                known.insert(id, address_key(remote.ip()).to_string());
            }
            return Response::from_parts(parts, Body::from(bytes));
        }
        Response::from_parts(parts, Body::new(body))
    }

    /// The loopback connection of `connection`, made now when it has none, or none to the server's
    /// current port.
    async fn sender(&self, connection: &FrontConnection) -> Result<SendRequest<Body>, String> {
        let target = self.target();
        let mut held = connection.sender.lock().await;
        if let Some((at, sender)) = held.as_ref()
            && *at == target
            && !sender.is_closed()
        {
            return Ok(sender.clone());
        }
        let sender = tokio::time::timeout(CONNECT_TIMEOUT, self.connect(target))
            .await
            .map_err(|_| "it took too long".to_owned())??;
        *held = Some((target, sender.clone()));
        Ok(sender)
    }

    async fn connect(&self, target: SocketAddr) -> Result<SendRequest<Body>, String> {
        let tcp = TcpStream::connect(target)
            .await
            .map_err(|e| e.to_string())?;
        let _ = tcp.set_nodelay(true);
        let name = ServerName::from(target.ip());
        let tls = self
            .connector
            .connect(name, tcp)
            .await
            .map_err(|e| e.to_string())?;
        let mut builder = hyper::client::conn::http2::Builder::new(TokioExecutor::new());
        builder.timer(TokioTimer::new()).adaptive_window(true);
        let (sender, connection) = builder
            .handshake(TokioIo::new(tls))
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::debug!("A loopback connection to Skakel's server ended: {e}");
            }
        });
        Ok(sender)
    }

    /// The request as it goes to the loopback port: HTTP/2 to its address, without the headers
    /// that belong to the hop from the caller.
    fn outbound(&self, request: Request) -> Result<hyper::Request<Body>, String> {
        let (mut parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or("/", |p| p.as_str())
            .to_owned();
        parts.uri =
            Uri::try_from(format!("https://{}{path}", self.target())).map_err(|e| e.to_string())?;
        parts.version = Version::HTTP_2;
        let headers = &mut parts.headers;
        let listed: Vec<HeaderName> = headers
            .get_all(CONNECTION)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .filter_map(|name| HeaderName::try_from(name.trim()).ok())
            .collect();
        for name in listed {
            headers.remove(name);
        }
        for name in [
            "connection",
            "keep-alive",
            "proxy-connection",
            "transfer-encoding",
            "upgrade",
        ] {
            headers.remove(name);
        }
        headers.remove(HOST);
        if headers
            .get(TE)
            .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"trailers"))
        {
            headers.remove(TE);
        }
        parts.extensions = Default::default();
        Ok(hyper::Request::from_parts(parts, body))
    }
}

/// What a caller gets when Skakel's server cannot be reached: gRPC's `UNAVAILABLE` for a call, 503
/// for the rest.
fn unavailable(grpc: bool) -> Response {
    if grpc {
        let mut response = Response::new(Body::empty());
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/grpc"));
        headers.insert("grpc-status", HeaderValue::from_static("14"));
        headers.insert(
            "grpc-message",
            HeaderValue::from_static("Skakel%27s%20server%20is%20not%20running."),
        );
        return response;
    }
    StatusCode::SERVICE_UNAVAILABLE.into_response()
}

/// The loopback port is this process's own Skakel server, which presents the same certificate as
/// the web ports. Nothing else listens there, so the certificate needs no check; the handshake's
/// signature still does.
#[derive(Debug)]
struct OwnServer(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for OwnServer {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The per-address limits Skakel's server would keep on its token and pairing calls, kept here
/// because it sees every caller as loopback.
pub(crate) struct SkakelLimits {
    token: crate::guard::RateLimit,
    pair: crate::guard::RateLimit,
}

impl SkakelLimits {
    pub(crate) fn new(token_per_minute: u32, pair_per_minute: u32) -> Self {
        Self {
            token: crate::guard::RateLimit::per_minute(token_per_minute),
            pair: crate::guard::RateLimit::per_minute(pair_per_minute),
        }
    }

    /// False when `path` is Skakel's token or pairing path and the caller is over its limit for
    /// this minute.
    pub(crate) fn allow(&self, path: &str, remote: IpAddr) -> bool {
        let on = |base: &str| crate::surface::starts_with_segments(path, base);
        let caller = address_key(remote).to_string();
        if on("/skakel/v2/token") {
            self.token.allow(&caller)
        } else if on(PAIR_PATH) {
            self.pair.allow(&caller)
        } else {
            true
        }
    }
}

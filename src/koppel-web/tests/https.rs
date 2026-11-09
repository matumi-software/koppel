//! The server on loopback as a browser sees it: HTTPS with the generated certificate trusted, HTTP/2
//! and HTTP/1.1 by ALPN, `/healthz`, the embedded UI, and a certificate replaced while it runs.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use flate2::read::GzDecoder;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::client::conn::http2::SendRequest;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use koppel_web::CertificateSlot;
use koppel_web::tls::{self, SelfSigned};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

struct Server {
    address: SocketAddr,
    slot: Arc<CertificateSlot>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(certificate: SelfSigned) -> Self {
        let slot = CertificateSlot::new(certificate.certified);
        let acceptor = tls::acceptor(slot.clone()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(koppel_web::serve(
            listener,
            acceptor,
            koppel_web::router(),
            async {
                let _ = stopped.await;
            },
        ));
        Self {
            address,
            slot,
            stop,
            task,
        }
    }

    /// Stops the server and checks `serve` returns.
    async fn stop(self) {
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("serve returns after stop")
            .unwrap();
    }
}

/// A client that trusts only `trusted` and offers one ALPN protocol. It keeps rustls's default
/// session cache, so it would resume a session if the server let it.
fn client(trusted: &[&CertificateDer<'static>], alpn: &[u8]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for der in trusted {
        roots.add((*der).clone()).unwrap();
    }
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = vec![alpn.to_vec()];
    Arc::new(config)
}

async fn connect(
    address: SocketAddr,
    config: Arc<ClientConfig>,
) -> std::io::Result<TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(address).await?;
    let name = ServerName::try_from("localhost").unwrap();
    TlsConnector::from(config).connect(name, tcp).await
}

fn peer_certificate(tls: &TlsStream<TcpStream>) -> CertificateDer<'static> {
    tls.get_ref().1.peer_certificates().unwrap()[0].clone()
}

async fn http2(tls: TlsStream<TcpStream>) -> SendRequest<Empty<Bytes>> {
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
    let (sender, connection) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
            .await
            .unwrap();
    tokio::spawn(connection);
    sender
}

async fn get(
    sender: &mut SendRequest<Empty<Bytes>>,
    path: &str,
    accept_encoding: Option<&str>,
) -> Response<Bytes> {
    let mut request = Request::get(format!("https://localhost{path}"));
    if let Some(value) = accept_encoding {
        request = request.header("accept-encoding", value);
    }
    let response = sender
        .send_request(request.body(Empty::new()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    Response::from_parts(parts, body.collect().await.unwrap().to_bytes())
}

fn header<'a>(response: &'a Response<Bytes>, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .map(|value| value.to_str().unwrap())
}

#[tokio::test]
async fn healthz_answers_ok_over_http2() {
    let certificate = tls::self_signed(&["localhost"]).unwrap();
    let trusted = certificate.der.clone();
    let server = Server::start(certificate).await;

    let tls = connect(server.address, client(&[&trusted], b"h2"))
        .await
        .unwrap();
    let mut sender = http2(tls).await;
    let response = get(&mut sender, "/healthz", None).await;

    assert_eq!(response.version(), Version::HTTP_2);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_ref(), b"ok");
    server.stop().await;
}

#[tokio::test]
async fn healthz_answers_ok_over_http1() {
    let certificate = tls::self_signed(&["localhost"]).unwrap();
    let trusted = certificate.der.clone();
    let server = Server::start(certificate).await;

    let tls = connect(server.address, client(&[&trusted], b"http/1.1"))
        .await
        .unwrap();
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(connection);
    let request = Request::get("/healthz")
        .header("host", "localhost")
        .body(Empty::<Bytes>::new())
        .unwrap();
    let response = sender.send_request(request).await.unwrap();

    assert_eq!(response.version(), Version::HTTP_11);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        b"ok"
    );
    server.stop().await;
}

#[tokio::test]
async fn the_ui_comes_gzipped_or_plain_as_the_client_asks() {
    let certificate = tls::self_signed(&["localhost"]).unwrap();
    let trusted = certificate.der.clone();
    let server = Server::start(certificate).await;
    let tls = connect(server.address, client(&[&trusted], b"h2"))
        .await
        .unwrap();
    let mut sender = http2(tls).await;

    let plain = get(&mut sender, "/", None).await;
    let gzipped = get(&mut sender, "/", Some("br, gzip")).await;
    let page = get(&mut sender, "/devices", Some("gzip")).await;

    for response in [&plain, &gzipped, &page] {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header(response, "content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(header(response, "cache-control"), Some("no-cache"));
        assert_eq!(header(response, "vary"), Some("accept-encoding"));
    }
    assert_eq!(header(&plain, "content-encoding"), None);
    assert!(
        plain
            .body()
            .to_ascii_lowercase()
            .starts_with(b"<!doctype html")
    );
    assert_eq!(header(&gzipped, "content-encoding"), Some("gzip"));
    let mut inflated = Vec::new();
    GzDecoder::new(gzipped.body().as_ref())
        .read_to_end(&mut inflated)
        .unwrap();
    assert_eq!(inflated, plain.body().as_ref());
    assert_eq!(page.body(), gzipped.body());

    // The built UI names its script under assets/; the placeholder of a build without the UI has none.
    let index = String::from_utf8(inflated).unwrap();
    if let Some(start) = index.find("/assets/") {
        let path = &index[start..start + index[start..].find('"').unwrap()];
        let script = get(&mut sender, path, Some("gzip")).await;
        assert_eq!(script.status(), StatusCode::OK);
        assert_eq!(
            header(&script, "cache-control"),
            Some("public, max-age=31536000, immutable")
        );
    }

    let missing = get(&mut sender, "/assets/missing-1234.js", Some("gzip")).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    server.stop().await;
}

#[tokio::test]
async fn a_replaced_certificate_is_presented_from_the_next_connection() {
    let first = tls::self_signed(&["localhost"]).unwrap();
    let second = tls::self_signed(&["localhost"]).unwrap();
    let (first_der, second_der) = (first.der.clone(), second.der.clone());
    let server = Server::start(first).await;
    // One client config for both connections, so its session cache would let it resume.
    let config = client(&[&first_der, &second_der], b"h2");

    let before = connect(server.address, config.clone()).await.unwrap();
    assert_eq!(peer_certificate(&before), first_der);
    let mut open = http2(before).await;
    assert_eq!(
        get(&mut open, "/healthz", None).await.status(),
        StatusCode::OK
    );

    server.slot.replace(second.certified);

    let after = connect(server.address, config).await.unwrap();
    assert_eq!(peer_certificate(&after), second_der);
    let mut sender = http2(after).await;
    assert_eq!(
        get(&mut sender, "/healthz", None).await.status(),
        StatusCode::OK
    );
    // A connection made before the switch carries on.
    assert_eq!(
        get(&mut open, "/healthz", None).await.status(),
        StatusCode::OK
    );
    // A client that trusts only the old certificate is refused.
    assert!(
        connect(server.address, client(&[&first_der], b"h2"))
            .await
            .is_err()
    );
    server.stop().await;
}

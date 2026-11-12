//! `koppel healthcheck`: the image's health check, as 0.1.x's `HealthProbe`. It sends
//! `GET /healthz` to the UI port and exits 0 on 200, 1 otherwise, without starting the server. The
//! port is HTTPS and the image has no shell or curl, so Koppel asks itself.
//!
//! It asks where the UI is set to listen (`KOPPEL_LISTEN_ADDRESS` and `KOPPEL_UI_PORT`, read as the
//! server reads them), then 127.0.0.1, so a container bound to one LAN address is not unhealthy forever just
//! because nothing listens on loopback. An any-address is reached through loopback. The certificate
//! is not checked, as 0.1.x did not: Koppel's is its own, self-signed, the probe sends nothing
//! secret, and the answer only says whether Koppel is up.

use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::config::Settings;

/// How long one address gets to connect, shake hands and answer, as 0.1.x's client timeout.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Runs the health check with `settings`, prints what it found and returns the exit code.
pub fn run(settings: &Settings) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("The async runtime did not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let targets = targets(settings.listen_address(), settings.ui_port());
    match runtime.block_on(check(&targets, TIMEOUT)) {
        Ok(target) => {
            println!("Koppel answers at https://{target}/healthz");
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("{why}");
            ExitCode::FAILURE
        }
    }
}

/// Where to ask: the address the UI listens at, loopback for an any-address, then 127.0.0.1.
fn targets(address: Option<IpAddr>, port: u16) -> Vec<SocketAddr> {
    let configured = match address {
        None | Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED)) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        Some(address) => address,
    };
    let mut targets = vec![SocketAddr::new(configured, port)];
    let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    if !targets.contains(&loopback) {
        targets.push(loopback);
    }
    targets
}

/// Asks each target in turn. The first that answers decides: `Ok` with it on 200, an error on
/// anything else. One that cannot be reached passes to the next. One that takes longer than
/// `timeout` fails the check, as in 0.1.x: something took the connection and never answered.
async fn check(targets: &[SocketAddr], timeout: Duration) -> Result<SocketAddr, String> {
    let mut unreachable = Vec::new();
    for target in targets {
        let url = format!("https://{target}/healthz");
        match tokio::time::timeout(timeout, ask(*target)).await {
            Ok(Ok(200)) => return Ok(*target),
            Ok(Ok(status)) => return Err(format!("{url} answered {status}")),
            Ok(Err(e)) => unreachable.push(format!("{url}: {e}")),
            Err(_) => {
                return Err(format!(
                    "{url} did not answer within {} s",
                    timeout.as_secs_f32()
                ));
            }
        }
    }
    Err(format!(
        "Koppel does not answer. {}",
        unreachable.join("; ")
    ))
}

/// One `GET /healthz` over HTTPS and HTTP/1.1, the status from the answer's first line. The request is
/// fixed and the answer is read only as far as its status line, so there is no HTTP client in this.
async fn ask(target: SocketAddr) -> Result<u16, Box<dyn Error + Send + Sync>> {
    let tcp = TcpStream::connect(target).await?;
    let mut tls = TlsConnector::from(client_config())
        .connect(ServerName::IpAddress(target.ip().into()), tcp)
        .await?;
    let request = format!("GET /healthz HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n");
    tls.write_all(request.as_bytes()).await?;
    tls.flush().await?;
    let mut head = Vec::with_capacity(64);
    let mut chunk = [0u8; 64];
    while !head.contains(&b'\n') && head.len() < 256 {
        let read = tls.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let line = String::from_utf8_lossy(&head);
    let line = line.lines().next().unwrap_or_default();
    line.strip_prefix("HTTP/1.1 ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("answered something that is not HTTP/1.1: \"{line}\"").into())
}

fn client_config() -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// Takes any certificate, but still checks the handshake's signatures against it, so the TLS
/// connection itself is sound.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
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

#[cfg(test)]
mod tests {
    use super::*;

    use axum::Router;
    use koppel_web::CertificateSlot;
    use koppel_web::tls;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    /// Koppel's HTTPS server with `router` on a free loopback port, until the sender is dropped.
    async fn server(router: Router) -> (SocketAddr, oneshot::Sender<()>) {
        let certificate = tls::self_signed(&["localhost"]).unwrap();
        let acceptor = tls::acceptor(CertificateSlot::new(certificate.certified)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        tokio::spawn(koppel_web::serve(listener, acceptor, router, async {
            let _ = stopped.await;
        }));
        (address, stop)
    }

    /// A loopback address nothing listens on.
    async fn closed_port() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    }

    #[test]
    fn an_any_address_is_asked_through_loopback() {
        let v4 = SocketAddr::from(([127, 0, 0, 1], 5240));
        assert_eq!(targets(None, 5240), [v4]);
        assert_eq!(targets("0.0.0.0".parse().ok(), 5240), [v4]);
        assert_eq!(
            targets("::".parse().ok(), 5240),
            ["[::1]:5240".parse().unwrap(), v4]
        );
    }

    #[test]
    fn a_set_address_is_asked_first_then_loopback() {
        assert_eq!(
            targets("192.168.1.20".parse().ok(), 15240),
            [
                "192.168.1.20:15240".parse().unwrap(),
                "127.0.0.1:15240".parse().unwrap()
            ]
        );
        assert_eq!(
            targets("127.0.0.1".parse().ok(), 15240),
            ["127.0.0.1:15240".parse().unwrap()]
        );
    }

    #[tokio::test]
    async fn healthy_when_healthz_answers() {
        let (address, _stop) = server(koppel_web::router()).await;
        assert_eq!(check(&[address], TIMEOUT).await, Ok(address));
    }

    #[tokio::test]
    async fn an_address_that_does_not_answer_passes_to_the_next() {
        let (address, _stop) = server(koppel_web::router()).await;
        let closed = closed_port().await;
        assert_eq!(check(&[closed, address], TIMEOUT).await, Ok(address));
    }

    #[tokio::test]
    async fn unhealthy_when_nothing_listens() {
        let closed = closed_port().await;
        let why = check(&[closed], TIMEOUT).await.unwrap_err();
        assert!(why.contains("does not answer"), "{why}");
    }

    #[tokio::test]
    async fn unhealthy_when_healthz_is_not_200() {
        let (address, _stop) = server(Router::new()).await;
        let why = check(&[address], TIMEOUT).await.unwrap_err();
        assert!(why.contains("404"), "{why}");
    }

    #[tokio::test]
    async fn unhealthy_when_the_connection_is_taken_and_never_answered() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let (connection, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
            drop(connection);
        });
        let (healthy, _stop) = server(koppel_web::router()).await;
        let why = check(&[address, healthy], Duration::from_millis(300))
            .await
            .unwrap_err();
        assert!(why.contains("did not answer"), "{why}");
        held.abort();
    }
}

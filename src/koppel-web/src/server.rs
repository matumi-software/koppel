//! The accept loop: TLS by hand over tokio-rustls, then HTTP/1.1 or HTTP/2 by what ALPN settled on,
//! the pattern of axum's low-level rustls example and Skakel's server.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

/// How long a client has to finish the TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a client has to send the headers of a request.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// How long open connections get to finish once the server stops.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// Serves `router` over TLS on `listener` until `stop` completes, then gives open connections a
/// moment to finish. Each request carries `ConnectInfo<SocketAddr>` with the caller's address.
pub async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    router: Router,
    stop: impl Future<Output = ()> + Send,
) {
    let (stopping, stopped) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, remote)) => {
                    connections.spawn(serve_connection(
                        stream,
                        remote,
                        acceptor.clone(),
                        router.clone(),
                        stopped.clone(),
                    ));
                }
                // A connection reset before it was taken, or out of file handles for a moment.
                Err(e) => {
                    tracing::debug!("accepting a connection failed: {e}");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    drop(listener);
    let _ = stopping.send(true);
    let _ = tokio::time::timeout(STOP_GRACE, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();
}

async fn serve_connection(
    stream: TcpStream,
    remote: SocketAddr,
    acceptor: TlsAcceptor,
    router: Router,
    mut stopped: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let tls = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
        Ok(Ok(tls)) => tls,
        Ok(Err(e)) => {
            tracing::debug!("TLS handshake with {remote} failed: {e}");
            return;
        }
        Err(_) => {
            tracing::debug!("TLS handshake with {remote} timed out");
            return;
        }
    };
    let service = hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
        request.extensions_mut().insert(ConnectInfo(remote));
        let mut router = router.clone();
        async move { router.call(request).await }
    });
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);
    let connection = builder.serve_connection_with_upgrades(TokioIo::new(tls), service);
    tokio::pin!(connection);
    tokio::select! {
        _ = connection.as_mut() => {}
        _ = stopped.changed() => {
            connection.as_mut().graceful_shutdown();
            let _ = tokio::time::timeout(STOP_GRACE, connection.as_mut()).await;
        }
    }
}

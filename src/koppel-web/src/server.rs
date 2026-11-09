//! The accept loop: plain HTTP answered with a redirect, TLS by hand over tokio-rustls, then HTTP/1.1
//! or HTTP/2 by what ALPN settled on, the pattern of axum's low-level rustls example and Skakel's
//! server.

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

use crate::guard::LocalAddr;
use crate::redirect;
use crate::roles::skakel::front::FrontConnection;

/// How long a client has to send its first byte and finish the TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a client has to send the headers of a request.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// How long open connections get to finish once the server stops.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// A listener on `port` at `address`, or on every address, IPv4 and IPv6, when it is `None`, as
/// Kestrel's `ListenAnyIP` binds: one dual-stack socket, or IPv4 alone where there is no IPv6.
pub fn bind(address: Option<IpAddr>, port: u16) -> io::Result<TcpListener> {
    match address {
        Some(address) => listen(SocketAddr::new(address, port), false),
        None => listen(
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port),
            true,
        )
        .or_else(|e| {
            tracing::debug!("no dual-stack socket on port {port} ({e}), so IPv4 only");
            listen(SocketAddr::from(([0, 0, 0, 0], port)), false)
        }),
    }
}

fn listen(address: SocketAddr, dual_stack: bool) -> io::Result<TcpListener> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    if dual_stack {
        socket.set_only_v6(false)?;
    }
    // On Windows this would let another program take the port too, so only elsewhere, where it
    // lets a restart bind while old connections wait out TIME_WAIT.
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    socket.listen(1024)?;
    TcpListener::from_std(socket.into())
}

/// Serves `router` over TLS on `listener` until `stop` completes, then gives open connections a
/// moment to finish. Each request carries `ConnectInfo<SocketAddr>` with the caller's address and
/// the local address it came in on.
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
    let Ok(local) = stream.local_addr() else {
        return;
    };
    let mut first = [0u8; 1];
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(Ok(1)) if first[0] == redirect::TLS_HANDSHAKE => {}
        Ok(Ok(1)) => return redirect::answer(stream, local).await,
        _ => return,
    }
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
    // The connection to Skakel's server that this connection's Skakel requests go over, made with
    // the first one and closed with this connection.
    let skakel = Arc::new(FrontConnection::default());
    let service = hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
        request.extensions_mut().insert(ConnectInfo(remote));
        request.extensions_mut().insert(LocalAddr(local));
        request.extensions_mut().insert(skakel.clone());
        let mut router = router.clone();
        async move { router.call(request).await }
    });
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);
    // Skakel's keep-alive on every connection that may carry gRPC: a PING after 15 seconds without a
    // frame from the client, and the connection is dead when that PING has no answer 15 seconds
    // later. Adaptive windows let a USB stream's flow-control window grow past HTTP/2's 64 KiB.
    builder
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(skakel_protocol::KEEP_ALIVE_INTERVAL)
        .keep_alive_timeout(
            skakel_protocol::KEEP_ALIVE_TIMEOUT - skakel_protocol::KEEP_ALIVE_INTERVAL,
        )
        .adaptive_window(true);
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

//! The accept loop: plain HTTP answered with a redirect, TLS by hand over tokio-rustls, then HTTP/1.1
//! or HTTP/2 by what ALPN settled on, the pattern of axum's low-level rustls example and Skakel's
//! server. Skakel's calls are served on these connections too (see `roles::skakel`), so each one
//! gets Skakel's connection settings.
//!
//! At most [`MAX_CONNECTIONS`] connections are served at once per listener: the loop takes a place
//! before it accepts, so the rest wait in the system's backlog instead of in tasks and memory here.
//! When the server stops, every request carries a [`Shutdown`] that is cancelled, so a response that
//! never ends on its own (the live push) ends and the connection can close.

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tower_service::Service;

use crate::redirect;
use crate::tls::Acceptors;

/// How long a client has to send its first byte and finish the TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long open connections get to finish once the server stops.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// The most connections one listener serves at once, below the 1024 open files a service often
/// starts with, so the data plane and the config files always have a handle to spare. An HTTP/2
/// connection carries any number of requests, so this is far more than the UI and the clients of
/// one server need.
const MAX_CONNECTIONS: usize = 512;

/// On every request: cancelled when the server stops. A response that does not end by itself, the
/// live push, listens for it so it does not hold the stop up for the whole grace period.
#[derive(Debug, Clone)]
pub(crate) struct Shutdown(pub CancellationToken);

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
/// moment to finish. Each connection shakes hands with the acceptor `tls` picks for it. Each request
/// carries `ConnectInfo<SocketAddr>` with the caller's address.
pub async fn serve(
    listener: TcpListener,
    tls: impl Into<Acceptors>,
    router: Router,
    stop: impl Future<Output = ()> + Send,
) {
    serve_at_most(listener, tls.into(), router, stop, MAX_CONNECTIONS).await;
}

async fn serve_at_most(
    listener: TcpListener,
    tls: Acceptors,
    router: Router,
    stop: impl Future<Output = ()> + Send,
    most: usize,
) {
    let shutdown = CancellationToken::new();
    let places = Arc::new(Semaphore::new(most));
    let mut connections = JoinSet::new();
    let mut full = false;
    tokio::pin!(stop);
    loop {
        // A place first, then the connection: at the cap the next one waits in the backlog.
        if places.available_permits() == 0 && !full {
            full = true;
            tracing::warn!(
                "{most} connections are open on this port; the next ones wait until one closes"
            );
        }
        let place = tokio::select! {
            () = &mut stop => break,
            Some(done) = connections.join_next(), if !connections.is_empty() => {
                drop(done);
                continue;
            }
            place = places.clone().acquire_owned() => match place {
                Ok(place) => place,
                Err(_) => break,
            },
        };
        full = false;
        tokio::select! {
            () = &mut stop => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, remote)) => {
                    connections.spawn(serve_connection(
                        stream,
                        remote,
                        tls.clone(),
                        router.clone(),
                        shutdown.clone(),
                        place,
                    ));
                }
                // A connection reset before it was taken, or out of file handles for a moment.
                Err(e) => {
                    tracing::debug!("accepting a connection failed: {e}");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            },
        }
    }
    drop(listener);
    shutdown.cancel();
    let _ = tokio::time::timeout(STOP_GRACE, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();
}

/// The TLS stream of a connection, or `None` when it is not one to serve: plain HTTP (answered with
/// a redirect), a silent or failed handshake.
async fn handshake(
    stream: TcpStream,
    remote: SocketAddr,
    tls: Acceptors,
) -> Option<tokio_rustls::server::TlsStream<TcpStream>> {
    let local = stream.local_addr().ok()?;
    let mut first = [0u8; 1];
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(Ok(1)) if first[0] == redirect::TLS_HANDSHAKE => {}
        Ok(Ok(1)) => {
            redirect::answer(stream, local).await;
            return None;
        }
        _ => return None,
    }
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, tls.pick().accept(stream)).await {
        Ok(Ok(tls)) => Some(tls),
        Ok(Err(e)) => {
            tracing::debug!("TLS handshake with {remote} failed: {e}");
            None
        }
        Err(_) => {
            tracing::debug!("TLS handshake with {remote} timed out");
            None
        }
    }
}

async fn serve_connection(
    stream: TcpStream,
    remote: SocketAddr,
    tls: Acceptors,
    router: Router,
    shutdown: CancellationToken,
    // Held until the connection is over: its place at the cap.
    _place: OwnedSemaphorePermit,
) {
    let _ = stream.set_nodelay(true);
    // A connection still shaking hands has nothing to finish: a stop drops it.
    let tls = tokio::select! {
        () = shutdown.cancelled() => return,
        tls = handshake(stream, remote, tls) => match tls {
            Some(tls) => tls,
            None => return,
        },
    };
    let stopping = Shutdown(shutdown.clone());
    let service = hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
        request.extensions_mut().insert(ConnectInfo(remote));
        request.extensions_mut().insert(stopping.clone());
        let mut router = router.clone();
        async move { router.call(request).await }
    });
    // Skakel's settings on every connection, since any may carry its calls: HTTP/2 keep-alive (a
    // PING after 15 seconds without a frame from the client, dead when it has no answer 15 seconds
    // later), adaptive windows so a USB stream's window grows past HTTP/2's 64 KiB, and 30 seconds
    // for an HTTP/1.1 request's headers.
    let builder = skakel_server::connection_builder();
    let connection = builder.serve_connection_with_upgrades(TokioIo::new(tls), service);
    tokio::pin!(connection);
    tokio::select! {
        _ = connection.as_mut() => {}
        () = shutdown.cancelled() => {
            connection.as_mut().graceful_shutdown();
            let _ = tokio::time::timeout(STOP_GRACE, connection.as_mut()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::tls::{self, CertificateSlot};

    fn acceptors() -> Acceptors {
        let slot = CertificateSlot::new(tls::self_signed(&["localhost"]).unwrap().certified);
        tls::acceptor(slot).unwrap().into()
    }

    /// A plain HTTP request, which the server answers with a redirect and closes: a way to tell
    /// whether the loop has taken the connection, with no TLS in the way.
    async fn plain_request(port: u16) -> TcpStream {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        stream
    }

    async fn answered(stream: &mut TcpStream, within: Duration) -> bool {
        let mut head = [0u8; 12];
        tokio::time::timeout(within, stream.read_exact(&mut head))
            .await
            .is_ok_and(|read| read.is_ok() && head.starts_with(b"HTTP/1.1 301"))
    }

    #[tokio::test]
    async fn at_the_cap_the_next_connection_waits_for_a_place_and_gets_it_when_one_closes() {
        let listener = bind(Some("127.0.0.1".parse().unwrap()), 0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let serving = tokio::spawn(serve_at_most(
            listener,
            acceptors(),
            Router::new(),
            async move {
                let _ = stopped.await;
            },
            2,
        ));

        // Two connections that say nothing hold the two places.
        let first = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let second = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut third = plain_request(port).await;
        assert!(
            !answered(&mut third, Duration::from_millis(400)).await,
            "the third waits: both places are taken"
        );

        drop(first);
        assert!(
            answered(&mut third, Duration::from_secs(5)).await,
            "a place came free and the third was served"
        );

        // And a stop does not wait for the connection that holds the other place for ever.
        let began = Instant::now();
        let _ = stop.send(());
        serving.await.unwrap();
        assert!(began.elapsed() < Duration::from_secs(2));
        drop(second);
    }

    #[tokio::test]
    async fn a_stop_while_the_places_are_full_still_stops() {
        let listener = bind(Some("127.0.0.1".parse().unwrap()), 0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let serving = tokio::spawn(serve_at_most(
            listener,
            acceptors(),
            Router::new(),
            async move {
                let _ = stopped.await;
            },
            1,
        ));
        let _held = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let _ = stop.send(());
        let stopped = tokio::time::timeout(Duration::from_secs(2), serving).await;
        assert!(
            stopped.is_ok(),
            "the loop stopped with its only place taken"
        );
    }
}

use std::io;
use std::net::{Ipv4Addr, TcpStream as StdTcpStream};
#[cfg(feature = "test-util")]
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// How long [`loopback_pair`] waits for its own connection to arrive.
const PAIR_TIMEOUT: Duration = Duration::from_secs(5);

/// The most one read takes before it is passed on, as .NET's `CopyToAsync` reads.
const RELAY_BUFFER: usize = 81920;

/// Copies both directions until either side closes or `stop` is cancelled, then returns. One
/// side closing cuts the other copy off: that is the normal end of a session.
pub async fn relay<A, B>(a: A, b: B, stop: &CancellationToken)
where
    A: AsyncRead + AsyncWrite + Unpin + Send,
    B: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut a_read, mut a_write) = tokio::io::split(a);
    let (mut b_read, mut b_write) = tokio::io::split(b);
    tokio::select! {
        _ = pass_on(&mut a_read, &mut b_write) => {}
        _ = pass_on(&mut b_read, &mut a_write) => {}
        _ = stop.cancelled() => {}
    }
}

/// Copies one direction until the reader ends. Each read is written and flushed at once, so a
/// stream that frames its writes sends one message per read, as 0.1.x's WebSocket stream did.
async fn pass_on<R, W>(reader: &mut R, writer: &mut W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; RELAY_BUFFER];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..read]).await?;
        writer.flush().await?;
    }
}

/// What a test attacher gets instead of a socket: one end of a pipe in memory, so an attach opens no
/// socket and the test can run on tokio's paused clock.
#[cfg(feature = "test-util")]
const MEMORY_PIPE: usize = 256 * 1024;

/// The kernel's end of an attach: the socket the virtual host controller is handed, or in a test
/// one end of an in-memory pipe (taken by the attacher when it attaches).
pub(crate) enum KernelEnd {
    Socket(StdTcpStream),
    #[cfg(feature = "test-util")]
    Memory(Mutex<Option<tokio::io::DuplexStream>>),
}

#[cfg(feature = "test-util")]
impl KernelEnd {
    /// The pipe's end, once.
    pub(crate) fn take_memory(&self) -> Option<tokio::io::DuplexStream> {
        match self {
            Self::Memory(end) => end.lock().unwrap_or_else(PoisonError::into_inner).take(),
            Self::Socket(_) => None,
        }
    }
}

/// The relay's end of an attach: a socket, or in a test the other end of the pipe.
pub(crate) trait RelayEnd: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> RelayEnd for T {}

/// The two ends of an attach: the kernel's and the relay's. `memory` is for a test attacher that
/// takes a pipe in memory ([`crate::UsbAttacher::in_memory`]); everything else gets
/// [`loopback_pair`].
pub(crate) async fn kernel_pair(memory: bool) -> io::Result<(KernelEnd, Box<dyn RelayEnd>)> {
    #[cfg(feature = "test-util")]
    if memory {
        let (kernel, relay) = tokio::io::duplex(MEMORY_PIPE);
        return Ok((KernelEnd::Memory(Mutex::new(Some(kernel))), Box::new(relay)));
    }
    #[cfg(not(feature = "test-util"))]
    let _ = memory;
    let (kernel_end, relay_end) = loopback_pair().await?;
    Ok((KernelEnd::Socket(kernel_end), Box::new(relay_end)))
}

/// Two connected loopback TCP sockets. The kernel gets one end as a plain socket; Koppel relays
/// the other end through the stream to the other machine. That is how USB/IP traffic stays
/// encrypted on the network.
///
/// Another process on the machine may connect to the listener first, so a connection counts only
/// when it comes from the kernel end's own address; any other is closed and the wait goes on.
/// The kernel end is in blocking mode, as the `usbip` tools hand it over.
pub async fn loopback_pair() -> io::Result<(StdTcpStream, TcpStream)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let pair = tokio::time::timeout(PAIR_TIMEOUT, async {
        let kernel_end = TcpStream::connect(address).await?;
        kernel_end.set_nodelay(true)?;
        let own = kernel_end.local_addr()?;
        loop {
            let (candidate, from) = listener.accept().await?;
            if from == own {
                candidate.set_nodelay(true)?;
                return Ok::<_, io::Error>((kernel_end, candidate));
            }
        }
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "The loopback connection for the kernel never arrived.",
        )
    })??;
    let (kernel_end, relay_end) = pair;
    let kernel_end = kernel_end.into_std()?;
    kernel_end.set_nonblocking(false)?;
    Ok((kernel_end, relay_end))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    use super::*;
    use crate::stream::memory;

    #[tokio::test]
    async fn the_pair_is_connected_end_to_end() {
        let (mut kernel_end, mut relay_end) = loopback_pair().await.unwrap();

        relay_end.write_all(b"urb").await.unwrap();
        let mut read = [0u8; 3];
        let kernel_end = tokio::task::spawn_blocking(move || {
            kernel_end.read_exact(&mut read).unwrap();
            kernel_end.write_all(b"ack").unwrap();
            (kernel_end, read)
        })
        .await
        .unwrap();
        let mut back = [0u8; 3];
        relay_end.read_exact(&mut back).await.unwrap();

        assert_eq!(&kernel_end.1, b"urb");
        assert_eq!(&back, b"ack");
    }

    /// A destination that notes how each write and flush arrives, and says nothing back.
    struct Notes {
        writes: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
        flushes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AsyncRead for Notes {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    impl AsyncWrite for Notes {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            self.writes.lock().unwrap().push(buf.len());
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            self.flushes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// The stream to the other machine frames its writes, so the relay must write what one read
    /// returned as one write and flush it at once: one message per read, as 0.1.x's WebSocket
    /// stream sent. `tokio::io::copy_bidirectional` reads into 8 KiB and flushes only when the
    /// reader would block, so it changes that, which is why the relay is not swapped for it (see
    /// docs/decisions.md).
    #[tokio::test]
    async fn one_read_is_one_write_and_one_flush() {
        let (mut source, source_inner) = memory::pair();
        let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let flushes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let notes = Notes {
            writes: writes.clone(),
            flushes: flushes.clone(),
        };
        let stop = CancellationToken::new();
        let ended = stop.clone();
        let running = tokio::spawn(async move { relay(source_inner, notes, &ended).await });

        let mut total = 0;
        for size in [5usize, 4096, 3] {
            source.write_all(&vec![7u8; size]).await.unwrap();
            total += size;
            let written = writes.clone();
            let until = tokio::time::Instant::now() + Duration::from_secs(5);
            while written.lock().unwrap().iter().sum::<usize>() < total {
                assert!(tokio::time::Instant::now() < until, "the write arrives");
                tokio::task::yield_now().await;
            }
        }
        stop.cancel();
        running.await.unwrap();

        assert_eq!(*writes.lock().unwrap(), vec![5, 4096, 3]);
        assert_eq!(flushes.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn the_relay_carries_both_ways_and_ends_when_one_side_closes() {
        let (mut left, left_inner) = memory::pair();
        let (right_inner, mut right) = memory::pair();
        let stop = CancellationToken::new();
        let running = tokio::spawn(async move { relay(left_inner, right_inner, &stop).await });

        left.write_all(b"out").await.unwrap();
        let mut read = [0u8; 3];
        right.read_exact(&mut read).await.unwrap();
        right.write_all(b"in!").await.unwrap();
        let mut back = [0u8; 3];
        left.read_exact(&mut back).await.unwrap();
        drop(left);

        tokio::time::timeout(Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&read, b"out");
        assert_eq!(&back, b"in!");
    }
}

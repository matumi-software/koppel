use std::io;
use std::net::{Ipv4Addr, TcpStream as StdTcpStream};
#[cfg(feature = "test-util")]
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};

/// How long [`loopback_pair`] waits for its own connection to arrive.
const PAIR_TIMEOUT: Duration = Duration::from_secs(5);

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

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

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
}

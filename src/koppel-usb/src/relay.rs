use std::io;
use std::net::{Ipv4Addr, TcpStream as StdTcpStream};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// How long [`loopback_pair`] waits for its own connection to arrive.
const PAIR_TIMEOUT: Duration = Duration::from_secs(5);

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
        _ = tokio::io::copy(&mut a_read, &mut b_write) => {}
        _ = tokio::io::copy(&mut b_read, &mut a_write) => {}
        _ = stop.cancelled() => {}
    }
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

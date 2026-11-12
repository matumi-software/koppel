use std::future::Future;
use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The most one read takes before it is passed on, as .NET's `CopyToAsync` reads.
const RELAY_BUFFER: usize = 81920;

/// Copies both directions until either side closes or `stop` is ready, then returns. One side
/// closing cuts the other copy off: that is the normal end of a session.
pub async fn relay<A, B>(a: A, b: B, stop: impl Future<Output = ()>)
where
    A: AsyncRead + AsyncWrite + Unpin + Send,
    B: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut a_read, mut a_write) = tokio::io::split(a);
    let (mut b_read, mut b_write) = tokio::io::split(b);
    tokio::select! {
        _ = pass_on(&mut a_read, &mut b_write) => {}
        _ = pass_on(&mut b_read, &mut a_write) => {}
        () = stop => {}
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::*;

    /// A destination that notes how each write and flush arrives, and says nothing back.
    struct Notes {
        writes: Arc<Mutex<Vec<usize>>>,
        flushes: Arc<AtomicUsize>,
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
            self.flushes.fetch_add(1, Ordering::SeqCst);
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
        let (mut source, source_inner) = tokio::io::duplex(64 * 1024);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let flushes = Arc::new(AtomicUsize::new(0));
        let notes = Notes {
            writes: writes.clone(),
            flushes: flushes.clone(),
        };
        let stop = Arc::new(Notify::new());
        let ended = stop.clone();
        let running = tokio::spawn(async move {
            relay(source_inner, notes, async move { ended.notified().await }).await;
        });

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
        stop.notify_one();
        running.await.unwrap();

        assert_eq!(*writes.lock().unwrap(), vec![5, 4096, 3]);
        assert_eq!(flushes.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn the_relay_carries_both_ways_and_ends_when_one_side_closes() {
        let (mut left, left_inner) = tokio::io::duplex(64 * 1024);
        let (right_inner, mut right) = tokio::io::duplex(64 * 1024);
        let running = tokio::spawn(async move {
            relay(left_inner, right_inner, std::future::pending()).await;
        });

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

    #[tokio::test]
    async fn the_relay_ends_when_stop_is_ready() {
        let (_left, left_inner) = tokio::io::duplex(1024);
        let (right_inner, _right) = tokio::io::duplex(1024);
        tokio::time::timeout(
            Duration::from_secs(5),
            relay(left_inner, right_inner, std::future::ready(())),
        )
        .await
        .unwrap();
    }
}

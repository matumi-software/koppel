//! The byte stream to the other machine for one device, and how the client opens one.
//!
//! Koppel's transport, Skakel, carries USB/IP between machines: each attached device has a stream
//! of its own, encrypted and behind the client's token. This crate does not know Skakel. It asks
//! for a [`DeviceStream`], reads and writes USB/IP on it, relays the device's traffic through it,
//! and closes it with a [`CloseReason`]. The server role is handed streams by the transport
//! ([`crate::UsbDataPlane::serve`]); the client role opens them through a [`StreamOpener`], which
//! the link to the server provides ([`crate::Link`]).
//!
//! [`memory::pair`] connects two ends in memory, for tests on either side.

use std::future::Future;
use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite};

/// A future that can be held as a trait object and sent between threads.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Why a stream to the other machine ended, for the other side's log and UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// The session is over: the device was released, detached, or the other end hung up.
    Done,
    /// This machine no longer serves USB (its role left the server side) or Koppel is stopping.
    /// Skakel's "stopping" close.
    Stopping,
    /// Something went wrong. The text is for the other machine's log.
    Failed(String),
}

/// The byte stream to the other machine for one device. Reading and writing are tokio's
/// [`AsyncRead`] and [`AsyncWrite`], so the relay copies without a buffer of its own per call.
/// A read that returns 0 bytes means the other end closed it.
///
/// Close a stream with [`DeviceStream::close`] when the session ends. Dropping one without
/// closing it ends it too, as a failure.
pub trait DeviceStream: AsyncRead + AsyncWrite + Unpin + Send {
    /// Ends the stream and tells the other machine why.
    fn close(self: Box<Self>, reason: CloseReason) -> BoxFuture<'static, ()>;
}

/// Why a stream could not be opened. Both texts are for a person: the client shows them on the
/// device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The server answered and said no, such as a client it no longer knows.
    Refused(String),
    /// The server could not be reached, its certificate did not match, or the like, in Skakel's
    /// fixed wording.
    Failed(String),
}

impl OpenError {
    /// The sentence the client shows on the device.
    pub fn describe(&self) -> String {
        match self {
            Self::Refused(why) => format!("The server refused the USB connection: {why}"),
            Self::Failed(why) => why.clone(),
        }
    }
}

/// Opens a stream to the server for one device. The client sets the deadline itself and drops
/// the future when it runs out, so an implementation needs no time limit of its own.
pub trait StreamOpener: Send + Sync {
    fn open(&self) -> BoxFuture<'_, Result<Box<dyn DeviceStream>, OpenError>>;
}

/// Two stream ends joined in memory, for tests.
#[cfg(any(test, feature = "test-util"))]
pub mod memory {
    use std::io;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::task::{Context, Poll};

    use tokio::io::{DuplexStream, ReadBuf};
    use tokio::sync::Notify;

    use super::*;

    const BUFFER: usize = 64 * 1024;

    /// One end of a [`pair`].
    pub struct MemoryStream {
        io: DuplexStream,
        /// What this end closed with, for the other end to read.
        mine: Arc<Closed>,
        /// What the other end closed with.
        theirs: Arc<Closed>,
    }

    #[derive(Default)]
    struct Closed {
        reason: Mutex<Option<CloseReason>>,
        notify: Notify,
    }

    /// Lets a test learn how the other end closed after it handed its end away.
    #[derive(Clone)]
    pub struct CloseWatch(Arc<Closed>);

    impl CloseWatch {
        /// How the other end closed, if it has.
        pub fn reason(&self) -> Option<CloseReason> {
            self.0
                .reason
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// Waits until the other end closes, and says how.
        pub async fn closed(&self) -> CloseReason {
            loop {
                let notified = self.0.notify.notified();
                if let Some(reason) = self.reason() {
                    return reason;
                }
                notified.await;
            }
        }
    }

    /// Two ends: what one writes the other reads.
    pub fn pair() -> (MemoryStream, MemoryStream) {
        let (a, b) = tokio::io::duplex(BUFFER);
        let a_closed = Arc::new(Closed::default());
        let b_closed = Arc::new(Closed::default());
        (
            MemoryStream {
                io: a,
                mine: a_closed.clone(),
                theirs: b_closed.clone(),
            },
            MemoryStream {
                io: b,
                mine: b_closed,
                theirs: a_closed,
            },
        )
    }

    impl MemoryStream {
        /// Watches for the other end's close.
        pub fn peer_close(&self) -> CloseWatch {
            CloseWatch(self.theirs.clone())
        }
    }

    impl Drop for MemoryStream {
        fn drop(&mut self) {
            let mut reason = self
                .mine
                .reason
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if reason.is_none() {
                *reason = Some(CloseReason::Failed("dropped without closing".into()));
            }
            drop(reason);
            self.mine.notify.notify_waiters();
        }
    }

    impl DeviceStream for MemoryStream {
        fn close(self: Box<Self>, reason: CloseReason) -> BoxFuture<'static, ()> {
            *self
                .mine
                .reason
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(reason);
            self.mine.notify.notify_waiters();
            Box::pin(async move { drop(self) })
        }
    }

    impl AsyncRead for MemoryStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.io).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for MemoryStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.io).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.io).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.io).poll_shutdown(cx)
        }
    }

    #[cfg(test)]
    mod tests {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        use super::*;

        #[tokio::test]
        async fn bytes_cross_and_the_other_end_learns_the_reason() {
            let (mut a, mut b) = pair();
            let watch = b.peer_close();

            a.write_all(b"ping").await.unwrap();
            let mut read = [0u8; 4];
            b.read_exact(&mut read).await.unwrap();
            Box::new(a).close(CloseReason::Stopping).await;

            assert_eq!(&read, b"ping");
            assert_eq!(b.read(&mut read).await.unwrap(), 0);
            assert_eq!(watch.closed().await, CloseReason::Stopping);
        }
    }
}

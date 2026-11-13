//! Koppel's `usb` stream type on Skakel's streams, as koppel-usb's [`DeviceStream`]: the server's
//! end of an accepted stream and the client's end of one it opened. The stream carries USB/IP in
//! its data frames, as 0.1.x's carried it in WebSocket binary frames: a device list, or an import
//! followed by the device's traffic. It takes no target and has no control messages.
//!
//! Each end is a byte pipe in memory with a task that moves the bytes to and from the stream's
//! frames, so the USB layer reads and writes with tokio's traits and needs nothing of Skakel. A
//! frame is `Bytes` from the pipe to Skakel and from Skakel to the pipe, and the only copy left is
//! the one into and out of the pipe itself.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use koppel_usb::{BoxFuture, CloseReason, DeviceStream, OpenError, RemoteClient, UsbDataPlane};
use skakel_client::{Frame, Link, OpenResult, StreamConnection};
use skakel_server::{
    StreamEnd, StreamHandler, StreamMessage, StreamOpen, StreamRequest, StreamSession, StreamWriter,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::tasks::Tasks;

/// Koppel's stream type for one USB device.
pub const USB_STREAM: &str = "usb";

/// The most bytes one data frame carries.
const CHUNK: usize = 64 * 1024;

/// What the pipe holds between the USB layer and the stream, each way.
const PIPE: usize = 256 * 1024;

/// How long closing waits for the bytes written before it to leave.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// The reason a stream type of Koppel's own ends with when the USB layer failed.
const FAILED: &str = "FAILED";

/// Opens `usb` streams on the server and hands each to the data plane, with the client Skakel
/// checked.
pub fn usb_handler(data_plane: UsbDataPlane, tasks: Tasks) -> impl StreamHandler {
    move |request: StreamRequest| {
        let data_plane = data_plane.clone();
        let tasks = tasks.clone();
        async move {
            if !request.target.is_empty() {
                return StreamOpen::BadTarget("A usb stream takes no target.".into());
            }
            StreamOpen::accept(move |session: StreamSession| async move {
                let client = RemoteClient {
                    id: session.client_id.clone(),
                    name: session.client_name.clone(),
                };
                let stream = Bridged::start(ServerEnd::new(session), &tasks);
                data_plane.serve(Box::new(stream), client).await;
            })
        }
    }
}

/// Opens a `usb` stream on `link`, on a connection of its own, as koppel-usb's client asks.
pub async fn open_usb(link: &Link, tasks: &Tasks) -> Result<Box<dyn DeviceStream>, OpenError> {
    match link.open_stream(USB_STREAM, &[]).await {
        Ok(OpenResult::Opened(stream)) => {
            Ok(Box::new(Bridged::start(ClientEnd(Arc::new(stream)), tasks)))
        }
        Ok(OpenResult::Refused(refusal)) => Err(OpenError::Refused(refusal.message)),
        Err(error) => Err(OpenError::Failed(error.failure().message().to_owned())),
    }
}

/// One end of a stream, as frames.
trait Frames: Send + Sync + 'static {
    /// Sends data. False once the stream is over.
    fn send(&self, data: Bytes) -> BoxFuture<'_, bool>;
    /// The next data, or `None` once the other end closed or the stream is over.
    fn next(&self) -> BoxFuture<'_, Option<Bytes>>;
    /// Ends the stream, after what was sent before.
    fn finish(&self, reason: CloseReason) -> BoxFuture<'_, ()>;
}

struct ServerEnd {
    session: Mutex<StreamSession>,
    writer: StreamWriter,
}

impl ServerEnd {
    fn new(session: StreamSession) -> Self {
        Self {
            writer: session.writer(),
            session: Mutex::new(session),
        }
    }
}

impl Frames for ServerEnd {
    fn send(&self, data: Bytes) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.writer.send_data(data).await.is_ok() })
    }

    fn next(&self) -> BoxFuture<'_, Option<Bytes>> {
        Box::pin(async move {
            let mut session = self.session.lock().await;
            loop {
                match session.recv().await? {
                    StreamMessage::Data(data) => return Some(data),
                    StreamMessage::Control(_) => {}
                }
            }
        })
    }

    fn finish(&self, reason: CloseReason) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            match reason {
                // Stopping ends with STOPPING when the server itself stops; a role change restarts
                // Skakel's server, which ends every stream that way.
                CloseReason::Done | CloseReason::Stopping => self.writer.close().await,
                CloseReason::Failed(why) => match StreamEnd::aborted(FAILED, why) {
                    Ok(end) => self.writer.end(end).await,
                    Err(_) => self.writer.close().await,
                },
            }
        })
    }
}

struct ClientEnd(Arc<StreamConnection>);

impl Frames for ClientEnd {
    fn send(&self, data: Bytes) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.0.send_data(data).await.is_ok() })
    }

    fn next(&self) -> BoxFuture<'_, Option<Bytes>> {
        Box::pin(async move {
            loop {
                // Koppel's stream has no control messages, and a frame a later Skakel adds is not
                // for it either.
                if let Frame::Data(data) = self.0.recv().await? {
                    return Some(data);
                }
            }
        })
    }

    fn finish(&self, _reason: CloseReason) -> BoxFuture<'_, ()> {
        // The client ends its side of the call; the server sends what it still has and ends with OK.
        Box::pin(async move { self.0.close().await })
    }
}

/// A [`DeviceStream`] over a stream's frames.
struct Bridged {
    io: Option<DuplexStream>,
    close: Option<oneshot::Sender<CloseReason>>,
    pump: Option<JoinHandle<()>>,
}

impl Bridged {
    fn start(frames: impl Frames, tasks: &Tasks) -> Self {
        let (ours, theirs) = tokio::io::duplex(PIPE);
        let (close, reason) = oneshot::channel();
        let pump = tasks.spawn(pump(Arc::new(frames), theirs, reason, tasks.clone()));
        Self {
            io: Some(ours),
            close: Some(close),
            pump: Some(pump),
        }
    }

    fn io(&mut self) -> &mut DuplexStream {
        self.io
            .as_mut()
            .expect("the pipe stays until the stream is closed")
    }
}

/// Moves bytes between the pipe and the frames until both ways are done, then ends the stream with
/// the reason the USB layer closed it with.
async fn pump(
    frames: Arc<dyn Frames>,
    pipe: DuplexStream,
    reason: oneshot::Receiver<CloseReason>,
    tasks: Tasks,
) {
    let (mut from_usb, mut to_usb) = tokio::io::split(pipe);
    let incoming = {
        let frames = frames.clone();
        tasks.spawn(async move {
            while let Some(data) = frames.next().await {
                if to_usb.write_all(&data).await.is_err() {
                    return;
                }
            }
            // The other end closed: the USB layer reads the end.
            let _ = to_usb.shutdown().await;
        })
    };
    // Each frame is a slice of this buffer handed on as it is: nothing is copied between the read
    // and the send. `reserve` starts a new allocation only when the frames sent so far, which share
    // the old one, leave less than a chunk of room, so small reads (USB/IP headers) share one too.
    let mut buffer = BytesMut::new();
    loop {
        buffer.reserve(CHUNK);
        match (&mut from_usb)
            .take(CHUNK as u64)
            .read_buf(&mut buffer)
            .await
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if !frames.send(buffer.split().freeze()).await {
                    break;
                }
            }
        }
    }
    let reason = reason
        .await
        .unwrap_or_else(|_| CloseReason::Failed("dropped without closing".into()));
    frames.finish(reason).await;
    incoming.abort();
}

impl DeviceStream for Bridged {
    fn close(mut self: Box<Self>, reason: CloseReason) -> BoxFuture<'static, ()> {
        if let Some(close) = self.close.take() {
            let _ = close.send(reason);
        }
        // The end of the pipe is how the pump learns the USB layer is done writing.
        self.io.take();
        let pump = self.pump.take();
        Box::pin(async move {
            if let Some(pump) = pump {
                let _ = tokio::time::timeout(CLOSE_WAIT, pump).await;
            }
        })
    }
}

impl AsyncRead for Bridged {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.io()).poll_read(cx, buf)
    }
}

impl AsyncWrite for Bridged {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.io()).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.io()).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.io()).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Two ends of a stream joined by channels, each end's `send` arriving at the other's `next`.
    struct Wire {
        out: mpsc::UnboundedSender<Bytes>,
        incoming: Mutex<mpsc::UnboundedReceiver<Bytes>>,
    }

    fn wire() -> (Wire, Wire) {
        let (a_out, b_in) = mpsc::unbounded_channel();
        let (b_out, a_in) = mpsc::unbounded_channel();
        (
            Wire {
                out: a_out,
                incoming: Mutex::new(a_in),
            },
            Wire {
                out: b_out,
                incoming: Mutex::new(b_in),
            },
        )
    }

    impl Frames for Wire {
        fn send(&self, data: Bytes) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                assert!(data.len() <= CHUNK, "a frame of {} bytes", data.len());
                self.out.send(data).is_ok()
            })
        }

        fn next(&self) -> BoxFuture<'_, Option<Bytes>> {
            Box::pin(async move { self.incoming.lock().await.recv().await })
        }

        fn finish(&self, _reason: CloseReason) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }

    /// Bytes written at one end come out of the other unchanged and in order, whether they were
    /// written a few at a time or many chunks at once.
    #[tokio::test]
    async fn bytes_cross_a_stream_intact_in_frames_no_larger_than_a_chunk() {
        let tasks = Tasks::new();
        let (left, right) = wire();
        let mut a = Bridged::start(left, &tasks);
        let mut b = Bridged::start(right, &tasks);

        let mut sent = Vec::new();
        for size in [
            1usize,
            48,
            300,
            CHUNK - 1,
            CHUNK,
            CHUNK + 1,
            3 * CHUNK + 7,
            5,
        ] {
            let piece: Vec<u8> = (0..size).map(|i| (i * 31 + size) as u8).collect();
            sent.extend_from_slice(&piece);
            a.write_all(&piece).await.unwrap();
        }
        a.flush().await.unwrap();

        let mut got = vec![0u8; sent.len()];
        b.read_exact(&mut got).await.unwrap();
        assert!(
            got == sent,
            "the bytes that came out differ from the bytes that went in"
        );

        // And the other way, then the close reaches the far end as the end of the stream.
        b.write_all(b"back again").await.unwrap();
        let mut back = [0u8; 10];
        a.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"back again");
        Box::new(a).close(CloseReason::Done).await;
        let mut rest = Vec::new();
        b.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        Box::new(b).close(CloseReason::Done).await;
    }
}

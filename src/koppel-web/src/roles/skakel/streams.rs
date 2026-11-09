//! Koppel's `usb` stream type on Skakel's streams, as koppel-usb's [`DeviceStream`]: the server's
//! end of an accepted stream and the client's end of one it opened. The stream carries USB/IP in
//! its data frames, as 0.1.x's carried it in WebSocket binary frames: a device list, or an import
//! followed by the device's traffic. It takes no target and has no control messages.
//!
//! Each end is a byte pipe in memory with a task that moves the bytes to and from the stream's
//! frames, so the USB layer reads and writes with tokio's traits and needs nothing of Skakel.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use koppel_usb::{BoxFuture, CloseReason, DeviceStream, OpenError, RemoteClient, UsbDataPlane};
use skakel_client::{Frame, Link, OpenResult, StreamConnection};
use skakel_server::{
    StreamEnd, StreamHandler, StreamMessage, StreamOpen, StreamRequest, StreamSession, StreamWriter,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

/// Koppel's stream type for one USB device.
pub(crate) const USB_STREAM: &str = "usb";

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
pub(crate) fn usb_handler(data_plane: UsbDataPlane) -> StreamHandler {
    Arc::new(move |request: StreamRequest| {
        let data_plane = data_plane.clone();
        Box::pin(async move {
            if !request.target.is_empty() {
                return StreamOpen::BadTarget("A usb stream takes no target.".into());
            }
            StreamOpen::accept(move |session: StreamSession| async move {
                let client = RemoteClient {
                    id: session.client_id.clone(),
                    name: session.client_name.clone(),
                };
                let stream = Bridged::start(ServerEnd::new(session));
                data_plane.serve(Box::new(stream), client).await;
            })
        })
    })
}

/// Opens a `usb` stream on `link`, on a connection of its own, as koppel-usb's client asks.
pub(crate) async fn open_usb(link: &Link) -> Result<Box<dyn DeviceStream>, OpenError> {
    match link.open_stream(USB_STREAM, &[]).await {
        Ok(OpenResult::Opened(stream)) => Ok(Box::new(Bridged::start(ClientEnd(Arc::new(stream))))),
        Ok(OpenResult::Refused(refusal)) => Err(OpenError::Refused(refusal.message)),
        Err(error) => Err(OpenError::Failed(error.failure().message().to_owned())),
    }
}

/// One end of a stream, as frames.
trait Frames: Send + Sync + 'static {
    /// Sends data. False once the stream is over.
    fn send(&self, data: Vec<u8>) -> BoxFuture<'_, bool>;
    /// The next data, or `None` once the other end closed or the stream is over.
    fn next(&self) -> BoxFuture<'_, Option<Vec<u8>>>;
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
    fn send(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.writer.send_data(data).await.is_ok() })
    }

    fn next(&self) -> BoxFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move {
            let mut session = self.session.lock().await;
            loop {
                match session.recv().await? {
                    StreamMessage::Data(data) => return Some(data.to_vec()),
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
    fn send(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.0.send_data(data).await.is_ok() })
    }

    fn next(&self) -> BoxFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move {
            loop {
                match self.0.recv().await? {
                    Frame::Data(data) => return Some(data.to_vec()),
                    Frame::Control(_) => {}
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
    fn start(frames: impl Frames) -> Self {
        let (ours, theirs) = tokio::io::duplex(PIPE);
        let (close, reason) = oneshot::channel();
        let pump = tokio::spawn(pump(Arc::new(frames), theirs, reason));
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
async fn pump(frames: Arc<dyn Frames>, pipe: DuplexStream, reason: oneshot::Receiver<CloseReason>) {
    let (mut from_usb, mut to_usb) = tokio::io::split(pipe);
    let incoming = {
        let frames = frames.clone();
        tokio::spawn(async move {
            while let Some(data) = frames.next().await {
                if to_usb.write_all(&data).await.is_err() {
                    return;
                }
            }
            // The other end closed: the USB layer reads the end.
            let _ = to_usb.shutdown().await;
        })
    };
    let mut buffer = vec![0u8; CHUNK];
    loop {
        match from_usb.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if !frames.send(buffer[..read].to_vec()).await {
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

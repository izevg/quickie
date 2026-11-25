//! Bounded NDJSON transport for `rmcp` server sessions.
//!
//! Enforces a strict [`MAX_FRAME_BYTES`] (256 KiB) limit per incoming serialized line including its
//! terminating newline, bounded read accumulation before JSON deserialization, and immediate
//! connection termination on overlong or malformed frames so one peer cannot affect another.

use std::io;
use std::sync::Arc;

use rmcp::service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, watch};

/// Reason the transport's read side terminated, used by [`crate::serve_stream`] to bound
/// half-close draining and cancel parked handlers promptly on peer disconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportClose {
    /// Clean peer EOF on the read stream (may still have a brief half-close response drain).
    Eof,
    /// Protocol or I/O failure on the read stream (cancel immediately).
    Failed,
}

/// Transport-owned handle that cancels any queued or in-flight writes immediately.
#[derive(Clone, Debug)]
pub(crate) struct WriteCancelHandle {
    tx: Arc<watch::Sender<bool>>,
}

impl WriteCancelHandle {
    pub(crate) fn cancel(&self) {
        self.tx.send_if_modified(|cancelled| {
            if *cancelled {
                false
            } else {
                *cancelled = true;
                true
            }
        });
    }
}

/// Maximum serialized size of a single incoming NDJSON frame, including its terminating `\n`.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Initial read buffer capacity per connection so idle or stalled peers use minimal memory.
const INITIAL_READ_BUF_CAPACITY: usize = 8 * 1024;

/// Cancellation-safe, strictly bounded NDJSON [`Transport`] for `rmcp` server connections.
///
/// Unlike `rmcp::transport::async_rw::AsyncRwTransport`, this transport disconnects immediately
/// when a peer sends an overlong frame or malformed JSON instead of discarding and continuing.
pub struct BoundedNdjsonTransport<R, W> {
    read: BufReader<R>,
    line_buf: Vec<u8>,
    max_frame_bytes: usize,
    failed: bool,
    write: Arc<Mutex<Option<W>>>,
    write_cancel: Arc<watch::Sender<bool>>,
    close_tx: Option<watch::Sender<Option<TransportClose>>>,
}

enum LineRead {
    Line,
    Oversized,
    Eof,
    Io,
}

enum ReadStep {
    Line,
    More,
    Oversized,
}

impl<R, W> BoundedNdjsonTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Creates a bounded NDJSON transport with the default [`MAX_FRAME_BYTES`] (256 KiB) limit.
    #[must_use]
    pub fn new(read: R, write: W) -> Self {
        Self::with_max_frame_bytes(read, write, MAX_FRAME_BYTES)
    }

    /// Creates a bounded NDJSON transport with an explicit maximum frame length (including `\n`).
    #[must_use]
    pub fn with_max_frame_bytes(read: R, write: W, max_frame_bytes: usize) -> Self {
        let buf_cap = INITIAL_READ_BUF_CAPACITY.min(max_frame_bytes.max(1));
        let (write_cancel_tx, _) = watch::channel(false);
        Self {
            read: BufReader::with_capacity(buf_cap, read),
            line_buf: Vec::new(),
            max_frame_bytes,
            failed: false,
            write: Arc::new(Mutex::new(Some(write))),
            write_cancel: Arc::new(write_cancel_tx),
            close_tx: None,
        }
    }

    pub(crate) fn with_close_signal(
        read: R,
        write: W,
    ) -> (
        Self,
        watch::Receiver<Option<TransportClose>>,
        WriteCancelHandle,
    ) {
        let (tx, rx) = watch::channel(None);
        let mut transport = Self::new(read, write);
        transport.close_tx = Some(tx);
        let write_cancel = WriteCancelHandle {
            tx: Arc::clone(&transport.write_cancel),
        };
        (transport, rx, write_cancel)
    }

    fn cancel_writes(&self) {
        self.write_cancel.send_if_modified(|cancelled| {
            if *cancelled {
                false
            } else {
                *cancelled = true;
                true
            }
        });
    }

    fn signal_close(&self, reason: TransportClose) {
        if let Some(tx) = &self.close_tx {
            let _ = tx.send(Some(reason));
        }
    }

    /// Reads one `\n`-terminated frame into `self.line_buf` without ever exceeding
    /// `self.max_frame_bytes`.
    ///
    /// Cancellation-safe: `fill_buf` consumes nothing if dropped while pending, and the copy plus
    /// `consume` happen synchronously with no `.await` in between.
    async fn read_line(&mut self) -> LineRead {
        loop {
            let (consumed, step) = {
                let Ok(available) = self.read.fill_buf().await else {
                    return LineRead::Io;
                };
                if available.is_empty() {
                    return LineRead::Eof;
                }

                let newline = available.iter().position(|&b| b == b'\n');
                let take = newline.map_or(available.len(), |idx| idx + 1);

                if self.line_buf.len().saturating_add(take) > self.max_frame_bytes {
                    self.line_buf.clear();
                    self.line_buf.shrink_to_fit();
                    (take, ReadStep::Oversized)
                } else {
                    self.line_buf.extend_from_slice(&available[..take]);
                    let step = if newline.is_some() {
                        ReadStep::Line
                    } else {
                        ReadStep::More
                    };
                    (take, step)
                }
            };
            self.read.consume(consumed);

            match step {
                ReadStep::Line => return LineRead::Line,
                ReadStep::More => {}
                ReadStep::Oversized => return LineRead::Oversized,
            }
        }
    }

    async fn shutdown_writer(&mut self) {
        self.cancel_writes();
        let mut guard = self.write.lock().await;
        if let Some(mut writer) = guard.take() {
            let _ = writer.shutdown().await;
        }
    }
}

fn closed_write_error() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "MCP transport is closed")
}

async fn wait_for_write_cancel(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
}

impl<R, W> Transport<RoleServer> for BoundedNdjsonTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let write_lock = Arc::clone(&self.write);
        let mut cancel_rx = self.write_cancel.subscribe();
        async move {
            let mut payload = serde_json::to_vec(&item)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            payload.push(b'\n');

            if *cancel_rx.borrow() {
                return Err(closed_write_error());
            }

            let mut guard = tokio::select! {
                biased;
                () = wait_for_write_cancel(&mut cancel_rx) => {
                    return Err(closed_write_error());
                }
                guard = write_lock.lock() => guard,
            };

            let Some(writer) = guard.as_mut() else {
                return Err(closed_write_error());
            };

            tokio::select! {
                biased;
                () = wait_for_write_cancel(&mut cancel_rx) => Err(closed_write_error()),
                res = async {
                    writer.write_all(&payload).await?;
                    writer.flush().await
                } => res,
            }
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        if self.failed {
            return None;
        }

        match self.read_line().await {
            LineRead::Line => {}
            LineRead::Oversized | LineRead::Io => {
                self.failed = true;
                self.cancel_writes();
                self.signal_close(TransportClose::Failed);
                self.shutdown_writer().await;
                return None;
            }
            LineRead::Eof => {
                self.line_buf.clear();
                self.signal_close(TransportClose::Eof);
                return None;
            }
        }

        let raw = self.line_buf.strip_suffix(b"\n").unwrap_or(&self.line_buf);
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let parsed = serde_json::from_slice::<RxJsonRpcMessage<RoleServer>>(raw);
        self.line_buf.clear();

        if let Ok(msg) = parsed {
            Some(msg)
        } else {
            self.failed = true;
            self.cancel_writes();
            self.signal_close(TransportClose::Failed);
            self.shutdown_writer().await;
            None
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.cancel_writes();
        let mut guard = self.write.lock().await;
        if let Some(mut writer) = guard.take() {
            writer.shutdown().await?;
        }
        Ok(())
    }
}

//! MCP transport adapter for Quickie.
//!
//! Provides:
//! - [`BoundedNdjsonTransport`] and [`serve_stream`] for generic `AsyncRead`/`AsyncWrite` MCP
//!   server connections with a strict 256 KiB frame budget.
//! - [`serve_socket`] for a private per-user Unix domain socket listener with peer-credential
//!   verification, `flock`-backed lifetime ownership, controlled stale reclaim, and a 16-client
//!   concurrency cap.
//! - [`run_relay`] / [`run_relay_fds`] for opaque CLI-to-GUI Unix socket relaying with half-close
//!   drain and immediate exit on server close.
//! - [`IsolatedStdio`] and [`serve_stdio`] for standalone stdio serving with process-global stdout
//!   isolation.

#![expect(
    unsafe_code,
    reason = "Unix FFI for confstr, open/O_NOFOLLOW, flock, fcntl, dup2, geteuid, and non-blocking fd I/O"
)]

pub mod browser;
pub mod framing;
pub mod socket;
pub mod stdio;

use std::io;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

pub use rmcp;
use rmcp::ServerHandler;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

pub use crate::browser::{
    AccessibilitySource, BrowserHandler, BrowserHandlerConfig, BrowserHandlerFactory, BrowserHost,
    BrowserOperation, BrowserStatus, DEFAULT_EVENTS_LIMIT, DEFAULT_MAX_IN_FLIGHT,
    DEFAULT_REQUEST_TIMEOUT, DiagnosticDetachResult, DiagnosticGrant, ForkCapabilities,
    HostDispatchError, HostPlatform, HostReply, MAX_ERROR_MESSAGE_BYTES, MAX_SELECTOR_BYTES,
    PageEventsResult, PageSnapshotResult, RuntimeMode, TOOL_CATALOG, ToolError, ToolErrorCode,
    ToolId, ToolSpec, supported_operation_names,
};
use crate::framing::TransportClose;
pub use crate::framing::{BoundedNdjsonTransport, MAX_FRAME_BYTES};
pub use crate::socket::endpoint::{
    DEFAULT_SOCKET_DIR_NAME, DEFAULT_SOCKET_FILE_NAME, SocketEndpoint, default_runtime_dir,
    default_socket_path,
};
pub use crate::socket::{MAX_CONCURRENT_CONNECTIONS, serve_socket};
pub use crate::stdio::relay::{RelayConfig, connect_socket, run_relay, run_relay_fds};
pub use crate::stdio::{IsolatedStdio, serve_stdio};

/// Errors returned by the MCP transport adapter.
#[derive(Debug, Error)]
pub enum McpError {
    /// The socket endpoint or one of its parent directories failed path, type, ownership, or mode
    /// validation.
    #[error("invalid MCP endpoint: {0}")]
    InvalidEndpoint(String),

    /// The socket endpoint (or its runtime parent directory) does not exist.
    #[error("MCP endpoint does not exist: {}", .0.display())]
    EndpointMissing(PathBuf),

    /// A socket file exists at the endpoint path, but no live listener accepted the connection.
    #[error("MCP endpoint is stale (connection refused): {}", .0.display())]
    EndpointStale(PathBuf),

    /// Another live listener already owns the endpoint or its lock.
    #[error("MCP endpoint is already in use: {}", .0.display())]
    EndpointInUse(PathBuf),

    /// Access to the endpoint was denied by the operating system.
    #[error("permission denied accessing MCP endpoint: {}", .0.display())]
    PermissionDenied(PathBuf),

    /// Connected Unix socket peer credentials did not match the current user's effective UID.
    #[error("MCP peer UID mismatch: expected {expected}, got {actual}")]
    PeerUidMismatch {
        /// Effective UID of this process.
        expected: u32,
        /// Effective UID reported by the kernel for the peer.
        actual: u32,
    },

    /// Underlying operating system or stream I/O error.
    #[error("MCP transport I/O error: {0}")]
    Io(#[from] io::Error),
}

impl McpError {
    /// Returns `true` only when the endpoint was verified absent (`ENOENT`), allowing attach-only
    /// callers to distinguish a missing GUI from stale, permission, or other errors.
    #[must_use]
    pub fn is_missing_endpoint(&self) -> bool {
        matches!(self, Self::EndpointMissing(_))
    }
}

/// Host-owned shutdown trigger paired with a [`ShutdownSignal`].
#[derive(Debug)]
pub struct ShutdownTrigger {
    tx: watch::Sender<bool>,
}

impl ShutdownTrigger {
    /// Creates a new `(ShutdownTrigger, ShutdownSignal)` pair.
    #[must_use]
    pub fn new() -> (Self, ShutdownSignal) {
        let (tx, rx) = watch::channel(false);
        (Self { tx }, ShutdownSignal { rx })
    }

    /// Signals shutdown to all clones of the paired [`ShutdownSignal`].
    pub fn trigger(&self) {
        let _ = self.tx.send(true);
    }
}

/// Cloneable async shutdown receiver for [`serve_stream`].
#[derive(Debug, Clone)]
pub struct ShutdownSignal {
    rx: watch::Receiver<bool>,
}

impl ShutdownSignal {
    pub(crate) fn from_receiver(rx: watch::Receiver<bool>) -> Self {
        Self { rx }
    }

    /// Waits until shutdown has been triggered or the trigger has been dropped.
    pub async fn wait(&mut self) {
        if *self.rx.borrow() {
            return;
        }
        while self.rx.changed().await.is_ok() {
            if *self.rx.borrow() {
                return;
            }
        }
    }
}

/// Host-owned handle to a running MCP server (Unix socket listener or standalone stdio server).
///
/// Calling [`ServerHandle::shutdown`] or dropping the handle signals shutdown, cancels active
/// connections, joins the dedicated I/O thread, and performs inode-checked socket cleanup.
#[derive(Debug)]
pub struct ServerHandle {
    socket_path: Option<PathBuf>,
    shutdown_tx: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    pub(crate) fn new(
        socket_path: Option<PathBuf>,
        shutdown_tx: watch::Sender<bool>,
        thread: JoinHandle<()>,
    ) -> Self {
        Self {
            socket_path,
            shutdown_tx,
            thread: Some(thread),
        }
    }

    /// Returns the bound Unix socket path when this handle belongs to a socket listener.
    #[must_use]
    pub fn socket_path(&self) -> Option<&Path> {
        self.socket_path.as_deref()
    }

    /// Signals shutdown and waits for the I/O thread to exit cleanly.
    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    /// Waits for the I/O thread to exit on its own (for example, after standalone `stdin` EOF).
    pub fn wait(mut self) {
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    fn shutdown_inner(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

/// Grace window after a clean peer read EOF to let already-completing half-close responses flush
/// before cancelling parked in-flight requests.
const EOF_DRAIN_GRACE: Duration = Duration::from_millis(150);

/// Serves a single MCP connection over generic `AsyncRead` and `AsyncWrite` streams using
/// [`BoundedNdjsonTransport`] (256 KiB frame limit) and host-owned shutdown.
///
/// Suitable for `tokio::io::duplex` in tests as well as Unix sockets and isolated stdio pipes.
///
/// # Errors
///
/// Returns [`McpError`] if a transport-level failure prevents serving the stream; normal client
/// disconnects and malformed/oversized peer frames terminate the connection cleanly with `Ok(())`.
pub async fn serve_stream<H, R, W>(
    handler: H,
    read: R,
    write: W,
    mut shutdown: ShutdownSignal,
) -> Result<(), McpError>
where
    H: ServerHandler + Send + 'static,
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (transport, mut close_rx, write_cancel) =
        BoundedNdjsonTransport::with_close_signal(read, write);

    let running = tokio::select! {
        biased;
        () = shutdown.wait() => {
            write_cancel.cancel();
            return Ok(());
        }
        res = rmcp::serve_server(handler, transport) => match res {
            Ok(running) => running,
            Err(_) => return Ok(()),
        },
    };

    let cancel_token = running.cancellation_token();
    let cancel_watcher = tokio::spawn(async move {
        tokio::select! {
            biased;
            () = shutdown.wait() => {
                write_cancel.cancel();
                cancel_token.cancel();
            }
            close = wait_for_transport_close(&mut close_rx) => {
                if close == Some(TransportClose::Eof) {
                    tokio::select! {
                        biased;
                        () = shutdown.wait() => {}
                        () = tokio::time::sleep(EOF_DRAIN_GRACE) => {}
                    }
                }
                write_cancel.cancel();
                cancel_token.cancel();
            }
        }
    });

    let _ = running.waiting().await;
    cancel_watcher.abort();
    Ok(())
}

async fn wait_for_transport_close(
    rx: &mut watch::Receiver<Option<TransportClose>>,
) -> Option<TransportClose> {
    if let Some(reason) = *rx.borrow() {
        return Some(reason);
    }
    while rx.changed().await.is_ok() {
        if let Some(reason) = *rx.borrow() {
            return Some(reason);
        }
    }
    None
}

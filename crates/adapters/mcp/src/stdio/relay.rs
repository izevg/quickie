//! Opaque byte relay between CLI stdio pipes and an existing GUI private Unix socket.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

use super::async_fd::{AsyncPipeReader, AsyncPipeWriter};
use super::dup_cloexec;
use crate::McpError;
use crate::socket::endpoint::{SocketEndpoint, current_euid};

/// Configuration for the opaque CLI-to-GUI Unix socket relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayConfig {
    /// Maximum time to drain server-to-client replies after client `stdin` reaches EOF and the
    /// Unix socket write half has been shut down.
    pub drain_timeout: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(2),
        }
    }
}

/// Connects to an existing GUI private Unix socket with strict path validation and attach-only
/// error classification.
///
/// Never creates directories or socket files. Distinguishes [`McpError::EndpointMissing`] (safe
/// for fallback when verified absent) from [`McpError::EndpointStale`],
/// [`McpError::InvalidEndpoint`], [`McpError::PermissionDenied`], and [`McpError::Io`].
///
/// # Errors
///
/// Returns a typed [`McpError`] describing why the endpoint could not be attached.
pub fn connect_socket(endpoint: &SocketEndpoint) -> Result<StdUnixStream, McpError> {
    endpoint.validate_for_connect()?;

    let stream =
        StdUnixStream::connect(endpoint.socket_path()).map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => {
                McpError::EndpointMissing(endpoint.socket_path().to_path_buf())
            }
            io::ErrorKind::ConnectionRefused => {
                McpError::EndpointStale(endpoint.socket_path().to_path_buf())
            }
            io::ErrorKind::PermissionDenied => {
                McpError::PermissionDenied(endpoint.socket_path().to_path_buf())
            }
            _ => McpError::Io(err),
        })?;
    stream.set_nonblocking(true).map_err(McpError::Io)?;
    Ok(stream)
}

/// Relays opaque bytes between the process's `stdin`/`stdout` and an existing GUI Unix socket.
///
/// - Verifies peer UID via Tokio Unix peer credentials before reading or writing any bytes.
/// - On client `stdin` EOF: half-closes the Unix socket write side and drains accepted replies up
///   to `config.drain_timeout`.
/// - On server close: exits immediately even if client `stdin` remains open (no stuck blocking
///   threads).
/// - Writes protocol bytes only to `stdout`; never emits ANSI escapes or assumes a TTY.
///
/// # Errors
///
/// Returns [`McpError::EndpointMissing`] when the socket does not exist,
/// [`McpError::EndpointStale`] when a stale socket file exists with no listener,
/// [`McpError::PeerUidMismatch`] if peer credentials do not match the current effective UID, or
/// [`McpError::InvalidEndpoint`] / [`McpError::Io`] on validation or I/O failure.
pub fn run_relay(endpoint: &SocketEndpoint, config: RelayConfig) -> Result<(), McpError> {
    let stdin = dup_cloexec(libc::STDIN_FILENO).map_err(McpError::Io)?;
    let stdout = dup_cloexec(libc::STDOUT_FILENO).map_err(McpError::Io)?;
    run_relay_fds(endpoint, stdin, stdout, config)
}

/// Relays opaque bytes between explicit `(stdin, stdout)` pipe descriptors and an existing GUI
/// Unix socket.
///
/// # Errors
///
/// Same error semantics as [`run_relay`].
pub fn run_relay_fds(
    endpoint: &SocketEndpoint,
    stdin: OwnedFd,
    stdout: OwnedFd,
    config: RelayConfig,
) -> Result<(), McpError> {
    let std_stream = connect_socket(endpoint)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(McpError::Io)?;

    runtime.block_on(async move {
        let stream = UnixStream::from_std(std_stream).map_err(McpError::Io)?;
        let expected_uid = current_euid();
        let peer = stream.peer_cred().map_err(McpError::Io)?;
        if peer.uid() != expected_uid {
            return Err(McpError::PeerUidMismatch {
                expected: expected_uid,
                actual: peer.uid(),
            });
        }

        let mut stdin_reader = AsyncPipeReader::new(stdin).map_err(McpError::Io)?;
        let mut stdout_writer = AsyncPipeWriter::new(stdout).map_err(McpError::Io)?;
        let (mut sock_read, mut sock_write) = stream.into_split();

        let mut client_to_server = std::pin::pin!(async {
            let copy_res = tokio::io::copy(&mut stdin_reader, &mut sock_write).await;
            let _ = sock_write.shutdown().await;
            copy_res
        });

        let mut server_to_client = std::pin::pin!(async {
            let copy_res = tokio::io::copy(&mut sock_read, &mut stdout_writer).await;
            let _ = stdout_writer.flush().await;
            copy_res
        });

        tokio::select! {
            res = &mut server_to_client => {
                // Server closed (or stdout pipe broke). Drop client_to_server immediately so an
                // open stdin pipe never blocks exit.
                ignore_broken_pipe(res)
            }
            res = &mut client_to_server => {
                // Client stdin hit EOF and the socket write side is half-closed. Drain remaining
                // replies from the server with a bounded graceful timeout.
                let c2s = ignore_broken_pipe(res);
                let drained = tokio::time::timeout(config.drain_timeout, server_to_client).await;
                let s2c = match drained {
                    Ok(inner) => ignore_broken_pipe(inner),
                    Err(_) => Ok(()),
                };
                c2s.and(s2c)
            }
        }
    })
}

fn ignore_broken_pipe(res: io::Result<u64>) -> Result<(), McpError> {
    match res {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(McpError::Io(err)),
    }
}

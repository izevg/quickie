//! Standalone stdio MCP serving with process-global stdout isolation and opaque Unix socket relay.

mod async_fd;
pub mod relay;

use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::thread;

use rmcp::ServerHandler;
use tokio::sync::watch;

use self::async_fd::{AsyncPipeReader, AsyncPipeWriter};
use crate::{McpError, ServerHandle, ShutdownSignal, serve_stream};

/// Owned close-on-exec file descriptors for an isolated standalone stdio MCP session.
///
/// Created by [`IsolatedStdio::isolate`] before native toolkit initialization so native chatter
/// written to fd 1 (`stdout`) is redirected to fd 2 (`stderr`), while MCP protocol frames travel
/// exclusively over the duplicated protocol descriptor.
#[derive(Debug)]
pub struct IsolatedStdio {
    stdin: OwnedFd,
    stdout: OwnedFd,
}

impl IsolatedStdio {
    /// Isolates process `stdout` for MCP protocol traffic and redirects fd 1 to `stderr`.
    ///
    /// # Process-global semantics
    ///
    /// This operation mutates file descriptor 1 (`STDOUT_FILENO`) for the entire process.
    /// Call it **exactly once**, on the main thread, **before** initializing `AppKit`, GTK,
    /// `WebKit`, or spawning any background threads.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Io`] if duplicating `stdin`/`stdout` or redirecting fd 1 to `stderr`
    /// fails. On error, any already-duplicated descriptor is closed cleanly.
    pub fn isolate() -> Result<Self, McpError> {
        let stdout = dup_cloexec(libc::STDOUT_FILENO).map_err(McpError::Io)?;
        dup2_retry(libc::STDERR_FILENO, libc::STDOUT_FILENO).map_err(McpError::Io)?;
        let stdin = dup_cloexec(libc::STDIN_FILENO).map_err(McpError::Io)?;
        Ok(Self { stdin, stdout })
    }

    /// Wraps explicit owned file descriptors (for tests or pre-isolated pipes) without mutating
    /// process-global file descriptors.
    #[must_use]
    pub fn from_fds(stdin: OwnedFd, stdout: OwnedFd) -> Self {
        Self { stdin, stdout }
    }

    /// Consumes this token and returns the underlying `(stdin, stdout)` descriptors.
    #[must_use]
    pub fn into_fds(self) -> (OwnedFd, OwnedFd) {
        (self.stdin, self.stdout)
    }
}

/// Starts a standalone stdio MCP server over an [`IsolatedStdio`] stream on a single named I/O
/// thread with a current-thread Tokio runtime.
///
/// Never touches browser profile or socket state. `stdin` EOF terminates only this server's
/// session cleanly.
///
/// # Errors
///
/// Returns [`McpError::Io`] if spawning the I/O thread fails.
pub fn serve_stdio<H>(isolated: IsolatedStdio, handler: H) -> Result<ServerHandle, McpError>
where
    H: ServerHandler + Send + 'static,
{
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (stdin_fd, stdout_fd) = isolated.into_fds();

    let thread_handle = thread::Builder::new()
        .name("quickie-mcp-stdio".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
            else {
                return;
            };

            runtime.block_on(async move {
                let Ok(reader) = AsyncPipeReader::new(stdin_fd) else {
                    return;
                };
                let Ok(writer) = AsyncPipeWriter::new(stdout_fd) else {
                    return;
                };
                let _ = serve_stream(
                    handler,
                    reader,
                    writer,
                    ShutdownSignal::from_receiver(shutdown_rx),
                )
                .await;
            });
        })
        .map_err(McpError::Io)?;

    Ok(ServerHandle::new(None, shutdown_tx, thread_handle))
}

pub(crate) fn dup_cloexec(fd: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: `F_DUPFD_CLOEXEC` with minimum fd 0 duplicates `fd` with `FD_CLOEXEC` set.
    let duped = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duped < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `duped` is a newly created valid file descriptor owned exclusively here.
    Ok(unsafe { OwnedFd::from_raw_fd(duped) })
}

fn dup2_retry(old_fd: RawFd, new_fd: RawFd) -> io::Result<()> {
    loop {
        // SAFETY: `dup2` atomically redirects `new_fd` to `old_fd`.
        let rc = unsafe { libc::dup2(old_fd, new_fd) };
        if rc >= 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

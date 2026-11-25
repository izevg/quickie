//! Private Unix domain socket MCP listener for GUI hosts.

pub mod endpoint;
mod lock;

use std::sync::Arc;
use std::thread;

use rmcp::ServerHandler;
use tokio::net::UnixListener;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

use self::endpoint::{SocketEndpoint, current_euid};
use self::lock::SocketGuard;
use crate::{McpError, ServerHandle, ShutdownSignal, serve_stream};

/// Maximum number of simultaneous client connections accepted by the GUI socket listener.
pub const MAX_CONCURRENT_CONNECTIONS: usize = 16;

/// Starts a private Unix domain socket MCP server on a single named I/O thread with a
/// current-thread Tokio runtime.
///
/// The socket is bound synchronously before this function returns, so callers receive bind/lock
/// errors immediately and can connect as soon as `Ok(ServerHandle)` is returned.
///
/// # Errors
///
/// Returns [`McpError::InvalidEndpoint`] if endpoint validation fails, [`McpError::EndpointInUse`]
/// if another live listener owns the endpoint, or [`McpError::Io`] on OS errors.
pub fn serve_socket<F, H>(
    endpoint: &SocketEndpoint,
    handler_factory: F,
) -> Result<ServerHandle, McpError>
where
    F: Fn() -> H + Send + Sync + 'static,
    H: ServerHandler + Send + 'static,
{
    let (std_listener, guard) = SocketGuard::bind(endpoint)?;
    let socket_path = endpoint.socket_path().to_path_buf();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let factory = Arc::new(handler_factory);

    let thread_handle = thread::Builder::new()
        .name("quickie-mcp-socket".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
            else {
                return;
            };

            runtime.block_on(async move {
                let _guard = guard;
                let Ok(listener) = UnixListener::from_std(std_listener) else {
                    return;
                };
                run_accept_loop(listener, factory, shutdown_rx).await;
            });
        })
        .map_err(McpError::Io)?;

    Ok(ServerHandle::new(
        Some(socket_path),
        shutdown_tx,
        thread_handle,
    ))
}

async fn run_accept_loop<F, H>(
    listener: UnixListener,
    factory: Arc<F>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    F: Fn() -> H + Send + Sync + 'static,
    H: ServerHandler + Send + 'static,
{
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let expected_uid = current_euid();
    let mut connections = JoinSet::new();

    loop {
        while connections.try_join_next().is_some() {}

        let accepted = tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
                continue;
            }
            res = listener.accept() => res,
        };

        let Ok((stream, _addr)) = accepted else {
            continue;
        };

        let Ok(cred) = stream.peer_cred() else {
            drop(stream);
            continue;
        };
        if cred.uid() != expected_uid {
            drop(stream);
            continue;
        }

        let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
            drop(stream);
            continue;
        };

        let handler = factory();
        let conn_shutdown = ShutdownSignal::from_receiver(shutdown_rx.clone());
        connections.spawn(async move {
            let _permit = permit;
            let (read_half, write_half) = stream.into_split();
            let _ = serve_stream(handler, read_half, write_half, conn_shutdown).await;
        });
    }

    drop(listener);
    while connections.join_next().await.is_some() {}
}

//! In-process MCP host and CLI relay/standalone entry point for Linux Quickie.

#[cfg(any(target_os = "linux", test))]
pub(crate) mod agent;
#[cfg(target_os = "linux")]
pub(crate) mod gtk;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod runtime;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod snapshot;
#[cfg(any(not(target_os = "linux"), test))]
pub(crate) mod standalone;

#[cfg(any(not(target_os = "linux"), test))]
use std::sync::Arc;

#[cfg(any(not(target_os = "linux"), test))]
use quickie_mcp::{BrowserHandler, BrowserHost, serve_stdio};
use quickie_mcp::{IsolatedStdio, McpError, RelayConfig, SocketEndpoint, run_relay};

#[cfg(not(target_os = "linux"))]
use self::standalone::StandaloneHost;

/// Product version string shared across `--version` and MCP `browser_status`.
#[must_use]
pub(crate) fn product_version() -> &'static str {
    option_env!("QUICKIE_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Runs `quickie mcp` / `quickie --mcp` before GTK initialization.
///
/// Attempts to attach to an existing GUI-owned private Unix socket first. Falls back to an
/// isolated standalone stdio MCP server only when the endpoint is verified absent (`ENOENT`) and
/// `attach_only` is `false`. Any other transport or endpoint validation error fails immediately.
///
/// # Errors
///
/// Returns a human-readable error message when endpoint validation, relay, attach-only lookup, or
/// standalone stdio isolation fails.
pub(crate) fn run(attach_only: bool) -> Result<(), String> {
    let endpoint = SocketEndpoint::default_endpoint().map_err(|error| error.to_string())?;
    run_with_endpoint(
        &endpoint,
        attach_only,
        |ep| run_relay(ep, RelayConfig::default()),
        IsolatedStdio::isolate,
    )
}

pub(crate) fn run_with_endpoint<R, I>(
    endpoint: &SocketEndpoint,
    attach_only: bool,
    relay: R,
    isolate_stdio: I,
) -> Result<(), String>
where
    R: FnOnce(&SocketEndpoint) -> Result<(), McpError>,
    I: FnOnce() -> Result<IsolatedStdio, McpError>,
{
    match relay(endpoint) {
        Ok(()) => Ok(()),
        Err(error) if error.is_missing_endpoint() => {
            if attach_only {
                return Err(format!(
                    "no running Quickie GUI MCP server at {}",
                    endpoint.socket_path().display()
                ));
            }
            let isolated = isolate_stdio().map_err(|err| err.to_string())?;
            #[cfg(target_os = "linux")]
            {
                gtk::agent::serve_standalone(isolated, product_version())
            }
            #[cfg(not(target_os = "linux"))]
            {
                serve_standalone_stdio(isolated, Arc::new(StandaloneHost::new(product_version())))
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn serve_standalone_stdio<H: BrowserHost>(
    isolated: IsolatedStdio,
    host: Arc<H>,
) -> Result<(), String> {
    serve_standalone_stdio_with_version(isolated, host, product_version())
}

#[cfg(any(not(target_os = "linux"), test))]
pub(crate) fn serve_standalone_stdio_with_version<H: BrowserHost>(
    isolated: IsolatedStdio,
    host: Arc<H>,
    version: &str,
) -> Result<(), String> {
    let handler = BrowserHandler::new(quickie_application::mcp::ClientId::new(1), host)
        .with_product_version(version);
    let handle = serve_stdio(isolated, handler).map_err(|error| error.to_string())?;
    handle.wait();
    Ok(())
}

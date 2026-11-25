//! CLI parsing, private Unix socket relay, and isolated standalone stdio MCP server.
//!
//! Invoked in `main()` **before** `AppKit` initialization (`MainThreadMarker` / `NSApplication`).
//! Standalone mode isolates `stdout` to `stderr` before starting the MCP stdio thread, never opens
//! the normal browser profile, never activates any UI, and advertises only implemented
//! capabilities.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use objc2::MainThreadMarker;
use quickie_application::mcp::{ClientId, ObservationTarget};
use quickie_mcp::{
    BrowserHandlerFactory, BrowserHost, BrowserOperation, BrowserStatus, ForkCapabilities,
    HostDispatchError, HostPlatform, IsolatedStdio, McpError, RelayConfig, RuntimeMode,
    SocketEndpoint, ToolError, ToolErrorCode, run_relay, serve_stdio,
};

use super::agent::native::NativeAgentBackend;
use super::agent::standalone::run_native_standalone;
use crate::updates::INSTALLED_VERSION;

/// Parsed CLI intent for `quickie mcp` / `quickie --mcp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct McpCliOptions {
    /// Fail instead of falling back to standalone mode when the GUI socket is absent.
    pub(crate) attach_only: bool,
    /// Optional explicit socket path (validated against the OS-trusted private runtime root).
    pub(crate) socket: Option<PathBuf>,
}

/// Top-level CLI dispatch outcome evaluated before `AppKit` starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CliAction {
    /// Normal GUI or non-MCP invocation.
    NotMcp,
    /// Run MCP relay (with optional standalone fallback).
    RunMcp(McpCliOptions),
    /// Print MCP CLI usage and exit successfully.
    ShowHelp,
    /// Invalid MCP CLI invocation; print message to stderr and exit non-zero.
    InvalidArgs(String),
}

const USAGE: &str = "usage: quickie mcp [--attach-only] [--socket <path>]\n       quickie --mcp [--attach-only] [--socket <path>]";

/// Parses process arguments (excluding `argv[0]`) to detect `quickie mcp` / `quickie --mcp`.
#[must_use]
pub(crate) fn parse_cli_args(args: &[String]) -> CliAction {
    let Some((first, rest)) = args.split_first() else {
        return CliAction::NotMcp;
    };
    if first != "mcp" && first != "--mcp" {
        return CliAction::NotMcp;
    }

    let mut attach_only = false;
    let mut socket = None;
    let mut idx = 0;

    while idx < rest.len() {
        let arg = &rest[idx];
        if arg == "--attach-only" {
            attach_only = true;
            idx += 1;
        } else if arg == "--help" || arg == "-h" {
            return CliAction::ShowHelp;
        } else if arg == "--socket" {
            let Some(path) = rest.get(idx + 1) else {
                return CliAction::InvalidArgs(format!("--socket requires a path\n{USAGE}"));
            };
            if path.is_empty() {
                return CliAction::InvalidArgs(format!("--socket path must not be empty\n{USAGE}"));
            }
            socket = Some(PathBuf::from(path));
            idx += 2;
        } else if let Some(path) = arg.strip_prefix("--socket=") {
            if path.is_empty() {
                return CliAction::InvalidArgs(format!("--socket path must not be empty\n{USAGE}"));
            }
            socket = Some(PathBuf::from(path));
            idx += 1;
        } else {
            return CliAction::InvalidArgs(format!("unknown MCP flag: {arg}\n{USAGE}"));
        }
    }

    CliAction::RunMcp(McpCliOptions {
        attach_only,
        socket,
    })
}

/// Resolves and validates the private Unix socket endpoint for `options`.
///
/// # Errors
///
/// Returns [`McpError::InvalidEndpoint`] when the runtime directory is unavailable or an explicit
/// `--socket` path escapes the trusted per-user runtime root.
pub(crate) fn resolve_endpoint(options: &McpCliOptions) -> Result<SocketEndpoint, McpError> {
    match &options.socket {
        Some(path) => SocketEndpoint::from_path(path),
        None => SocketEndpoint::default_endpoint(),
    }
}

/// Decides whether a relay error permits falling back to the isolated standalone MCP server.
///
/// Fallback is allowed **only** when `--attach-only` is false and the endpoint was verified
/// missing (`McpError::EndpointMissing`). Stale sockets, invalid endpoints, permission errors,
/// UID mismatches, and I/O errors always fail closed.
#[must_use]
pub(crate) fn should_fallback_to_standalone(attach_only: bool, error: &McpError) -> bool {
    !attach_only && error.is_missing_endpoint()
}

/// Executes the MCP CLI entrypoint before any `AppKit` or browser profile initialization.
pub(crate) fn run_cli(action: CliAction) -> ExitCode {
    match action {
        CliAction::NotMcp => ExitCode::SUCCESS,
        CliAction::ShowHelp => {
            use std::io::{self, Write};
            match writeln!(io::stdout().lock(), "{USAGE}") {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => cli_fail(&error.to_string()),
            }
        }
        CliAction::InvalidArgs(reason) => cli_fail(&reason),
        CliAction::RunMcp(options) => run_mcp(&options),
    }
}

fn run_mcp(options: &McpCliOptions) -> ExitCode {
    let endpoint = match resolve_endpoint(options) {
        Ok(endpoint) => endpoint,
        Err(error) => return cli_fail(&error.to_string()),
    };

    match run_relay(&endpoint, RelayConfig::default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if should_fallback_to_standalone(options.attach_only, &error) => {
            run_standalone()
        }
        Err(error) => cli_fail(&error.to_string()),
    }
}

fn run_standalone() -> ExitCode {
    // Isolate process stdout before any native runtime or server thread starts.
    let isolated = match IsolatedStdio::isolate() {
        Ok(isolated) => isolated,
        Err(error) => return cli_fail(&format!("could not isolate stdio: {error}")),
    };
    let caps = NativeAgentBackend::probe_capabilities();
    if caps.isolated_automation
        && let Some(mtm) = MainThreadMarker::new()
    {
        return match run_native_standalone(isolated, mtm) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => cli_fail(&format!("could not start standalone MCP server: {error}")),
        };
    }

    // Stock macOS WebKit or non-WebKit build: serve `browser_status` honestly without starting an
    // AppKit loop or opening any user profile.
    let factory = BrowserHandlerFactory::new(Arc::new(StandaloneHost::new()))
        .with_product_version(INSTALLED_VERSION);
    let handle = match serve_stdio(isolated, factory.next_handler()) {
        Ok(handle) => handle,
        Err(error) => return cli_fail(&format!("could not start standalone MCP server: {error}")),
    };
    handle.wait();
    ExitCode::SUCCESS
}

#[expect(
    clippy::print_stderr,
    reason = "MCP CLI diagnostics must go to stderr so stdout remains reserved for protocol frames"
)]
fn cli_fail(reason: &str) -> ExitCode {
    eprintln!("quickie mcp: {reason}");
    ExitCode::FAILURE
}

/// Isolated standalone MCP host that never opens the normal profile or activates any UI.
///
/// Responds to `browser_status` honestly with `RuntimeMode::Standalone` and empty live-tab
/// capabilities, and reports live tab diagnostics (and unwired agent tools) as unavailable.
#[derive(Debug, Default)]
pub(crate) struct StandaloneHost;

impl StandaloneHost {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl BrowserHost for StandaloneHost {
    fn dispatch(
        &self,
        _client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError> {
        match operation {
            BrowserOperation::BrowserStatus { reply } => {
                reply.resolve(BrowserStatus {
                    platform: HostPlatform::Macos,
                    runtime_mode: RuntimeMode::Standalone,
                    product_version: INSTALLED_VERSION.to_owned(),
                    fork_capabilities: ForkCapabilities::default(),
                });
            }
            BrowserOperation::DiagnosticRequest { reply, .. } => {
                reply.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "live tab diagnostics are unavailable in standalone mode",
                ));
            }
            BrowserOperation::DiagnosticDetach { reply, .. } => {
                reply.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "live tab diagnostics are unavailable in standalone mode",
                ));
            }
            BrowserOperation::PageSnapshot { target, reply, .. } => {
                reply.reject(standalone_target_error(target));
            }
            BrowserOperation::PageEvents { target, reply, .. } => {
                reply.reject(standalone_target_error(target));
            }
            BrowserOperation::Agent(agent_op) => {
                agent_op.reject(ToolError::new(
                    ToolErrorCode::UnsupportedCapability,
                    "isolated agent automation requires the Quickie WebKit fork",
                ));
            }
        }
        Ok(())
    }

    fn client_closed(&self, _client: ClientId) {}
}

fn standalone_target_error(target: ObservationTarget) -> ToolError {
    let message = match target {
        ObservationTarget::Diagnostic(_) => {
            "live tab diagnostics are unavailable in standalone mode"
        }
        ObservationTarget::Agent(_) => "isolated agent contexts are not available in this runtime",
    };
    ToolError::new(ToolErrorCode::ToolNotAvailable, message)
}

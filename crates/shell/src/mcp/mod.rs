//! macOS Quickie in-process MCP host, CLI relay/standalone entry, and native diagnostic consent.
//!
//! - [`cli`]: parses `quickie mcp` / `quickie --mcp` before `AppKit` initialization, relays to a
//!   validated private Unix socket, and falls back to an isolated standalone stdio server only on
//!   a verified missing endpoint.
//! - [`bridge`]: `BrowserHost` dispatch queue (`MainThreadBound<Weak<Browser>>` on the main
//!   `DispatchQueue`) and the pure main-thread `McpCoordinator` managing `DiagnosticState`,
//!   document epochs, explicit reload intent, leases, and revocation.
//! - [`consent`]: `AppKit` `DialogSheet` consent presentation (without stealing focus), active-tab
//!   `AgentBadge` stop indicator, and `Browser` lifecycle wiring.

pub(crate) mod agent;
pub(crate) mod bridge;
pub(crate) mod cli;
pub(crate) mod consent;
#[cfg(test)]
mod tests;

pub(crate) use cli::{CliAction, parse_cli_args, run_cli};
pub(crate) use consent::McpUi;

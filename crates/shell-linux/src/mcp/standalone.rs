//! Standalone stdio MCP host that never opens `Profile::Main` or initializes GTK.

use quickie_application::mcp::{ClientId, ObservationTarget};
use quickie_mcp::{
    BrowserHost, BrowserOperation, BrowserStatus, ForkCapabilities, HostDispatchError,
    HostPlatform, RuntimeMode, ToolError, ToolErrorCode,
};

/// In-process MCP host for standalone `quickie mcp` fallback.
///
/// Owns no GUI state, never touches the Main browsing profile on disk, and truthfully advertises
/// no live tab diagnostics and no isolated Agent automation contexts in this first wave.
#[derive(Debug, Clone)]
pub(crate) struct StandaloneHost {
    product_version: String,
}

impl StandaloneHost {
    #[must_use]
    pub(crate) fn new(product_version: impl Into<String>) -> Self {
        Self {
            product_version: product_version.into(),
        }
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
                    platform: HostPlatform::Linux,
                    runtime_mode: RuntimeMode::Standalone,
                    product_version: self.product_version.clone(),
                    fork_capabilities: ForkCapabilities::default(),
                });
            }
            BrowserOperation::DiagnosticRequest { reply, .. } => {
                reply.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "no live browser tab is available in standalone mode",
                ));
            }
            BrowserOperation::DiagnosticDetach { reply, .. } => {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "diagnostic attachment is not available",
                ));
            }
            BrowserOperation::PageSnapshot { target, reply, .. } => {
                reply.reject(unavailable_target_error(target));
            }
            BrowserOperation::PageEvents { target, reply, .. } => {
                reply.reject(unavailable_target_error(target));
            }
            BrowserOperation::Agent(operation) => {
                operation.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "isolated agent contexts are unavailable in this runtime",
                ));
            }
        }
        Ok(())
    }

    fn client_closed(&self, _client: ClientId) {}
}

fn unavailable_target_error(target: ObservationTarget) -> ToolError {
    match target {
        ObservationTarget::Diagnostic(_) => ToolError::new(
            ToolErrorCode::AttachmentUnavailable,
            "diagnostic attachment is not available",
        ),
        ObservationTarget::Agent(_) => ToolError::new(
            ToolErrorCode::ToolNotAvailable,
            "isolated agent contexts are unavailable in this runtime",
        ),
    }
}

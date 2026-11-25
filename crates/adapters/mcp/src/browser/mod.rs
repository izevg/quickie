//! Shared `rmcp` [`ServerHandler`] implementation with typed one-shot host dispatch.
//!
//! Used by both `AppKit` and `GTK` composition roots over [`crate::serve_socket`] and
//! [`crate::serve_stdio`]. Each [`BrowserHandler`] owns a single [`ClientId`], enforces a bounded
//! per-client in-flight budget, validates wire arguments before reaching [`BrowserHost`], handles
//! cancellation and timeouts without panicking on late replies, and notifies
//! [`BrowserHost::client_closed`] exactly once when the last handler ownership is released.

pub mod agent;
pub mod catalog;
pub mod host;
mod wire;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use quickie_application::mcp::ClientId;
use rmcp::ErrorData as RmcpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerConfig, ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer};
use tokio::sync::{Semaphore, oneshot};

pub use self::agent::{
    AgentActResult, AgentActStatus, AgentAction, AgentContextCloseResult, AgentContextListResult,
    AgentContextOpenResult, AgentContextSummary, AgentElementRef, AgentKey, AgentKeyModifier,
    AgentLocateResult, AgentLocator, AgentLocatorKind, AgentMouseButton, AgentNavigateResult,
    AgentNavigateStatus, AgentObservedState, AgentOperation, AgentScreenshotResult,
    AgentScreenshotTarget, AgentViewport, AgentWaitCondition, AgentWaitResult, AgentWaitStage,
    AgentWaitStatus, DEFAULT_ACTION_TIMEOUT_MS, DEFAULT_LOCATE_RESULTS,
    DEFAULT_NAVIGATE_TIMEOUT_MS, DEFAULT_SCREENSHOT_MAX_BYTES, DEFAULT_VIEWPORT_HEIGHT,
    DEFAULT_VIEWPORT_WIDTH, ElementBounds, ElementProvenance, FrameScope, LocatedElement,
    MAX_ACTION_TIMEOUT_MS, MAX_BBOX_COORD, MAX_ELEMENT_ID_BYTES, MAX_INPUT_TEXT_BYTES,
    MAX_KEY_MODIFIERS, MAX_LOCATE_RESULTS, MAX_LOCATOR_TEXT_BYTES, MAX_REASON_BYTES,
    MAX_ROLE_BYTES, MAX_SCREENSHOT_BYTES, MAX_SCROLL_DELTA, MAX_URL_BYTES, MAX_VIEWPORT_HEIGHT,
    MAX_VIEWPORT_WIDTH, MIN_ACTION_TIMEOUT_MS, MIN_VIEWPORT_HEIGHT, MIN_VIEWPORT_WIDTH,
    SelectOptionBy,
};
pub use self::catalog::{
    TOOL_CATALOG, ToolId, ToolSpec, supported_operation_names, supported_operation_names_for,
};
pub use self::host::{
    AccessibilitySource, BrowserHost, BrowserOperation, BrowserStatus, DiagnosticDetachResult,
    DiagnosticGrant, ForkCapabilities, HostDispatchError, HostPlatform, HostReply,
    MAX_ERROR_MESSAGE_BYTES, PageEventsResult, PageSnapshotResult, RuntimeMode, ToolError,
    ToolErrorCode,
};
pub use self::wire::{DEFAULT_EVENTS_LIMIT, MAX_SELECTOR_BYTES};

/// Default timeout for a single browser tool invocation.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum number of concurrent in-flight tool requests per MCP client.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 8;

/// Configuration for [`BrowserHandler`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserHandlerConfig {
    /// Maximum duration a tool call waits for the host to complete its [`HostReply`].
    pub request_timeout: Duration,
    /// Maximum simultaneous in-flight host dispatches allowed for this client connection.
    pub max_in_flight: usize,
}

impl Default for BrowserHandlerConfig {
    fn default() -> Self {
        Self {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
        }
    }
}

/// Issues per-connection [`BrowserHandler`] instances with unique monotonic [`ClientId`]s.
#[derive(Debug)]
pub struct BrowserHandlerFactory<H: BrowserHost> {
    host: Arc<H>,
    config: BrowserHandlerConfig,
    product_version: Option<Arc<str>>,
    next_client_id: AtomicU64,
}

impl<H: BrowserHost> BrowserHandlerFactory<H> {
    /// Creates a factory backed by `host` using [`BrowserHandlerConfig::default`].
    #[must_use]
    pub fn new(host: Arc<H>) -> Self {
        Self::with_config(host, BrowserHandlerConfig::default())
    }

    /// Creates a factory backed by `host` with explicit handler limits.
    #[must_use]
    pub fn with_config(host: Arc<H>, config: BrowserHandlerConfig) -> Self {
        Self {
            host,
            config,
            product_version: None,
            next_client_id: AtomicU64::new(1),
        }
    }

    /// Sets the installed product version reported in MCP `initialize` `serverInfo.version`.
    #[must_use]
    pub fn with_product_version(mut self, version: impl AsRef<str>) -> Self {
        self.product_version = normalize_product_version(version.as_ref());
        self
    }

    /// Allocates a fresh [`BrowserHandler`] with its own unique [`ClientId`].
    #[must_use]
    pub fn next_handler(&self) -> BrowserHandler<H> {
        let raw_id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        let mut handler =
            BrowserHandler::with_config(ClientId::new(raw_id), Arc::clone(&self.host), self.config);
        handler.product_version.clone_from(&self.product_version);
        handler
    }
}

#[derive(Debug)]
struct ClientLifecycle<H: BrowserHost> {
    client_id: ClientId,
    host: Arc<H>,
    closed_notified: AtomicBool,
}

impl<H: BrowserHost> ClientLifecycle<H> {
    fn new(client_id: ClientId, host: Arc<H>) -> Self {
        Self {
            client_id,
            host,
            closed_notified: AtomicBool::new(false),
        }
    }

    fn notify_closed_once(&self) {
        if !self.closed_notified.swap(true, Ordering::AcqRel) {
            self.host.client_closed(self.client_id);
        }
    }
}

impl<H: BrowserHost> Drop for ClientLifecycle<H> {
    fn drop(&mut self) {
        self.notify_closed_once();
    }
}

/// Per-connection MCP [`ServerHandler`] that validates tool calls and dispatches typed
/// [`BrowserOperation`]s to [`BrowserHost`].
#[derive(Debug)]
pub struct BrowserHandler<H: BrowserHost> {
    lifecycle: Arc<ClientLifecycle<H>>,
    in_flight: Arc<Semaphore>,
    request_timeout: Duration,
    product_version: Option<Arc<str>>,
}

impl<H: BrowserHost> Clone for BrowserHandler<H> {
    fn clone(&self) -> Self {
        Self {
            lifecycle: Arc::clone(&self.lifecycle),
            in_flight: Arc::clone(&self.in_flight),
            request_timeout: self.request_timeout,
            product_version: self.product_version.clone(),
        }
    }
}

impl<H: BrowserHost> BrowserHandler<H> {
    /// Creates a handler bound to `client_id` using [`BrowserHandlerConfig::default`].
    #[must_use]
    pub fn new(client_id: ClientId, host: Arc<H>) -> Self {
        Self::with_config(client_id, host, BrowserHandlerConfig::default())
    }

    /// Creates a handler bound to `client_id` with explicit timeout and concurrency limits.
    #[must_use]
    pub fn with_config(client_id: ClientId, host: Arc<H>, config: BrowserHandlerConfig) -> Self {
        let max_in_flight = config.max_in_flight.max(1);
        Self {
            lifecycle: Arc::new(ClientLifecycle::new(client_id, host)),
            in_flight: Arc::new(Semaphore::new(max_in_flight)),
            request_timeout: config.request_timeout,
            product_version: None,
        }
    }

    /// Sets the installed product version reported in MCP `initialize` `serverInfo.version`.
    #[must_use]
    pub fn with_product_version(mut self, version: impl AsRef<str>) -> Self {
        self.product_version = normalize_product_version(version.as_ref());
        self
    }

    /// Returns the host-owned [`ClientId`] permanently bound to this handler instance.
    #[must_use]
    pub fn client_id(&self) -> ClientId {
        self.lifecycle.client_id
    }

    async fn dispatch_and_await<T, F>(
        &self,
        context: &RequestContext<RoleServer>,
        build_op: impl FnOnce(HostReply<T>) -> BrowserOperation,
        encode: F,
    ) -> CallToolResult
    where
        F: FnOnce(T) -> CallToolResult,
    {
        if context.ct.is_cancelled() {
            return wire::encode_tool_error(&ToolError::new(
                ToolErrorCode::Cancelled,
                "request cancelled",
            ));
        }

        let Ok(_permit) = Arc::clone(&self.in_flight).try_acquire_owned() else {
            return wire::encode_tool_error(&ToolError::new(
                ToolErrorCode::Busy,
                "too many in-flight requests",
            ));
        };

        let (tx, rx) = oneshot::channel();
        let op = build_op(HostReply::new(tx));
        if let Err(dispatch_err) = self.lifecycle.host.dispatch(self.lifecycle.client_id, op) {
            return wire::encode_tool_error(&ToolError::from(dispatch_err));
        }

        tokio::select! {
            biased;
            () = context.ct.cancelled() => {
                wire::encode_tool_error(&ToolError::new(
                    ToolErrorCode::Cancelled,
                    "request cancelled",
                ))
            }
            () = tokio::time::sleep(self.request_timeout) => {
                wire::encode_tool_error(&ToolError::new(
                    ToolErrorCode::TimedOut,
                    "request timed out",
                ))
            }
            reply = rx => match reply {
                Ok(Ok(payload)) => encode(payload),
                Ok(Err(err)) => wire::encode_tool_error(&err),
                Err(_) => wire::encode_tool_error(&ToolError::new(
                    ToolErrorCode::Unavailable,
                    "browser host dropped request",
                )),
            }
        }
    }
}

impl<H: BrowserHost> ServerHandler for BrowserHandler<H> {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::default();
        "quickie".clone_into(&mut config.server_info.name);
        if let Some(version) = &self.product_version {
            version.as_ref().clone_into(&mut config.server_info.version);
        }
        config.capabilities.tools = Some(ToolsCapability::default());
        config
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, RmcpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: catalog::list_tools(),
            ..ListToolsResult::default()
        }))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, RmcpError> {
        let Some(tool_id) = catalog::lookup_tool(request.name.as_ref()) else {
            return Err(RmcpError::invalid_params("unknown tool", None));
        };

        let args = request.arguments.as_ref();
        let result = match tool_id {
            ToolId::BrowserStatus
            | ToolId::DiagnosticRequest
            | ToolId::DiagnosticDetach
            | ToolId::PageSnapshot
            | ToolId::PageEvents => self.call_diagnostic_tool(tool_id, args, &context).await,
            ToolId::AgentContextOpen
            | ToolId::AgentContextList
            | ToolId::AgentContextClose
            | ToolId::AgentNavigate => self.call_agent_context_tool(tool_id, args, &context).await,
            ToolId::AgentLocate
            | ToolId::AgentAct
            | ToolId::AgentWait
            | ToolId::AgentScreenshot => self.call_agent_page_tool(tool_id, args, &context).await,
        };

        Ok(result.into())
    }
}

impl<H: BrowserHost> BrowserHandler<H> {
    async fn call_diagnostic_tool(
        &self,
        tool_id: ToolId,
        args: Option<&JsonObject>,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResult {
        match tool_id {
            ToolId::BrowserStatus => match wire::decode_browser_status_args(args) {
                Ok(()) => {
                    self.dispatch_and_await(
                        context,
                        |reply| BrowserOperation::BrowserStatus { reply },
                        |status| wire::encode_browser_status(&status),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::DiagnosticRequest => match wire::decode_diagnostic_request_args(args) {
                Ok(scope) => {
                    self.dispatch_and_await(
                        context,
                        |reply| BrowserOperation::DiagnosticRequest { scope, reply },
                        |grant| wire::encode_diagnostic_grant(&grant),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::DiagnosticDetach => match wire::decode_diagnostic_detach_args(args) {
                Ok(attachment) => {
                    self.dispatch_and_await(
                        context,
                        |reply| BrowserOperation::DiagnosticDetach { attachment, reply },
                        wire::encode_diagnostic_detach,
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::PageSnapshot => match wire::decode_page_snapshot_args(args) {
                Ok(parsed) => {
                    self.dispatch_and_await(
                        context,
                        |reply| BrowserOperation::PageSnapshot {
                            target: parsed.target,
                            request: parsed.request,
                            reply,
                        },
                        |snapshot| wire::encode_page_snapshot(&snapshot),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::PageEvents => match wire::decode_page_events_args(args) {
                Ok(parsed) => {
                    self.dispatch_and_await(
                        context,
                        |reply| BrowserOperation::PageEvents {
                            target: parsed.target,
                            cursor: parsed.cursor,
                            filter: parsed.filter,
                            limit: parsed.limit,
                            reply,
                        },
                        |events| wire::encode_page_events(&events),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            _ => wire::encode_tool_error(&ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "unsupported diagnostic tool",
            )),
        }
    }

    async fn call_agent_context_tool(
        &self,
        tool_id: ToolId,
        args: Option<&JsonObject>,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResult {
        match tool_id {
            ToolId::AgentContextOpen => match agent::wire::decode_context_open_args(args) {
                Ok(viewport) => {
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::ContextOpen { viewport, reply })
                        },
                        |res| agent::wire::encode_context_open(&res),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::AgentContextList => match agent::wire::decode_context_list_args(args) {
                Ok(()) => {
                    self.dispatch_and_await(
                        context,
                        |reply| BrowserOperation::Agent(AgentOperation::ContextList { reply }),
                        |res| agent::wire::encode_context_list(&res),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::AgentContextClose => match agent::wire::decode_context_close_args(args) {
                Ok(context_id) => {
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::ContextClose {
                                context_id,
                                reply,
                            })
                        },
                        agent::wire::encode_context_close,
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::AgentNavigate => match agent::wire::decode_navigate_args(args) {
                Ok(parsed) => {
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::Navigate {
                                context_id: parsed.context_id,
                                url: parsed.url,
                                wait_stage: parsed.wait_stage,
                                timeout_ms: parsed.timeout_ms,
                                reply,
                            })
                        },
                        |res| agent::wire::encode_navigate(&res),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            _ => wire::encode_tool_error(&ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "unsupported agent context tool",
            )),
        }
    }

    async fn call_agent_page_tool(
        &self,
        tool_id: ToolId,
        args: Option<&JsonObject>,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResult {
        match tool_id {
            ToolId::AgentLocate => match agent::wire::decode_locate_args(args) {
                Ok(parsed) => {
                    let max_results = parsed.max_results;
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::Locate {
                                context_id: parsed.context_id,
                                locator: parsed.locator,
                                max_results,
                                reply,
                            })
                        },
                        move |res| agent::wire::encode_locate(&res, max_results),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::AgentAct => match agent::wire::decode_act_args(args) {
                Ok(parsed) => {
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::Act {
                                context_id: parsed.context_id,
                                action: parsed.action,
                                reply,
                            })
                        },
                        |res| agent::wire::encode_act(&res),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::AgentWait => match agent::wire::decode_wait_args(args) {
                Ok(parsed) => {
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::Wait {
                                context_id: parsed.context_id,
                                condition: parsed.condition,
                                timeout_ms: parsed.timeout_ms,
                                reply,
                            })
                        },
                        agent::wire::encode_wait,
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            ToolId::AgentScreenshot => match agent::wire::decode_screenshot_args(args) {
                Ok(parsed) => {
                    let max_bytes = parsed.max_bytes;
                    self.dispatch_and_await(
                        context,
                        |reply| {
                            BrowserOperation::Agent(AgentOperation::Screenshot {
                                context_id: parsed.context_id,
                                target: parsed.target,
                                max_bytes,
                                reply,
                            })
                        },
                        move |res| agent::wire::encode_screenshot(&res, max_bytes),
                    )
                    .await
                }
                Err(err) => wire::encode_tool_error(&err),
            },
            _ => wire::encode_tool_error(&ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "unsupported agent page tool",
            )),
        }
    }
}

fn normalize_product_version(raw: &str) -> Option<Arc<str>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(Arc::from(trimmed))
    }
}

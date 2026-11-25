//! Native standalone macOS `AppKit` MCP runtime (`NSApplicationActivationPolicy::Prohibited`).
//!
//! Runs exclusively on the main thread when the GUI socket is absent, never opens the `Main`
//! profile or `SQLite` store, never creates or activates any user window, keeps `stdout` strictly
//! reserved for MCP protocol frames, and exits the `NSApplication` loop promptly on `stdin` EOF
//! without idle polling.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType,
};
use objc2_foundation::NSPoint;
use quickie_application::mcp::{ClientId, ObservationTarget};
use quickie_mcp::{
    BrowserHandlerFactory, BrowserHost, BrowserOperation, BrowserStatus, ForkCapabilities,
    HostDispatchError, HostPlatform, IsolatedStdio, McpError, RuntimeMode, ToolError,
    ToolErrorCode, serve_stdio,
};

use super::native::NativeAgentBackend;
use super::{AgentBackend, AgentCoordinator};
use crate::mcp::bridge::{MAX_QUEUED_OPS, is_reply_closed};
use crate::updates::INSTALLED_VERSION;

struct QueuedStandaloneOp {
    client: ClientId,
    operation: BrowserOperation,
}

#[derive(Default)]
struct StandaloneQueueState {
    closed: bool,
    scheduled: bool,
    pending: VecDeque<QueuedStandaloneOp>,
    closed_clients: VecDeque<ClientId>,
}

struct StandaloneMainState {
    coordinator: RefCell<AgentCoordinator>,
    backend: Rc<dyn AgentBackend>,
    on_eof: Box<dyn Fn()>,
}

/// Thread-safe [`BrowserHost`] for standalone stdio mode dispatching onto `DispatchQueue::main()`.
struct MacStandaloneHost {
    queue: Arc<Mutex<StandaloneQueueState>>,
    state: Arc<MainThreadBound<Rc<StandaloneMainState>>>,
}

impl MacStandaloneHost {
    fn new(state: Rc<StandaloneMainState>, mtm: MainThreadMarker) -> Self {
        Self {
            queue: Arc::new(Mutex::new(StandaloneQueueState::default())),
            state: Arc::new(MainThreadBound::new(state, mtm)),
        }
    }

    fn close(&self) {
        let drained = {
            let Ok(mut state) = self.queue.lock() else {
                return;
            };
            state.closed = true;
            state.closed_clients.clear();
            std::mem::take(&mut state.pending)
        };
        drop(drained);
    }

    fn schedule_drain(&self) {
        let queue = Arc::clone(&self.queue);
        let main_state = Arc::clone(&self.state);
        DispatchQueue::main().exec_async(move || {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let (closed_clients, ops) = {
                let Ok(mut state) = queue.lock() else {
                    return;
                };
                state.scheduled = false;
                if state.closed {
                    state.closed_clients.clear();
                    state.pending.clear();
                    return;
                }
                (
                    std::mem::take(&mut state.closed_clients),
                    std::mem::take(&mut state.pending),
                )
            };

            let state = main_state.get(mtm);
            for item in ops {
                if is_reply_closed(&item.operation) {
                    continue;
                }
                dispatch_standalone_operation(
                    &mut state.coordinator.borrow_mut(),
                    item.client,
                    item.operation,
                    state.backend.as_ref(),
                );
            }
            if !closed_clients.is_empty() {
                let mut coordinator = state.coordinator.borrow_mut();
                for client in closed_clients {
                    let _ = coordinator.on_client_closed(client);
                }
                coordinator.shutdown();
                drop(coordinator);
                (state.on_eof)();
            }
        });
    }
}

impl BrowserHost for MacStandaloneHost {
    fn dispatch(
        &self,
        client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError> {
        let should_schedule = {
            let Ok(mut state) = self.queue.lock() else {
                return Err(HostDispatchError::Closed);
            };
            if state.closed {
                return Err(HostDispatchError::Closed);
            }
            if state.pending.len() >= MAX_QUEUED_OPS {
                return Err(HostDispatchError::Full);
            }
            state
                .pending
                .push_back(QueuedStandaloneOp { client, operation });
            !std::mem::replace(&mut state.scheduled, true)
        };
        if should_schedule {
            self.schedule_drain();
        }
        Ok(())
    }

    fn client_closed(&self, client: ClientId) {
        let (should_schedule, dropped) = {
            let Ok(mut state) = self.queue.lock() else {
                return;
            };
            if state.closed {
                return;
            }
            let mut remaining = VecDeque::with_capacity(state.pending.len());
            let mut dropped = Vec::new();
            for item in state.pending.drain(..) {
                if item.client == client {
                    dropped.push(item);
                } else {
                    remaining.push_back(item);
                }
            }
            state.pending = remaining;
            state.closed_clients.push_back(client);
            (!std::mem::replace(&mut state.scheduled, true), dropped)
        };
        drop(dropped);
        if should_schedule {
            self.schedule_drain();
        }
    }
}

/// Builds honest [`BrowserStatus`] for standalone mode from `backend` capabilities.
#[must_use]
pub(crate) fn standalone_browser_status(backend: &dyn AgentBackend) -> BrowserStatus {
    let caps = backend.agent_capabilities();
    let fork_capabilities = if caps.isolated_automation {
        ForkCapabilities {
            diagnostic: caps.observations,
            native_accessibility: caps.observations.accessibility,
            visual_inspector_priority: false,
            agent_automation: true,
        }
    } else {
        ForkCapabilities::default()
    };
    BrowserStatus {
        platform: HostPlatform::Macos,
        runtime_mode: RuntimeMode::Standalone,
        product_version: INSTALLED_VERSION.to_owned(),
        fork_capabilities,
    }
}

/// Dispatches one [`BrowserOperation`] in standalone mode on the coordinator's thread.
///
/// Used by both the live `AppKit` standalone loop and synthetic standalone lifecycle tests.
pub(crate) fn dispatch_standalone_operation(
    coordinator: &mut AgentCoordinator,
    client: ClientId,
    operation: BrowserOperation,
    backend: &dyn AgentBackend,
) {
    match operation {
        BrowserOperation::BrowserStatus { reply } => {
            reply.resolve(standalone_browser_status(backend));
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
        BrowserOperation::PageSnapshot {
            target: ObservationTarget::Diagnostic(_),
            reply,
            ..
        } => {
            reply.reject(ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "live tab diagnostics are unavailable in standalone mode",
            ));
        }
        BrowserOperation::PageEvents {
            target: ObservationTarget::Diagnostic(_),
            reply,
            ..
        } => {
            reply.reject(ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "live tab diagnostics are unavailable in standalone mode",
            ));
        }
        BrowserOperation::PageSnapshot {
            target: ObservationTarget::Agent(context_id),
            request,
            reply,
        } => {
            coordinator.handle_page_snapshot(client, context_id, &request, reply);
        }
        BrowserOperation::PageEvents {
            target: ObservationTarget::Agent(context_id),
            cursor,
            filter,
            limit,
            reply,
        } => {
            coordinator.handle_page_events(client, context_id, cursor, &filter, limit, reply);
        }
        BrowserOperation::Agent(agent_op) => {
            let _ = coordinator.handle_operation(client, agent_op, backend, None);
        }
    }
}

/// Runs the native `AppKit` standalone MCP loop on the main thread with activation prohibited.
///
/// # Errors
///
/// Returns [`McpError`] if starting the stdio server thread fails.
pub(crate) fn run_native_standalone(
    isolated: IsolatedStdio,
    mtm: MainThreadMarker,
) -> Result<(), McpError> {
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);

    let backend: Rc<dyn AgentBackend> = Rc::new(NativeAgentBackend::new(mtm));
    let state = Rc::new(StandaloneMainState {
        coordinator: RefCell::new(AgentCoordinator::new()),
        backend,
        on_eof: Box::new(move || stop_app_run_loop(mtm)),
    });

    let host = Arc::new(MacStandaloneHost::new(Rc::clone(&state), mtm));
    let factory =
        BrowserHandlerFactory::new(Arc::clone(&host)).with_product_version(INSTALLED_VERSION);
    let server = serve_stdio(isolated, factory.next_handler())?;

    app.run();

    host.close();
    state.coordinator.borrow_mut().shutdown();
    server.wait();
    Ok(())
}

fn stop_app_run_loop(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);
    // Post a synthetic application-defined event so `NSApplication::run` wakes up immediately
    // without waiting for user input or any polling timer.
    let wake = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::ZERO,
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        0,
        0,
    );
    if let Some(event) = wake {
        app.postEvent_atStart(&event, true);
    }
}

//! Bounded non-blocking MCP host dispatch port and toolkit-thread controller.

pub(crate) mod controller;
pub(crate) mod protocol;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Mutex;

use quickie_application::mcp::ClientId;
use quickie_mcp::{BrowserHost, BrowserOperation, HostDispatchError, ToolError, ToolErrorCode};

/// Maximum number of queued MCP tool operations waiting for the owning `GLib` main context.
pub(crate) const MAX_PENDING_OPERATIONS: usize = 64;

/// Native main-context wakeup mechanism (`GLib` `g_main_context_invoke_full` on Linux, fake in tests).
pub(crate) trait MainContextWakeup: Send + Sync + 'static {
    /// Schedules one drain pass on the owning main context without blocking the caller.
    fn wake(&self);
}

/// One queued item delivered from the MCP I/O thread to the toolkit main thread.
#[derive(Debug)]
pub(crate) enum HostEvent {
    /// A typed MCP browser tool request.
    Operation {
        client: ClientId,
        operation: BrowserOperation,
    },
    /// Notification that an MCP client disconnected and will send no further operations.
    ClientClosed(ClientId),
}

#[derive(Debug)]
struct QueueState {
    closed: bool,
    wakeup_scheduled: bool,
    operations: VecDeque<(ClientId, BrowserOperation)>,
    closed_clients: VecDeque<ClientId>,
}

/// Thread-safe bounded dispatch port implementing [`BrowserHost`].
///
/// Holds only application-level operations and one-shot reply handles—never toolkit objects.
/// Coalesces native main-context wakeups so bursts of requests schedule at most one pending
/// idle invocation at a time, while ensuring `client_closed` notifications are never dropped
/// even when the operation queue is full.
#[derive(Debug)]
pub(crate) struct DispatchPort<W: MainContextWakeup> {
    capacity: usize,
    wakeup: W,
    state: Mutex<QueueState>,
}

impl<W: MainContextWakeup> DispatchPort<W> {
    /// Creates a bounded dispatch port with the default capacity ([`MAX_PENDING_OPERATIONS`]).
    #[must_use]
    pub(crate) fn new(wakeup: W) -> Self {
        Self::with_capacity(MAX_PENDING_OPERATIONS, wakeup)
    }

    /// Creates a bounded dispatch port with an explicit queue capacity (at least 1).
    #[must_use]
    pub(crate) fn with_capacity(capacity: usize, wakeup: W) -> Self {
        Self {
            capacity: capacity.max(1),
            wakeup,
            state: Mutex::new(QueueState {
                closed: false,
                wakeup_scheduled: false,
                operations: VecDeque::new(),
                closed_clients: VecDeque::new(),
            }),
        }
    }

    /// Drains all queued client-disconnect notifications and operations on the owning main thread.
    ///
    /// Disconnect notifications are drained first so operations from a dead client are skipped
    /// cleanly. Clearing `wakeup_scheduled` under the queue lock guarantees no lost wakeups.
    pub(crate) fn drain(&self) -> Vec<HostEvent> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.wakeup_scheduled = false;
        if guard.closed {
            guard.operations.clear();
            guard.closed_clients.clear();
            return Vec::new();
        }
        let mut events = Vec::with_capacity(
            guard
                .closed_clients
                .len()
                .saturating_add(guard.operations.len()),
        );
        while let Some(client) = guard.closed_clients.pop_front() {
            events.push(HostEvent::ClientClosed(client));
        }
        while let Some((client, operation)) = guard.operations.pop_front() {
            events.push(HostEvent::Operation { client, operation });
        }
        events
    }

    /// Closes the dispatch port during GUI shutdown and completes any queued replies promptly.
    pub(crate) fn close(&self) {
        let pending = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.closed = true;
            guard.wakeup_scheduled = false;
            guard.closed_clients.clear();
            std::mem::take(&mut guard.operations)
        };
        for (_, operation) in pending {
            reject_operation(
                operation,
                ToolError::new(ToolErrorCode::Unavailable, "browser is shutting down"),
            );
        }
    }

    /// Whether the port has been closed for shutdown.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_closed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
    }
}

impl<W: MainContextWakeup> BrowserHost for DispatchPort<W> {
    fn dispatch(
        &self,
        client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError> {
        let should_wake = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.closed {
                return Err(HostDispatchError::Closed);
            }
            if guard.operations.len() >= self.capacity {
                return Err(HostDispatchError::Full);
            }
            guard.operations.push_back((client, operation));
            if guard.wakeup_scheduled {
                false
            } else {
                guard.wakeup_scheduled = true;
                true
            }
        };
        if should_wake {
            self.wakeup.wake();
        }
        Ok(())
    }

    fn client_closed(&self, client: ClientId) {
        let should_wake = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.closed {
                return;
            }
            if !guard.closed_clients.contains(&client) {
                guard.closed_clients.push_back(client);
            }
            if guard.wakeup_scheduled {
                false
            } else {
                guard.wakeup_scheduled = true;
                true
            }
        };
        if should_wake {
            self.wakeup.wake();
        }
    }
}

pub(crate) fn reject_operation(operation: BrowserOperation, error: ToolError) {
    match operation {
        BrowserOperation::BrowserStatus { reply } => reply.reject(error),
        BrowserOperation::DiagnosticRequest { reply, .. } => reply.reject(error),
        BrowserOperation::DiagnosticDetach { reply, .. } => reply.reject(error),
        BrowserOperation::PageSnapshot { reply, .. } => reply.reject(error),
        BrowserOperation::PageEvents { reply, .. } => reply.reject(error),
        BrowserOperation::Agent(operation) => operation.reject(error),
    }
}

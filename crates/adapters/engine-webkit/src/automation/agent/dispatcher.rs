//! Pure bounded in-flight classic Automation request dispatcher and lifecycle state machine.
//!
//! Tracks pending requests by unique non-reusing [`AutomationRequestId`]s, matches out-of-order
//! JSON-RPC responses, routes unsolicited events, and returns owned completions *out* of every
//! mutating method so callers never hold a `RefCell` borrow across a user callback.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Value, json};

use super::protocol::{
    AutomationEvent, CommandResponse, EncodedCommand, MAX_INBOUND_MESSAGE_BYTES, ResponseSpec,
    decode_event, parse_response,
};
use crate::automation::AutomationError;

/// Default hard ceiling on concurrent in-flight classic Automation commands per session.
pub const MAX_IN_FLIGHT_REQUESTS: usize = 16;

/// Unique, strictly monotonic, non-reusing identifier for one in-flight Automation command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AutomationRequestId(u64);

impl AutomationRequestId {
    /// Creates a request identifier from a raw non-zero value.
    #[cfg(test)]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The underlying numeric JSON-RPC request identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AutomationRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "automation-request#{}", self.0)
    }
}

/// Owned completion callback detached from the dispatcher before invocation.
pub(crate) struct PendingCompletion {
    pub(crate) id: AutomationRequestId,
    pub(crate) is_navigation: bool,
    complete: Box<dyn FnOnce(Result<CommandResponse, AutomationError>)>,
}

impl PendingCompletion {
    pub(crate) fn finish(self, outcome: Result<CommandResponse, AutomationError>) {
        (self.complete)(outcome);
    }
}

/// Result of decoding one inbound message from the native session.
pub(crate) enum InboundDispatch {
    /// Matched an in-flight request; invoke `completion.finish(outcome)` after dropping borrows.
    Reply {
        completion: PendingCompletion,
        outcome: Result<CommandResponse, AutomationError>,
    },
    /// Decoded an unsolicited classic Automation event; deliver to observer after dropping borrows.
    Event(AutomationEvent),
    /// Stale/unknown ID, duplicate reply, malformed JSON, passthrough event, or closed session.
    Ignored,
}

struct PendingEntry {
    spec: ResponseSpec,
    is_navigation: bool,
    complete: Box<dyn FnOnce(Result<CommandResponse, AutomationError>)>,
}

/// Pure protocol dispatcher managing request IDs, bounds, out-of-order replies, and lifecycle.
pub(crate) struct Dispatcher {
    next_id: u64,
    max_in_flight: usize,
    closed: bool,
    pending: BTreeMap<AutomationRequestId, PendingEntry>,
}

impl Dispatcher {
    pub(crate) const fn new(max_in_flight: usize) -> Self {
        Self {
            next_id: 0,
            max_in_flight,
            closed: false,
            pending: BTreeMap::new(),
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(crate) fn in_flight_count(&self) -> usize {
        self.pending.len()
    }

    /// Reserves a unique non-reusing request ID, stores the pending completion, and builds the
    /// JSON-RPC wire payload.
    ///
    /// If the transport fails to accept the serialized payload, call [`Self::rollback_dispatch`]
    /// to reclaim the pending slot without reusing the allocated ID.
    pub(crate) fn begin(
        &mut self,
        command: &EncodedCommand,
        is_navigation: bool,
        complete: Box<dyn FnOnce(Result<CommandResponse, AutomationError>)>,
    ) -> Result<(AutomationRequestId, String), AutomationError> {
        if self.closed {
            return Err(AutomationError::Disconnected);
        }
        if self.pending.len() >= self.max_in_flight {
            return Err(AutomationError::Busy);
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or(AutomationError::Transport)?;
        self.next_id = next;
        let id = AutomationRequestId(next);
        let wire = json!({
            "id": next,
            "method": command.method,
            "params": command.params,
        })
        .to_string();
        self.pending.insert(
            id,
            PendingEntry {
                spec: command.response,
                is_navigation,
                complete,
            },
        );
        Ok((id, wire))
    }

    /// Removes a newly reserved entry if the transport rejected `dispatch` synchronously.
    /// The allocated ID is intentionally never reused.
    pub(crate) fn rollback_dispatch(&mut self, id: AutomationRequestId) {
        let _ = self.pending.remove(&id);
    }

    /// Decodes an inbound native message and returns any detached completion or event to invoke
    /// outside the dispatcher's borrow.
    pub(crate) fn receive(&mut self, raw: &str) -> InboundDispatch {
        if self.closed || raw.len() > MAX_INBOUND_MESSAGE_BYTES {
            return InboundDispatch::Ignored;
        }
        let Ok(mut value) = serde_json::from_str::<Value>(raw) else {
            return InboundDispatch::Ignored;
        };
        if let Some(raw_id) = value.get("id").and_then(Value::as_u64) {
            let id = AutomationRequestId(raw_id);
            let Some(entry) = self.pending.remove(&id) else {
                return InboundDispatch::Ignored;
            };
            let outcome = decode_reply_outcome(entry.spec, &mut value);
            return InboundDispatch::Reply {
                completion: PendingCompletion {
                    id,
                    is_navigation: entry.is_navigation,
                    complete: entry.complete,
                },
                outcome,
            };
        }
        decode_event(&value).map_or(InboundDispatch::Ignored, InboundDispatch::Event)
    }

    /// Cancels a single pending request and returns its owned completion for delivery outside the
    /// borrow. Any late reply for `id` will be ignored.
    pub(crate) fn cancel(&mut self, id: AutomationRequestId) -> Option<PendingCompletion> {
        self.take_pending(id)
    }

    /// Times out a single pending request and returns its owned completion for delivery outside
    /// the borrow. Other in-flight requests remain active; any late reply for `id` will be ignored.
    pub(crate) fn timeout(&mut self, id: AutomationRequestId) -> Option<PendingCompletion> {
        self.take_pending(id)
    }

    /// Marks the dispatcher closed and drains all remaining pending completions in ascending ID
    /// order so the caller can complete each with [`AutomationError::Disconnected`] outside the
    /// borrow.
    pub(crate) fn close(&mut self) -> Vec<PendingCompletion> {
        self.closed = true;
        let drained = std::mem::take(&mut self.pending);
        drained
            .into_iter()
            .map(|(id, entry)| PendingCompletion {
                id,
                is_navigation: entry.is_navigation,
                complete: entry.complete,
            })
            .collect()
    }

    fn take_pending(&mut self, id: AutomationRequestId) -> Option<PendingCompletion> {
        let entry = self.pending.remove(&id)?;
        Some(PendingCompletion {
            id,
            is_navigation: entry.is_navigation,
            complete: entry.complete,
        })
    }
}

fn decode_reply_outcome(
    spec: ResponseSpec,
    response: &mut Value,
) -> Result<CommandResponse, AutomationError> {
    if let Some(error) = response.get("error") {
        let message = error.get("message").and_then(Value::as_str);
        if message == Some("Timeout") {
            return Err(AutomationError::Timeout);
        }
        return Err(AutomationError::CommandFailed);
    }
    let result = response.get_mut("result");
    if result.as_ref().is_some_and(|value| value.is_object()) {
        let payload = result.map_or(Value::Null, Value::take);
        parse_response(spec, &payload)
    } else {
        Err(AutomationError::InvalidReply)
    }
}

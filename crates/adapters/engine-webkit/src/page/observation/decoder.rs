//! Stateful `WebKit` Inspector observation decoder.
//!
//! Tracks live targets, separates target identifiers from protocol request identifiers, correlates
//! pending and terminal HTTP(S) network phases (including redirects) within a bounded map,
//! redacts sensitive URL and header data via [`NetworkEvent`], filters disallowed command output
//! and stale targets, and propagates native or local overflow as explicit [`DiagnosticGapReason`]
//! signals.

use std::collections::{HashMap, HashSet, VecDeque};

use quickie_application::mcp::{
    ConsoleLevel, DiagnosticEventData, LifecycleEventKind, NetworkEvent, NetworkPhase,
};
use quickie_application::{DiagnosticGapReason, DiagnosticSignal};
use serde_json::{Map, Value};
use url::Url;

use super::protocol::{
    ObserveCommand, decode_local_diagnostic_gap, encode_direct, encode_for_target,
    is_valid_target_id, parse_bounded_object,
};

/// Default maximum number of in-flight network requests tracked simultaneously.
pub const MAX_TRACKED_REQUESTS: usize = 256;

/// Maximum number of live Inspector targets tracked simultaneously.
pub const MAX_ACTIVE_TARGETS: usize = 64;

/// Maximum number of outstanding authorized command IDs remembered for response correlation.
const MAX_PENDING_COMMANDS: usize = 256;

/// Output of decoding a single incoming `WebKit` Inspector protocol message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeOutcome {
    /// Typed diagnostic events and explicit trace-gap signals to deliver to the shell.
    pub signals: Vec<DiagnosticSignal>,
    /// Allowlisted read-only subscription commands to dispatch back to the native channel.
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RequestKey {
    target: Option<String>,
    protocol_id: String,
}

#[derive(Clone, Debug)]
struct TrackedRequest {
    host_id: u64,
    url: Url,
    method: String,
    status: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CommandKey {
    target: Option<String>,
    id: u64,
}

/// Cross-platform decoder for `WebKit` Inspector read-only diagnostic observation streams.
#[derive(Debug)]
pub struct ObservationDecoder {
    next_command_id: u64,
    next_request_id: u64,
    max_tracked_requests: usize,
    active_targets: HashSet<String>,
    provisional_targets: HashSet<String>,
    pending_commands: HashSet<CommandKey>,
    pending_command_order: VecDeque<CommandKey>,
    tracked_requests: HashMap<RequestKey, TrackedRequest>,
    request_order: VecDeque<RequestKey>,
}

impl Default for ObservationDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservationDecoder {
    /// Creates a decoder with the default bounded in-flight request capacity
    /// ([`MAX_TRACKED_REQUESTS`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_tracked_requests(MAX_TRACKED_REQUESTS)
    }

    /// Creates a decoder with a custom bounded in-flight request capacity (clamped to at least 1).
    #[must_use]
    pub fn with_max_tracked_requests(max_tracked_requests: usize) -> Self {
        Self {
            next_command_id: 0,
            next_request_id: 0,
            max_tracked_requests: max_tracked_requests.max(1),
            active_targets: HashSet::new(),
            provisional_targets: HashSet::new(),
            pending_commands: HashSet::new(),
            pending_command_order: VecDeque::new(),
            tracked_requests: HashMap::new(),
            request_order: VecDeque::new(),
        }
    }

    /// Generates the initial direct enable commands (`Network.enable`, `Page.enable`) for a
    /// freshly connected diagnostic channel.
    #[must_use]
    pub fn initial_direct_commands(&mut self) -> Vec<String> {
        ObserveCommand::direct_enables()
            .into_iter()
            .filter_map(|command| self.encode_direct_command(command))
            .collect()
    }

    /// Encodes a single allowlisted direct command and registers its command ID.
    #[must_use]
    pub fn encode_direct_command(&mut self, command: ObserveCommand) -> Option<String> {
        let id = self.allocate_command_id()?;
        let encoded = encode_direct(id, command)?;
        self.remember_command(CommandKey { target: None, id });
        Some(encoded)
    }

    /// Encodes a single allowlisted target-scoped command for an active target and registers both
    /// outer and inner command IDs.
    #[must_use]
    pub fn encode_target_command(
        &mut self,
        target_id: &str,
        command: ObserveCommand,
    ) -> Option<String> {
        if !self.active_targets.contains(target_id) {
            return None;
        }
        let outer_id = self.allocate_command_id()?;
        let inner_id = self.allocate_command_id()?;
        let encoded = encode_for_target(outer_id, target_id, inner_id, command)?;
        self.remember_command(CommandKey {
            target: None,
            id: outer_id,
        });
        self.remember_command(CommandKey {
            target: Some(target_id.to_owned()),
            id: inner_id,
        });
        Some(encoded)
    }

    /// Number of currently tracked in-flight network requests.
    #[must_use]
    pub fn tracked_request_count(&self) -> usize {
        self.tracked_requests.len()
    }

    /// Whether `target_id` is currently an active (non-provisional, non-destroyed) target.
    #[must_use]
    pub fn is_active_target(&self, target_id: &str) -> bool {
        self.active_targets.contains(target_id)
    }

    /// Decodes one bounded incoming Inspector protocol JSON message.
    ///
    /// Accepts both direct `WebKit` Inspector messages and `Target.dispatchMessageFromTarget`
    /// envelopes. Malformed, oversized, disallowed, or stale-target messages produce no events.
    #[must_use]
    pub fn decode_message(&mut self, raw: &str) -> DecodeOutcome {
        let mut outcome = DecodeOutcome::default();
        let Some(object) = parse_bounded_object(raw) else {
            return outcome;
        };

        if let Some(id) = object.get("id").and_then(Value::as_u64) {
            self.complete_command(None, id);
            return outcome;
        }

        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return outcome;
        };
        let params = object.get("params");

        if let Some(gap) = decode_local_diagnostic_gap(method, params) {
            self.handle_native_gap(gap, &mut outcome);
            return outcome;
        }

        match method {
            "Target.targetCreated" => self.handle_target_created(params, &mut outcome),
            "Target.targetDestroyed" => self.handle_target_destroyed(params),
            "Target.didCommitProvisionalTarget" => {
                self.handle_provisional_commit(params, &mut outcome);
            }
            "Target.dispatchMessageFromTarget" => {
                self.handle_target_message(params, &mut outcome);
            }
            _ => self.decode_domain_event(None, method, params, &mut outcome),
        }

        outcome
    }

    /// Resets tracked state when the native channel closes or is invalidated, returning the
    /// explicit [`DiagnosticSignal::Gap`] to report to the shell.
    #[must_use]
    pub fn on_channel_closed(&mut self) -> DiagnosticSignal {
        self.clear_all_state();
        DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed)
    }

    fn handle_native_gap(&mut self, gap: DiagnosticGapReason, outcome: &mut DecodeOutcome) {
        match gap {
            DiagnosticGapReason::NativeQueueOverflow { .. } => {
                // Missing completion events cannot be distinguished from truly pending requests
                // once the native queue drops messages, so drop tracked in-flight entries.
                self.tracked_requests.clear();
                self.request_order.clear();
            }
            DiagnosticGapReason::ChannelClosed => {
                self.clear_all_state();
            }
            DiagnosticGapReason::RequestMapOverflow { .. } => {}
        }
        outcome.signals.push(DiagnosticSignal::Gap(gap));
    }

    fn clear_all_state(&mut self) {
        self.active_targets.clear();
        self.provisional_targets.clear();
        self.pending_commands.clear();
        self.pending_command_order.clear();
        self.tracked_requests.clear();
        self.request_order.clear();
    }

    fn handle_target_created(&mut self, params: Option<&Value>, outcome: &mut DecodeOutcome) {
        let Some(info) = params.and_then(|p| p.get("targetInfo")) else {
            return;
        };
        let Some(target_id) = info.get("targetId").and_then(Value::as_str) else {
            return;
        };
        if !is_valid_target_id(target_id) {
            return;
        }
        let target_type = info.get("type").and_then(Value::as_str);
        if !matches!(target_type, Some("page" | "frame")) {
            return;
        }
        let is_provisional = info
            .get("isProvisional")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_provisional {
            if self.provisional_targets.len() < MAX_ACTIVE_TARGETS {
                self.provisional_targets.insert(target_id.to_owned());
            }
            return;
        }
        self.activate_target(target_id, outcome);
    }

    fn handle_target_destroyed(&mut self, params: Option<&Value>) {
        let Some(target_id) = params
            .and_then(|p| p.get("targetId"))
            .and_then(Value::as_str)
        else {
            return;
        };
        self.retire_target(target_id);
    }

    fn handle_provisional_commit(&mut self, params: Option<&Value>, outcome: &mut DecodeOutcome) {
        let Some(old_id) = params
            .and_then(|p| p.get("oldTargetId"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(new_id) = params
            .and_then(|p| p.get("newTargetId"))
            .and_then(Value::as_str)
        else {
            return;
        };
        self.retire_target(old_id);
        self.provisional_targets.remove(new_id);
        if is_valid_target_id(new_id) {
            self.activate_target(new_id, outcome);
        }
    }

    fn activate_target(&mut self, target_id: &str, outcome: &mut DecodeOutcome) {
        if self.active_targets.contains(target_id) {
            return;
        }
        if self.active_targets.len() >= MAX_ACTIVE_TARGETS {
            return;
        }
        self.active_targets.insert(target_id.to_owned());
        for command in ObserveCommand::target_enables() {
            if let Some(encoded) = self.encode_target_command(target_id, command) {
                outcome.commands.push(encoded);
            }
        }
    }

    fn retire_target(&mut self, target_id: &str) {
        self.active_targets.remove(target_id);
        self.provisional_targets.remove(target_id);
        self.tracked_requests
            .retain(|key, _| key.target.as_deref() != Some(target_id));
        self.request_order
            .retain(|key| key.target.as_deref() != Some(target_id));
        self.pending_commands
            .retain(|key| key.target.as_deref() != Some(target_id));
        self.pending_command_order
            .retain(|key| key.target.as_deref() != Some(target_id));
    }

    fn handle_target_message(&mut self, params: Option<&Value>, outcome: &mut DecodeOutcome) {
        let Some(target_id) = params
            .and_then(|p| p.get("targetId"))
            .and_then(Value::as_str)
        else {
            return;
        };
        if !self.active_targets.contains(target_id) {
            return;
        }
        let Some(inner_raw) = params
            .and_then(|p| p.get("message"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(inner) = parse_bounded_object(inner_raw) else {
            return;
        };
        self.decode_inner_message(target_id, &inner, outcome);
    }

    fn decode_inner_message(
        &mut self,
        target_id: &str,
        inner: &Map<String, Value>,
        outcome: &mut DecodeOutcome,
    ) {
        if let Some(id) = inner.get("id").and_then(Value::as_u64) {
            self.complete_command(Some(target_id), id);
            return;
        }
        let Some(method) = inner.get("method").and_then(Value::as_str) else {
            return;
        };
        self.decode_domain_event(Some(target_id), method, inner.get("params"), outcome);
    }

    fn decode_domain_event(
        &mut self,
        target_id: Option<&str>,
        method: &str,
        params: Option<&Value>,
        outcome: &mut DecodeOutcome,
    ) {
        match method {
            "Console.messageAdded" => {
                if let Some(data) = decode_console_message(params) {
                    outcome.signals.push(DiagnosticSignal::Event(data));
                }
            }
            "Page.domContentEventFired" => {
                outcome
                    .signals
                    .push(DiagnosticSignal::Event(DiagnosticEventData::lifecycle(
                        LifecycleEventKind::DomContentLoaded,
                    )));
            }
            "Page.loadEventFired" => {
                outcome
                    .signals
                    .push(DiagnosticSignal::Event(DiagnosticEventData::lifecycle(
                        LifecycleEventKind::Load,
                    )));
            }
            "Network.requestWillBeSent" => {
                self.handle_request_will_be_sent(target_id, params, outcome);
            }
            "Network.responseReceived" => {
                self.handle_response_received(target_id, params);
            }
            "Network.loadingFinished" => {
                self.handle_request_terminal(target_id, params, NetworkPhase::Completed, outcome);
            }
            "Network.loadingFailed" => {
                self.handle_request_terminal(target_id, params, NetworkPhase::Failed, outcome);
            }
            _ => {}
        }
    }

    fn handle_request_will_be_sent(
        &mut self,
        target_id: Option<&str>,
        params: Option<&Value>,
        outcome: &mut DecodeOutcome,
    ) {
        let Some(protocol_id) = request_id_from(params) else {
            return;
        };
        let key = RequestKey {
            target: target_id.map(str::to_owned),
            protocol_id: protocol_id.to_owned(),
        };

        // Redirects reuse the same protocol requestId and carry `redirectResponse` for the
        // completed previous hop.
        if let Some(redirect) = params.and_then(|p| p.get("redirectResponse"))
            && let Some(previous) = self.remove_tracked_request(&key)
        {
            let status = status_from_response(redirect).or(previous.status);
            if let Some(event) = NetworkEvent::new(
                previous.host_id,
                &previous.url,
                &previous.method,
                status,
                NetworkPhase::Completed,
            ) {
                outcome
                    .signals
                    .push(DiagnosticSignal::Event(DiagnosticEventData::Network(event)));
            }
        } else {
            // If the same protocol requestId appears again without a redirectResponse, replace
            // any stale tracked entry cleanly.
            let _ = self.remove_tracked_request(&key);
        }

        let Some(request) = params.and_then(|p| p.get("request")) else {
            return;
        };
        let Some(url_str) = request.get("url").and_then(Value::as_str) else {
            return;
        };
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return;
        };
        let Ok(url) = Url::parse(url_str) else {
            return;
        };
        let Some(host_id) = self.allocate_request_id() else {
            return;
        };
        let Some(event) = NetworkEvent::new(host_id, &url, method, None, NetworkPhase::Pending)
        else {
            return;
        };

        if self.tracked_requests.len() >= self.max_tracked_requests {
            self.evict_oldest_request(outcome);
        }

        self.tracked_requests.insert(
            key.clone(),
            TrackedRequest {
                host_id,
                url,
                method: event.method().to_owned(),
                status: None,
            },
        );
        self.request_order.push_back(key);
        outcome
            .signals
            .push(DiagnosticSignal::Event(DiagnosticEventData::Network(event)));
    }

    fn handle_response_received(&mut self, target_id: Option<&str>, params: Option<&Value>) {
        let Some(protocol_id) = request_id_from(params) else {
            return;
        };
        let Some(response) = params.and_then(|p| p.get("response")) else {
            return;
        };
        let key = RequestKey {
            target: target_id.map(str::to_owned),
            protocol_id: protocol_id.to_owned(),
        };
        if let Some(tracked) = self.tracked_requests.get_mut(&key)
            && let Some(status) = status_from_response(response)
        {
            tracked.status = Some(status);
        }
    }

    fn handle_request_terminal(
        &mut self,
        target_id: Option<&str>,
        params: Option<&Value>,
        phase: NetworkPhase,
        outcome: &mut DecodeOutcome,
    ) {
        let Some(protocol_id) = request_id_from(params) else {
            return;
        };
        let key = RequestKey {
            target: target_id.map(str::to_owned),
            protocol_id: protocol_id.to_owned(),
        };
        let Some(tracked) = self.remove_tracked_request(&key) else {
            return;
        };
        if let Some(event) = NetworkEvent::new(
            tracked.host_id,
            &tracked.url,
            &tracked.method,
            tracked.status,
            phase,
        ) {
            outcome
                .signals
                .push(DiagnosticSignal::Event(DiagnosticEventData::Network(event)));
        }
    }

    fn remove_tracked_request(&mut self, key: &RequestKey) -> Option<TrackedRequest> {
        let removed = self.tracked_requests.remove(key)?;
        if let Some(pos) = self.request_order.iter().position(|entry| entry == key) {
            self.request_order.remove(pos);
        }
        Some(removed)
    }

    fn evict_oldest_request(&mut self, outcome: &mut DecodeOutcome) {
        while let Some(oldest) = self.request_order.pop_front() {
            if self.tracked_requests.remove(&oldest).is_some() {
                outcome.signals.push(DiagnosticSignal::Gap(
                    DiagnosticGapReason::RequestMapOverflow { evicted: 1 },
                ));
                return;
            }
        }
    }

    fn allocate_command_id(&mut self) -> Option<u64> {
        self.next_command_id = self.next_command_id.checked_add(1)?;
        Some(self.next_command_id)
    }

    fn allocate_request_id(&mut self) -> Option<u64> {
        self.next_request_id = self.next_request_id.checked_add(1)?;
        Some(self.next_request_id)
    }

    fn remember_command(&mut self, key: CommandKey) {
        if self.pending_commands.len() >= MAX_PENDING_COMMANDS
            && let Some(oldest) = self.pending_command_order.pop_front()
        {
            self.pending_commands.remove(&oldest);
        }
        if self.pending_commands.insert(key.clone()) {
            self.pending_command_order.push_back(key);
        }
    }

    fn complete_command(&mut self, target: Option<&str>, id: u64) {
        let key = CommandKey {
            target: target.map(str::to_owned),
            id,
        };
        if self.pending_commands.remove(&key)
            && let Some(pos) = self
                .pending_command_order
                .iter()
                .position(|entry| entry == &key)
        {
            self.pending_command_order.remove(pos);
        }
    }
}

fn decode_console_message(params: Option<&Value>) -> Option<DiagnosticEventData> {
    let message = params?.get("message")?;
    let source = message.get("source").and_then(Value::as_str)?;
    let level = message.get("level").and_then(Value::as_str)?;
    let text = message.get("text").and_then(Value::as_str)?;

    if source == "javascript" && level == "error" {
        return Some(DiagnosticEventData::runtime_error(text));
    }

    let console_level = match level {
        "debug" => ConsoleLevel::Debug,
        "log" | "info" => ConsoleLevel::Info,
        "warning" => ConsoleLevel::Warn,
        "error" => ConsoleLevel::Error,
        _ => return None,
    };
    Some(DiagnosticEventData::console(console_level, text))
}

fn request_id_from(params: Option<&Value>) -> Option<&str> {
    params
        .and_then(|p| p.get("requestId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn status_from_response(response: &Value) -> Option<u16> {
    let raw = response.get("status").and_then(Value::as_u64)?;
    u16::try_from(raw).ok()
}

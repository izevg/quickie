use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use quickie_application::mcp::{
    AgentContextId, ClientId, ConsoleLevel, DiagnosticCapabilities, DiagnosticEventData,
    ObservationTarget, SnapshotKind, SnapshotRequest,
};
use quickie_application::{DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError};
use quickie_domain_browsing::{Profile, WindowId};
use quickie_mcp::browser::{
    AgentAction, AgentElementRef, AgentKey, AgentLocator, AgentMouseButton, AgentViewport,
    AgentWaitCondition, AgentWaitStage, AgentWaitStatus, ElementBounds, SelectOptionBy,
};
use quickie_mcp::{
    BrowserHandlerFactory, BrowserHost, BrowserOperation, HostDispatchError, IsolatedStdio,
    SocketEndpoint, ToolErrorCode, serve_socket, serve_stdio,
};
use serde_json::{Value, json};
use url::Url;

use crate::mcp::agent::policy::{
    AgentCapabilities, AgentEngineError, AgentLocateOutput, AgentLocatedItem, AgentObservedOutput,
    AgentSignals, ApprovalDecision, decode_png_base64, evaluate_action_policy,
    evaluate_navigation_policy,
};
use crate::mcp::agent::standalone::dispatch_standalone_operation;
use crate::mcp::agent::{
    AgentBackend, AgentCoordinator, AgentDispatchEffect, AgentSession, AgentSessionEvent,
    MouseActionRequest,
};
use crate::updates::INSTALLED_VERSION;

type SignalCallback = Box<dyn Fn(DiagnosticSignal) + 'static>;

// Minimal valid 1x1 PNG (67 bytes), base64-encoded.
const VALID_1X1_PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("valid test URL")
}

fn pipe_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0_i32; 2];
    // SAFETY: `fds` is a valid two-element array for `pipe`.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "pipe() failed");
    // SAFETY: `fds[0]` is a newly allocated valid read end from `pipe`.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: `fds[1]` is a newly allocated valid write end from `pipe`.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    (read_fd, write_fd)
}

struct SyntheticSession {
    connected: Cell<bool>,
    closed_count: Arc<std::sync::atomic::AtomicUsize>,
    current: RefCell<Option<Url>>,
    signal_cb: RefCell<Option<SignalCallback>>,
    located: RefCell<Vec<AgentLocatedItem>>,
    resolved_signals: RefCell<AgentSignals>,
    mouse_calls: Cell<usize>,
}

impl SyntheticSession {
    fn new(closed_count: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        Self {
            connected: Cell::new(true),
            closed_count,
            current: RefCell::new(None),
            signal_cb: RefCell::new(None),
            located: RefCell::new(vec![AgentLocatedItem {
                element_id: "el-1".to_owned(),
                role: "button".to_owned(),
                name: Some("Continue".to_owned()),
                tag: Some("button".to_owned()),
                bounds: Some(ElementBounds {
                    x: 10.0,
                    y: 20.0,
                    width: 80.0,
                    height: 32.0,
                }),
                signals: AgentSignals {
                    connected: true,
                    visible: true,
                    action_point: Some((50.0, 36.0)),
                    ..AgentSignals::default()
                },
            }]),
            resolved_signals: RefCell::new(AgentSignals {
                connected: true,
                visible: true,
                action_point: Some((50.0, 36.0)),
                ..AgentSignals::default()
            }),
            mouse_calls: Cell::new(0),
        }
    }

    fn emit_signal(&self, signal: DiagnosticSignal) {
        if let Some(cb) = self.signal_cb.borrow().as_ref() {
            cb(signal);
        }
    }
}

impl AgentSession for SyntheticSession {
    fn is_connected(&self) -> bool {
        self.connected.get()
    }

    fn set_viewport(&self, _viewport: AgentViewport) {}

    fn current_url(&self) -> Option<Url> {
        self.current.borrow().clone()
    }

    fn set_event_observer(&self, _observer: Option<Box<dyn Fn(AgentSessionEvent) + 'static>>) {}

    fn observe_diagnostics(
        &self,
        on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> Result<DiagnosticLease, EngineError> {
        *self.signal_cb.borrow_mut() = Some(on_signal);
        Ok(DiagnosticLease::new(|| {}))
    }

    fn diagnostic_snapshot(
        &self,
        request: &SnapshotRequest,
        done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
    ) -> Result<(), EngineError> {
        if request.kind() == SnapshotKind::Accessibility {
            return Err(EngineError(
                "native accessibility snapshots are unavailable".to_owned(),
            ));
        }
        done(Ok(DiagnosticSnapshot {
            data: "<body>agent-page</body>".to_owned(),
            truncated: false,
        }));
        Ok(())
    }

    fn navigate(
        &self,
        url: Url,
        approved_origin: url::Origin,
        _wait_stage: AgentWaitStage,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        if url.origin() != approved_origin {
            done(Err(AgentEngineError::OriginMismatch));
            return;
        }
        *self.current.borrow_mut() = Some(url);
        done(Ok(()));
    }

    fn locate_elements(
        &self,
        _locator: &AgentLocator,
        max_results: usize,
        done: Box<dyn FnOnce(Result<AgentLocateOutput, AgentEngineError>) + 'static>,
    ) {
        let items = self.located.borrow();
        let truncated = items.len() > max_results;
        done(Ok(AgentLocateOutput {
            elements: items.iter().take(max_results).cloned().collect(),
            truncated,
        }));
    }

    fn resolve_element(
        &self,
        _element_id: &str,
        _scroll_into_view: bool,
        done: Box<dyn FnOnce(Result<AgentSignals, AgentEngineError>) + 'static>,
    ) {
        done(Ok(self.resolved_signals.borrow().clone()));
    }

    fn inspect_focused(
        &self,
        done: Box<dyn FnOnce(Result<AgentSignals, AgentEngineError>) + 'static>,
    ) {
        done(Ok(self.resolved_signals.borrow().clone()));
    }

    fn mouse_action(
        &self,
        _request: MouseActionRequest,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        self.mouse_calls.set(self.mouse_calls.get() + 1);
        done(Ok(()));
    }

    fn type_text(
        &self,
        _text: String,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        done(Ok(()));
    }

    fn fill_text(
        &self,
        _text: String,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        done(Ok(()));
    }

    fn press_key(
        &self,
        _key: AgentKey,
        _modifiers: Vec<quickie_mcp::browser::AgentKeyModifier>,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        done(Ok(()));
    }

    fn scroll_wheel(
        &self,
        _x: f64,
        _y: f64,
        _delta_x: i32,
        _delta_y: i32,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        done(Ok(()));
    }

    fn select_option(
        &self,
        _element_id: &str,
        _option: &SelectOptionBy,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    ) {
        done(Ok(()));
    }

    fn observe_state(
        &self,
        done: Box<dyn FnOnce(Result<AgentObservedOutput, AgentEngineError>) + 'static>,
    ) {
        done(Ok(AgentObservedOutput {
            focused_role: Some("button".to_owned()),
            value_length: None,
            scroll_x: Some(0),
            scroll_y: Some(0),
        }));
    }

    fn wait_for(
        &self,
        _condition: &AgentWaitCondition,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<AgentWaitStatus, AgentEngineError>) + 'static>,
    ) {
        done(Ok(AgentWaitStatus::Matched));
    }

    fn screenshot_viewport(
        &self,
        _max_bytes: usize,
        _timeout: Duration,
        done: Box<dyn FnOnce(Result<String, AgentEngineError>) + 'static>,
    ) {
        done(Ok(VALID_1X1_PNG_BASE64.to_owned()));
    }

    fn close(&self) {
        if self.connected.replace(false) {
            self.closed_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

struct SyntheticAgentBackend {
    caps: AgentCapabilities,
    closed_count: Arc<std::sync::atomic::AtomicUsize>,
    sessions: RefCell<Vec<Rc<SyntheticSession>>>,
}

impl SyntheticAgentBackend {
    fn enabled() -> Self {
        Self {
            caps: AgentCapabilities {
                isolated_automation: true,
                observations: DiagnosticCapabilities {
                    dom: true,
                    accessibility: false,
                    console: true,
                    runtime_errors: true,
                    pending_network: true,
                    completed_network: true,
                    lifecycle: true,
                },
            },
            closed_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            sessions: RefCell::new(Vec::new()),
        }
    }
}

impl AgentBackend for SyntheticAgentBackend {
    fn agent_capabilities(&self) -> AgentCapabilities {
        self.caps
    }

    fn open_session(&self) -> Result<Rc<dyn AgentSession>, AgentEngineError> {
        let session = Rc::new(SyntheticSession::new(Arc::clone(&self.closed_count)));
        self.sessions.borrow_mut().push(Rc::clone(&session));
        Ok(session)
    }
}

#[test]
fn navigation_policy_requires_explicit_approval_even_for_loopback() {
    // Arrange
    let loopback = url("http://127.0.0.1:8080/control");
    let same_origin_next = url("http://127.0.0.1:8080/next");
    let other_origin = url("https://example.com/");
    let with_creds = url("https://user:pass@example.com/");

    // Act & Assert: no implicit authorization for loopback or any origin.
    assert!(matches!(
        evaluate_navigation_policy(None, &loopback),
        ApprovalDecision::RequiresApproval { .. }
    ));
    let approved = loopback.origin();
    assert_eq!(
        evaluate_navigation_policy(Some(&approved), &same_origin_next),
        ApprovalDecision::Allowed
    );
    assert!(matches!(
        evaluate_navigation_policy(Some(&approved), &other_origin),
        ApprovalDecision::RequiresApproval { .. }
    ));
    assert!(matches!(
        evaluate_navigation_policy(Some(&approved), &with_creds),
        ApprovalDecision::Rejected { .. }
    ));
}

#[test]
fn action_policy_enforces_containment_for_submit_file_input_and_cross_origin_links() {
    // Arrange
    let origin = url("https://example.com/").origin();
    let el = AgentElementRef {
        context_id: AgentContextId::new(1),
        epoch: 2,
        element_id: "el-1".to_owned(),
    };
    let click = AgentAction::Click {
        element: el.clone(),
        button: AgentMouseButton::Left,
        modifiers: Vec::new(),
    };

    // Act & Assert: submit control requires approval.
    let submit_sig = AgentSignals {
        connected: true,
        visible: true,
        is_submit_control: true,
        in_form: true,
        ..AgentSignals::default()
    };
    assert!(matches!(
        evaluate_action_policy(&click, &submit_sig, Some(&origin)),
        ApprovalDecision::RequiresApproval { .. }
    ));

    // File input is rejected.
    let file_sig = AgentSignals {
        connected: true,
        visible: true,
        is_file_input: true,
        ..AgentSignals::default()
    };
    assert!(matches!(
        evaluate_action_policy(&click, &file_sig, Some(&origin)),
        ApprovalDecision::Rejected { .. }
    ));

    // Cross-origin link requires approval; same-origin link is allowed.
    let cross_link = AgentSignals {
        connected: true,
        visible: true,
        link_url: Some("https://evil.example/pay".to_owned()),
        ..AgentSignals::default()
    };
    assert!(matches!(
        evaluate_action_policy(&click, &cross_link, Some(&origin)),
        ApprovalDecision::RequiresApproval { .. }
    ));

    let same_link = AgentSignals {
        connected: true,
        visible: true,
        link_url: Some("https://example.com/docs".to_owned()),
        ..AgentSignals::default()
    };
    assert_eq!(
        evaluate_action_policy(&click, &same_link, Some(&origin)),
        ApprovalDecision::Allowed
    );

    // Enter inside a form requires approval.
    let enter_in_form = AgentAction::PressKey {
        element: Some(el.clone()),
        key: AgentKey::Enter,
        modifiers: Vec::new(),
    };
    let form_input = AgentSignals {
        connected: true,
        visible: true,
        in_form: true,
        is_editable: true,
        ..AgentSignals::default()
    };
    assert!(matches!(
        evaluate_action_policy(&enter_in_form, &form_input, Some(&origin)),
        ApprovalDecision::RequiresApproval { .. }
    ));

    // Non-select rejected for SelectOption.
    let select_op = AgentAction::SelectOption {
        element: el,
        option: SelectOptionBy::Value("a".to_owned()),
    };
    assert!(matches!(
        evaluate_action_policy(&select_op, &form_input, Some(&origin)),
        ApprovalDecision::Rejected { .. }
    ));
}

#[test]
fn png_base64_decoder_validates_signature_dimensions_and_bounds() {
    // Arrange & Act
    let (bytes, w, h) = decode_png_base64(VALID_1X1_PNG_BASE64, 512).expect("valid 1x1 PNG");
    let too_small = decode_png_base64(VALID_1X1_PNG_BASE64, 16);
    let bogus = decode_png_base64("SGVsbG8gd29ybGQ=", 512);

    // Assert
    assert_eq!((w, h), (1, 1));
    assert_eq!(bytes.len(), 68);
    assert_eq!(too_small.unwrap_err().code(), ToolErrorCode::BoundsExceeded);
    assert_eq!(bogus.unwrap_err().code(), ToolErrorCode::Unavailable);
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end multi-client MCP lifecycle test covers open, isolation, approval, epochs, unsupported capabilities, and cleanup"
)]
fn agent_coordinator_enforces_client_isolation_approval_epochs_and_cleanup() {
    // Arrange: synthetic GUI host over a private Unix socket with two independent MCP clients.
    enum GuiMsg {
        Op(ClientId, BrowserOperation),
        Closed(ClientId),
        EmitConsole(String),
        Shutdown,
    }

    struct GuiChannelHost {
        tx: mpsc::Sender<GuiMsg>,
    }

    impl BrowserHost for GuiChannelHost {
        fn dispatch(
            &self,
            client: ClientId,
            operation: BrowserOperation,
        ) -> Result<(), HostDispatchError> {
            self.tx
                .send(GuiMsg::Op(client, operation))
                .map_err(|_| HostDispatchError::Closed)
        }

        fn client_closed(&self, client: ClientId) {
            let _ = self.tx.send(GuiMsg::Closed(client));
        }
    }

    let (tx, rx) = mpsc::channel();
    let tx_ctrl = tx.clone();
    let closed_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closed_for_thread = Arc::clone(&closed_count);
    let approvals_seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let approvals_for_thread = Arc::clone(&approvals_seen);

    let ui_thread = thread::spawn(move || {
        let mut backend = SyntheticAgentBackend::enabled();
        backend.closed_count = closed_for_thread;
        let mut coord = AgentCoordinator::new();
        let window = WindowId::new(Profile::Main, 1);

        while let Ok(msg) = rx.recv() {
            match msg {
                GuiMsg::Op(client, op) => match op {
                    BrowserOperation::Agent(agent_op) => {
                        let effect =
                            coord.handle_operation(client, agent_op, &backend, Some(window));
                        if effect == AgentDispatchEffect::ApprovalChanged
                            && let Some(prompt) = coord.front_approval()
                        {
                            approvals_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let _ = coord.resolve_approval(prompt.id, true);
                        }
                    }
                    BrowserOperation::PageSnapshot {
                        target: ObservationTarget::Agent(ctx_id),
                        request,
                        reply,
                    } => {
                        coord.handle_page_snapshot(client, ctx_id, &request, reply);
                    }
                    BrowserOperation::PageEvents {
                        target: ObservationTarget::Agent(ctx_id),
                        cursor,
                        filter,
                        limit,
                        reply,
                    } => {
                        coord.handle_page_events(client, ctx_id, cursor, &filter, limit, reply);
                    }
                    other => {
                        dispatch_standalone_operation(&mut coord, client, other, &backend);
                    }
                },
                GuiMsg::Closed(client) => {
                    let _ = coord.on_client_closed(client);
                }
                GuiMsg::EmitConsole(text) => {
                    if let Some(session) = backend.sessions.borrow().first() {
                        session.emit_signal(DiagnosticSignal::Event(DiagnosticEventData::console(
                            ConsoleLevel::Info,
                            &text,
                        )));
                    }
                }
                GuiMsg::Shutdown => {
                    coord.shutdown();
                    break;
                }
            }
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let sub = dir.path().join("quickie");
    std::fs::create_dir(&sub).expect("create socket subdir");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("chmod root 0700");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o700))
            .expect("chmod sub 0700");
    }
    let sock_path = sub.join("agent-test.sock");
    let endpoint = SocketEndpoint::in_trusted_root(dir.path(), &sock_path).expect("endpoint");
    let factory = Arc::new(
        BrowserHandlerFactory::new(Arc::new(GuiChannelHost { tx }))
            .with_product_version(INSTALLED_VERSION),
    );
    let factory_clone = Arc::clone(&factory);
    let server =
        serve_socket(&endpoint, move || factory_clone.next_handler()).expect("serve_socket");

    let connect = || -> (UnixStream, BufReader<UnixStream>) {
        let stream = UnixStream::connect(&sock_path).expect("connect unix socket");
        let reader = BufReader::new(stream.try_clone().expect("clone stream"));
        (stream, reader)
    };
    let send = |stream: &mut UnixStream, msg: &Value| {
        writeln!(stream, "{msg}").expect("write json");
        stream.flush().expect("flush json");
    };
    let recv = |reader: &mut BufReader<UnixStream>| -> Value {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read json");
        assert!(n > 0, "unexpected EOF");
        serde_json::from_str(&line).expect("valid json")
    };
    let handshake = |stream: &mut UnixStream, reader: &mut BufReader<UnixStream>| {
        send(
            stream,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0.1.0" }
                }
            }),
        );
        let _ = recv(reader);
        send(
            stream,
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );
    };

    let (mut client_a, mut reader_a) = connect();
    handshake(&mut client_a, &mut reader_a);
    let (mut client_b, mut reader_b) = connect();
    handshake(&mut client_b, &mut reader_b);

    // Act 1: Client A opens a context; Client B cannot close or observe it.
    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "agent_context_open", "arguments": {} }
        }),
    );
    let open_resp = recv(&mut reader_a);
    let ctx_id = open_resp["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("context_id");

    send(
        &mut client_b,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "agent_context_close", "arguments": { "context_id": ctx_id } }
        }),
    );
    let b_close = recv(&mut reader_b);
    assert_eq!(
        b_close["result"]["structuredContent"]["code"],
        "attachment_unavailable"
    );

    // Act 2: Client A navigates (triggers GUI approval -> commits epoch 2), locates ref, then
    // navigates same-origin (no second prompt -> epoch 3), then uses stale epoch-2 ref.
    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "agent_navigate",
                "arguments": { "context_id": ctx_id, "url": "https://example.com/start" }
            }
        }),
    );
    let nav1 = recv(&mut reader_a);
    assert_eq!(nav1["result"]["structuredContent"]["status"], "committed");
    assert_eq!(nav1["result"]["structuredContent"]["epoch"], 2);
    assert_eq!(approvals_seen.load(std::sync::atomic::Ordering::SeqCst), 1);

    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "agent_locate",
                "arguments": {
                    "context_id": ctx_id,
                    "locator": { "kind": "css", "selector": "button" }
                }
            }
        }),
    );
    let loc = recv(&mut reader_a);
    let stale_ref = loc["result"]["structuredContent"]["elements"][0]["element_ref"].clone();
    assert_eq!(stale_ref["epoch"], 2);

    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "agent_navigate",
                "arguments": { "context_id": ctx_id, "url": "https://example.com/next" }
            }
        }),
    );
    let nav2 = recv(&mut reader_a);
    assert_eq!(nav2["result"]["structuredContent"]["epoch"], 3);
    assert_eq!(approvals_seen.load(std::sync::atomic::Ordering::SeqCst), 1);

    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "agent_act",
                "arguments": {
                    "context_id": ctx_id,
                    "action": { "kind": "click", "element": stale_ref }
                }
            }
        }),
    );
    let stale_act = recv(&mut reader_a);
    assert_eq!(
        stale_act["result"]["structuredContent"]["code"],
        "stale_document"
    );

    // Act 3: Unsupported native AX & element screenshot return unsupported_capability; viewport
    // screenshot and page_events work.
    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "page_snapshot",
                "arguments": {
                    "target": { "kind": "agent", "context_id": ctx_id },
                    "kind": "accessibility"
                }
            }
        }),
    );
    let ax_snap = recv(&mut reader_a);
    assert_eq!(
        ax_snap["result"]["structuredContent"]["code"],
        "unsupported_capability"
    );

    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "agent_screenshot",
                "arguments": {
                    "context_id": ctx_id,
                    "target": {
                        "kind": "element",
                        "element": { "context_id": ctx_id, "epoch": 3, "element_id": "el-1" }
                    }
                }
            }
        }),
    );
    let el_shot = recv(&mut reader_a);
    assert_eq!(
        el_shot["result"]["structuredContent"]["code"],
        "unsupported_capability"
    );

    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "agent_screenshot",
                "arguments": { "context_id": ctx_id }
            }
        }),
    );
    let vp_shot = recv(&mut reader_a);
    assert_eq!(vp_shot["result"]["structuredContent"]["width"], 1);
    assert_eq!(vp_shot["result"]["structuredContent"]["height"], 1);

    tx_ctrl
        .send(GuiMsg::EmitConsole("agent-console-ok".to_owned()))
        .expect("emit console");
    send(
        &mut client_a,
        &json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "page_events",
                "arguments": {
                    "target": { "kind": "agent", "context_id": ctx_id },
                    "cursor": 0
                }
            }
        }),
    );
    let events_resp = recv(&mut reader_a);
    let events = events_resp["result"]["structuredContent"]["events"]
        .as_array()
        .expect("events array");
    assert_eq!(events.len(), 1);

    // Act 4: Client A disconnects -> native session is closed; shutdown completes cleanly.
    drop(client_a);
    drop(reader_a);
    drop(client_b);
    drop(reader_b);
    server.shutdown();
    let _ = tx_ctrl.send(GuiMsg::Shutdown);
    ui_thread.join().expect("ui thread joins");

    // Assert
    assert_eq!(closed_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end standalone stdio MCP test covers handshake, capabilities, approval_required, and EOF teardown"
)]
fn standalone_stdio_advertises_version_capabilities_and_surfaces_approval_required_on_eof_cleanup()
{
    // Arrange: synthetic standalone host on its own worker thread driving `dispatch_standalone_operation`.
    enum Msg {
        Op(ClientId, BrowserOperation),
        Closed(ClientId),
    }

    struct ChannelStandaloneHost {
        tx: mpsc::Sender<Msg>,
    }

    impl BrowserHost for ChannelStandaloneHost {
        fn dispatch(
            &self,
            client: ClientId,
            operation: BrowserOperation,
        ) -> Result<(), HostDispatchError> {
            self.tx
                .send(Msg::Op(client, operation))
                .map_err(|_| HostDispatchError::Closed)
        }

        fn client_closed(&self, client: ClientId) {
            let _ = self.tx.send(Msg::Closed(client));
        }
    }

    let (tx, rx) = mpsc::channel();
    let closed_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closed_for_thread = Arc::clone(&closed_count);

    let main_thread = thread::spawn(move || {
        let mut backend = SyntheticAgentBackend::enabled();
        backend.closed_count = closed_for_thread;
        let mut coordinator = AgentCoordinator::new();
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Op(client, op) => {
                    dispatch_standalone_operation(&mut coordinator, client, op, &backend);
                }
                Msg::Closed(client) => {
                    let _ = coordinator.on_client_closed(client);
                    coordinator.shutdown();
                    break;
                }
            }
        }
    });

    let (server_in, client_out) = pipe_pair();
    let (client_in, server_out) = pipe_pair();
    let isolated = IsolatedStdio::from_fds(server_in, server_out);
    let factory = BrowserHandlerFactory::new(Arc::new(ChannelStandaloneHost { tx }))
        .with_product_version(INSTALLED_VERSION);
    let server = serve_stdio(isolated, factory.next_handler()).expect("serve_stdio starts");

    let mut writer = File::from(client_out);
    let mut reader = BufReader::new(File::from(client_in));

    let send = |writer: &mut File, msg: &Value| {
        writeln!(writer, "{msg}").expect("write json line");
        writer.flush().expect("flush json line");
    };
    let recv = |reader: &mut BufReader<File>| -> Value {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read json line");
        assert!(n > 0, "unexpected EOF from standalone MCP server");
        serde_json::from_str(&line).expect("valid JSON-RPC frame")
    };

    // Act 1: initialize + browser_status.
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.1.0" }
            }
        }),
    );
    let init = recv(&mut reader);
    send(
        &mut writer,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );

    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "browser_status", "arguments": {} }
        }),
    );
    let status = recv(&mut reader);

    // Act 2: open isolated context and attempt navigation to loopback (must NOT silently authorize).
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "agent_context_open", "arguments": { "viewport": { "width": 640, "height": 480 } } }
        }),
    );
    let opened = recv(&mut reader);
    let ctx_id = opened["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("context_id");

    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "agent_navigate",
                "arguments": {
                    "context_id": ctx_id,
                    "url": "http://127.0.0.1:8080/fixture"
                }
            }
        }),
    );
    let nav = recv(&mut reader);

    // Close stdin EOF -> server exits and tears down native session.
    drop(writer);
    server.wait();
    main_thread.join().expect("standalone main thread exits");

    // Assert
    assert_eq!(init["result"]["serverInfo"]["version"], INSTALLED_VERSION);
    assert_eq!(
        status["result"]["structuredContent"]["fork_capabilities"]["agent_automation"],
        true
    );
    assert_eq!(
        nav["result"]["structuredContent"]["status"],
        "approval_required"
    );
    assert_eq!(closed_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

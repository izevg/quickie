//! Focused fake-ABI unit tests for isolated `WebKitGTK` local Automation sessions.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString, c_int, c_ulong};
use std::ptr;
use std::rc::Rc;

use url::Url;

use crate::gtk::ffi::{self, Object};

use super::session::{
    AutomationAbi, CreateWebViewCallback, DecidePolicyCallback, DestroyCallback, MessageCallback,
    NavigationRequestInfo, POLICY_DECISION_NAVIGATION_ACTION, POLICY_DECISION_NEW_WINDOW_ACTION,
    POLICY_DECISION_RESPONSE, PermissionRequestCallback, ResponseInfo, WillCloseCallback,
    availability_with,
};
use super::{
    AutomationAvailability, AutomationError, AutomationRequest, AutomationSession,
    CoordinateSystem, DEFAULT_VIEWPORT, InspectorCommand, InspectorObservationMethod, KeyModifier,
    KeyboardInteraction, MouseButton, MouseInteraction, PageLoadStrategy, ViewportSize,
};

const CONTEXT_PTR: usize = 0x1000;
const SESSION_PTR: usize = 0x2000;
const VIEW_PTR: usize = 0x3000;
const DECISION_PTR: usize = 0x4000;
const PERMISSION_PTR: usize = 0x5000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum AbiEvent {
    CreateLocalContext(String),
    GetLocalSession(usize),
    CloseLocalSession(usize),
    SendMessage(usize, String),
    CreateControlledView(usize),
    AllocateView(usize, i32, i32),
    StopLoading(usize),
    RefObject(usize),
    UnrefObject(usize),
    ConnectMessage(usize, c_ulong),
    ConnectWillClose(usize, c_ulong),
    ConnectCreateWebView(usize, c_ulong),
    ConnectDecidePolicy(usize, c_ulong),
    ConnectPermissionRequest(usize, c_ulong),
    DisconnectSignal(usize, c_ulong),
    DestroySignalData(c_ulong),
    UsePolicyDecision(usize),
    IgnorePolicyDecision(usize),
    DenyPermissionRequest(usize),
}

enum SignalSlot {
    Message {
        instance: usize,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    },
    WillClose {
        instance: usize,
        handler: WillCloseCallback,
        data: Object,
        destroy: DestroyCallback,
    },
    CreateWebView {
        instance: usize,
        handler: CreateWebViewCallback,
        data: Object,
        destroy: DestroyCallback,
    },
    DecidePolicy {
        instance: usize,
        handler: DecidePolicyCallback,
        data: Object,
        destroy: DestroyCallback,
    },
    PermissionRequest {
        instance: usize,
        handler: PermissionRequestCallback,
        data: Object,
        destroy: DestroyCallback,
    },
}

impl SignalSlot {
    fn destroy_data(self) {
        let (data, destroy) = match self {
            Self::Message { data, destroy, .. }
            | Self::WillClose { data, destroy, .. }
            | Self::CreateWebView { data, destroy, .. }
            | Self::DecidePolicy { data, destroy, .. }
            | Self::PermissionRequest { data, destroy, .. } => (data, destroy),
        };
        // SAFETY: Reclaims the test closure's `Rc<CallbackState>` just like GObject does.
        unsafe { destroy(data, ptr::null_mut()) };
    }
}

struct FakeState {
    main_thread: bool,
    has_new_context: bool,
    has_get_session: bool,
    has_close_session: bool,
    has_send_message: bool,
    has_message_signal: bool,
    create_context_ok: bool,
    get_session_ok: bool,
    create_view_ok: bool,
    message_signal_ok: bool,
    will_close_signal_ok: bool,
    create_view_signal_ok: bool,
    policy_signal_ok: bool,
    permission_signal_ok: bool,
    send_ok: bool,
    next_handler_id: c_ulong,
    slots: BTreeMap<c_ulong, SignalSlot>,
    refs: HashMap<usize, isize>,
    events: Vec<AbiEvent>,
    nav_info: NavigationRequestInfo,
    resp_info: ResponseInfo,
}

impl FakeState {
    fn ready() -> Self {
        Self {
            main_thread: true,
            has_new_context: true,
            has_get_session: true,
            has_close_session: true,
            has_send_message: true,
            has_message_signal: true,
            create_context_ok: true,
            get_session_ok: true,
            create_view_ok: true,
            message_signal_ok: true,
            will_close_signal_ok: true,
            create_view_signal_ok: true,
            policy_signal_ok: true,
            permission_signal_ok: true,
            send_ok: true,
            next_handler_id: 1,
            slots: BTreeMap::new(),
            refs: HashMap::new(),
            events: Vec::new(),
            nav_info: NavigationRequestInfo {
                url: None,
                method: None,
            },
            resp_info: ResponseInfo {
                url: None,
                mime_supported: true,
                content_disposition: None,
            },
        }
    }

    fn has_complete_spi(&self) -> bool {
        self.has_new_context
            && self.has_get_session
            && self.has_close_session
            && self.has_send_message
            && self.has_message_signal
    }

    fn alloc_handler(&mut self) -> c_ulong {
        let id = self.next_handler_id;
        self.next_handler_id += 1;
        id
    }

    fn adjust_ref(&mut self, addr: usize, delta: isize) {
        *self.refs.entry(addr).or_insert(0) += delta;
    }
}

#[derive(Clone)]
struct FakeAbi {
    state: Rc<RefCell<FakeState>>,
}

impl FakeAbi {
    fn ready() -> Self {
        Self {
            state: Rc::new(RefCell::new(FakeState::ready())),
        }
    }

    fn events(&self) -> Vec<AbiEvent> {
        self.state.borrow().events.clone()
    }

    fn ref_count(&self, addr: usize) -> isize {
        *self.state.borrow().refs.get(&addr).unwrap_or(&0)
    }

    fn emit_message(&self, frame: &str) {
        let targets: Vec<_> = {
            let state = self.state.borrow();
            state
                .slots
                .values()
                .filter_map(|slot| match *slot {
                    SignalSlot::Message {
                        instance,
                        handler,
                        data,
                        ..
                    } => Some((instance, handler, data)),
                    _ => None,
                })
                .collect()
        };
        let payload = CString::new(frame).expect("test frame has no NUL");
        for (instance, handler, data) in targets {
            handler(
                ptr::without_provenance_mut(instance),
                payload.as_ptr(),
                data,
            );
        }
    }

    fn emit_will_close(&self) {
        let targets: Vec<_> = {
            let state = self.state.borrow();
            state
                .slots
                .values()
                .filter_map(|slot| match *slot {
                    SignalSlot::WillClose {
                        instance,
                        handler,
                        data,
                        ..
                    } => Some((instance, handler, data)),
                    _ => None,
                })
                .collect()
        };
        for (instance, handler, data) in targets {
            handler(ptr::without_provenance_mut(instance), data);
        }
    }

    fn emit_create_web_view(&self) -> Vec<Object> {
        let targets: Vec<_> = {
            let state = self.state.borrow();
            state
                .slots
                .values()
                .filter_map(|slot| match *slot {
                    SignalSlot::CreateWebView {
                        instance,
                        handler,
                        data,
                        ..
                    } => Some((instance, handler, data)),
                    _ => None,
                })
                .collect()
        };
        targets
            .into_iter()
            .map(|(instance, handler, data)| handler(ptr::without_provenance_mut(instance), data))
            .collect()
    }

    fn emit_decide_policy(&self, decision_type: c_int) -> Vec<ffi::Bool> {
        let targets: Vec<_> = {
            let state = self.state.borrow();
            state
                .slots
                .values()
                .filter_map(|slot| match *slot {
                    SignalSlot::DecidePolicy {
                        instance,
                        handler,
                        data,
                        ..
                    } => Some((instance, handler, data)),
                    _ => None,
                })
                .collect()
        };
        targets
            .into_iter()
            .map(|(instance, handler, data)| {
                handler(
                    ptr::without_provenance_mut(instance),
                    ptr::without_provenance_mut(DECISION_PTR),
                    decision_type,
                    data,
                )
            })
            .collect()
    }

    fn emit_permission_request(&self) -> Vec<ffi::Bool> {
        let targets: Vec<_> = {
            let state = self.state.borrow();
            state
                .slots
                .values()
                .filter_map(|slot| match *slot {
                    SignalSlot::PermissionRequest {
                        instance,
                        handler,
                        data,
                        ..
                    } => Some((instance, handler, data)),
                    _ => None,
                })
                .collect()
        };
        targets
            .into_iter()
            .map(|(instance, handler, data)| {
                handler(
                    ptr::without_provenance_mut(instance),
                    ptr::without_provenance_mut(PERMISSION_PTR),
                    data,
                )
            })
            .collect()
    }
}

impl AutomationAbi for FakeAbi {
    fn is_main_thread(&self) -> bool {
        self.state.borrow().main_thread
    }

    fn has_local_automation_spi(&self) -> bool {
        self.state.borrow().has_complete_spi()
    }

    fn create_local_context(&self, session_id: &CStr) -> Object {
        let mut state = self.state.borrow_mut();
        let id = session_id.to_string_lossy().into_owned();
        state.events.push(AbiEvent::CreateLocalContext(id));
        if !state.has_complete_spi() || !state.create_context_ok {
            return ptr::null_mut();
        }
        state.adjust_ref(CONTEXT_PTR, 1);
        ptr::without_provenance_mut(CONTEXT_PTR)
    }

    fn local_automation_session(&self, context: Object) -> Object {
        let mut state = self.state.borrow_mut();
        state.events.push(AbiEvent::GetLocalSession(context.addr()));
        if !state.has_complete_spi() || !state.get_session_ok {
            return ptr::null_mut();
        }
        ptr::without_provenance_mut(SESSION_PTR)
    }

    fn close_local_automation_session(&self, context: Object) {
        let mut state = self.state.borrow_mut();
        state
            .events
            .push(AbiEvent::CloseLocalSession(context.addr()));
    }

    fn send_message(&self, session: Object, message: &CStr) -> bool {
        let mut state = self.state.borrow_mut();
        let payload = message.to_string_lossy().into_owned();
        state
            .events
            .push(AbiEvent::SendMessage(session.addr(), payload));
        state.has_complete_spi() && state.send_ok
    }

    fn create_controlled_view(&self, context: Object) -> Object {
        let mut state = self.state.borrow_mut();
        state
            .events
            .push(AbiEvent::CreateControlledView(context.addr()));
        if !state.create_view_ok {
            return ptr::null_mut();
        }
        state.adjust_ref(VIEW_PTR, 1);
        ptr::without_provenance_mut(VIEW_PTR)
    }

    fn allocate_view(&self, view: Object, width: i32, height: i32) {
        let mut state = self.state.borrow_mut();
        state
            .events
            .push(AbiEvent::AllocateView(view.addr(), width, height));
    }

    fn stop_loading(&self, view: Object) {
        let mut state = self.state.borrow_mut();
        state.events.push(AbiEvent::StopLoading(view.addr()));
    }

    fn ref_object(&self, object: Object) {
        let mut state = self.state.borrow_mut();
        let addr = object.addr();
        state.adjust_ref(addr, 1);
        state.events.push(AbiEvent::RefObject(addr));
    }

    fn unref_object(&self, object: Object) {
        let mut state = self.state.borrow_mut();
        let addr = object.addr();
        state.adjust_ref(addr, -1);
        state.events.push(AbiEvent::UnrefObject(addr));
    }

    fn connect_message_signal(
        &self,
        session: Object,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.message_signal_ok {
            return 0;
        }
        let id = state.alloc_handler();
        state
            .events
            .push(AbiEvent::ConnectMessage(session.addr(), id));
        state.slots.insert(
            id,
            SignalSlot::Message {
                instance: session.addr(),
                handler,
                data,
                destroy,
            },
        );
        id
    }

    fn connect_will_close_signal(
        &self,
        session: Object,
        handler: WillCloseCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.will_close_signal_ok {
            return 0;
        }
        let id = state.alloc_handler();
        state
            .events
            .push(AbiEvent::ConnectWillClose(session.addr(), id));
        state.slots.insert(
            id,
            SignalSlot::WillClose {
                instance: session.addr(),
                handler,
                data,
                destroy,
            },
        );
        id
    }

    fn connect_create_web_view_signal(
        &self,
        session: Object,
        handler: CreateWebViewCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.create_view_signal_ok {
            return 0;
        }
        let id = state.alloc_handler();
        state
            .events
            .push(AbiEvent::ConnectCreateWebView(session.addr(), id));
        state.slots.insert(
            id,
            SignalSlot::CreateWebView {
                instance: session.addr(),
                handler,
                data,
                destroy,
            },
        );
        id
    }

    fn connect_decide_policy_signal(
        &self,
        view: Object,
        handler: DecidePolicyCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.policy_signal_ok {
            return 0;
        }
        let id = state.alloc_handler();
        state
            .events
            .push(AbiEvent::ConnectDecidePolicy(view.addr(), id));
        state.slots.insert(
            id,
            SignalSlot::DecidePolicy {
                instance: view.addr(),
                handler,
                data,
                destroy,
            },
        );
        id
    }

    fn connect_permission_request_signal(
        &self,
        view: Object,
        handler: PermissionRequestCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.permission_signal_ok {
            return 0;
        }
        let id = state.alloc_handler();
        state
            .events
            .push(AbiEvent::ConnectPermissionRequest(view.addr(), id));
        state.slots.insert(
            id,
            SignalSlot::PermissionRequest {
                instance: view.addr(),
                handler,
                data,
                destroy,
            },
        );
        id
    }

    fn disconnect_signal(&self, instance: Object, handler_id: c_ulong) {
        let removed = {
            let mut state = self.state.borrow_mut();
            state
                .events
                .push(AbiEvent::DisconnectSignal(instance.addr(), handler_id));
            state.slots.remove(&handler_id)
        };
        if let Some(slot) = removed {
            slot.destroy_data();
            self.state
                .borrow_mut()
                .events
                .push(AbiEvent::DestroySignalData(handler_id));
        }
    }

    fn navigation_request_info(&self, _decision: Object) -> NavigationRequestInfo {
        self.state.borrow().nav_info.clone()
    }

    fn response_info(&self, _decision: Object) -> ResponseInfo {
        self.state.borrow().resp_info.clone()
    }

    fn use_policy_decision(&self, decision: Object) {
        self.state
            .borrow_mut()
            .events
            .push(AbiEvent::UsePolicyDecision(decision.addr()));
    }

    fn ignore_policy_decision(&self, decision: Object) {
        self.state
            .borrow_mut()
            .events
            .push(AbiEvent::IgnorePolicyDecision(decision.addr()));
    }

    fn deny_permission_request(&self, request: Object) {
        self.state
            .borrow_mut()
            .events
            .push(AbiEvent::DenyPermissionRequest(request.addr()));
    }
}

#[test]
fn availability_requires_main_thread_and_complete_optional_symbol_set() {
    // Arrange
    let abi = FakeAbi::ready();

    // Act & Assert: complete SPI on main thread
    assert_eq!(availability_with(&abi), AutomationAvailability::Available);

    // Off main thread
    abi.state.borrow_mut().main_thread = false;
    assert_eq!(availability_with(&abi), AutomationAvailability::WrongThread);
    abi.state.borrow_mut().main_thread = true;

    // Missing any one optional fork symbol or the message-received signal
    let toggles: [fn(&mut FakeState, bool); 5] = [
        |state, value| state.has_new_context = value,
        |state, value| state.has_get_session = value,
        |state, value| state.has_close_session = value,
        |state, value| state.has_send_message = value,
        |state, value| state.has_message_signal = value,
    ];
    for toggle in toggles {
        toggle(&mut abi.state.borrow_mut(), false);
        assert_eq!(
            availability_with(&abi),
            AutomationAvailability::UnsupportedSpi
        );
        let err =
            AutomationSession::open_with(abi.clone(), "agent-1", DEFAULT_VIEWPORT, |_| {}, || {})
                .err();
        assert_eq!(err, Some(AutomationError::UnsupportedSpi));
        toggle(&mut abi.state.borrow_mut(), true);
    }
}

#[test]
fn open_dispatch_receive_and_drop_enforce_ownership_and_destruction_order() {
    // Arrange
    let abi = FakeAbi::ready();
    let received = Rc::new(RefCell::new(Vec::<String>::new()));
    let received_sink = Rc::clone(&received);

    // Act
    let mut session = AutomationSession::open_with(
        abi.clone(),
        "agent-task-1",
        DEFAULT_VIEWPORT,
        move |frame| received_sink.borrow_mut().push(frame.to_owned()),
        || {},
    )
    .expect("session opens with complete fake SPI");

    assert!(session.is_open());
    assert_eq!(session.session_id(), "agent-task-1");
    assert_eq!(session.viewport(), Some(DEFAULT_VIEWPORT));
    assert_eq!(session.raw_view().map(Object::addr), Some(VIEW_PTR));

    let request = AutomationRequest::get_browsing_contexts(1).expect("valid request");
    session
        .dispatch(&request)
        .expect("bounded request dispatches");
    abi.emit_message(r#"{"id":1,"result":{"contexts":[{"handle":"page-1"}]}}"#);
    drop(session);

    // Assert
    assert_eq!(
        received.borrow().as_slice(),
        &[r#"{"id":1,"result":{"contexts":[{"handle":"page-1"}]}}"#]
    );
    assert_eq!(abi.ref_count(VIEW_PTR), 0);
    assert_eq!(abi.ref_count(CONTEXT_PTR), 0);
    assert_eq!(abi.ref_count(SESSION_PTR), 0);

    let events = abi.events();
    let pos = |target: &AbiEvent| {
        events
            .iter()
            .position(|event| event == target)
            .expect("expected ABI event recorded")
    };
    let alloc_idx = pos(&AbiEvent::AllocateView(VIEW_PTR, 640, 480));
    let stop_idx = pos(&AbiEvent::StopLoading(VIEW_PTR));
    let close_idx = pos(&AbiEvent::CloseLocalSession(CONTEXT_PTR));
    let unref_view = pos(&AbiEvent::UnrefObject(VIEW_PTR));
    let unref_context = pos(&AbiEvent::UnrefObject(CONTEXT_PTR));
    let unref_session = pos(&AbiEvent::UnrefObject(SESSION_PTR));

    assert!(alloc_idx < stop_idx);
    assert!(stop_idx < close_idx);
    assert!(close_idx < unref_view);
    assert!(unref_view < unref_context);
    assert!(unref_context < unref_session);
}

#[test]
fn controlled_view_allocates_bounded_viewport_and_reasserts_before_screenshot() {
    // Arrange
    let abi = FakeAbi::ready();
    let custom = ViewportSize::new(800, 600).expect("valid viewport");
    assert_eq!(
        ViewportSize::new(0, 480),
        Err(AutomationError::InvalidViewport)
    );
    assert_eq!(
        ViewportSize::new(640, 5000),
        Err(AutomationError::InvalidViewport)
    );

    // Act
    let mut session =
        AutomationSession::open_with(abi.clone(), "agent-shot", custom, |_| {}, || {})
            .expect("session opens");
    let shot = AutomationRequest::take_screenshot(2, "page-1", None, None, false, true)
        .expect("valid screenshot request");
    session.dispatch(&shot).expect("screenshot dispatches");

    // Assert
    let allocations: Vec<_> = abi
        .events()
        .into_iter()
        .filter(|event| matches!(event, AbiEvent::AllocateView(..)))
        .collect();
    assert_eq!(
        allocations,
        vec![
            AbiEvent::AllocateView(VIEW_PTR, 800, 600),
            AbiEvent::AllocateView(VIEW_PTR, 800, 600),
        ]
    );
    assert_eq!(abi.emit_create_web_view(), vec![ptr::null_mut()]);
}

#[test]
fn navigation_is_blocked_without_explicit_grant_and_restricted_to_granted_origin() {
    // Arrange
    let abi = FakeAbi::ready();
    let mut session =
        AutomationSession::open_with(abi.clone(), "agent-nav", DEFAULT_VIEWPORT, |_| {}, || {})
            .expect("session opens");
    let approved_url = Url::parse("https://example.com/start").expect("valid url");
    let same_origin_redirect = Url::parse("https://example.com/next").expect("valid url");
    let cross_origin_url = Url::parse("https://evil.example/phish").expect("valid url");

    // Act & Assert: locked before explicit grant
    let nav_req = AutomationRequest::navigate_browsing_context(
        1,
        "page-1",
        &approved_url,
        Some(PageLoadStrategy::Normal),
        Some(5000),
    )
    .expect("valid navigation request");
    assert_eq!(
        session.dispatch(&nav_req),
        Err(AutomationError::NavigationNotApproved)
    );

    abi.state.borrow_mut().nav_info = NavigationRequestInfo {
        url: Some(approved_url.clone()),
        method: Some("GET".into()),
    };
    assert_eq!(
        abi.emit_decide_policy(POLICY_DECISION_NAVIGATION_ACTION),
        vec![ffi::TRUE]
    );
    assert_eq!(
        abi.events().last(),
        Some(&AbiEvent::IgnorePolicyDecision(DECISION_PTR))
    );

    // Grant origin explicitly
    let grant = session
        .grant_navigation(&approved_url)
        .expect("explicit grant succeeds");
    assert_eq!(grant.initial_url(), &approved_url);
    session
        .dispatch(&nav_req)
        .expect("approved navigation dispatches");

    // Same-origin GET redirect allowed
    abi.state.borrow_mut().nav_info = NavigationRequestInfo {
        url: Some(same_origin_redirect.clone()),
        method: Some("GET".into()),
    };
    abi.emit_decide_policy(POLICY_DECISION_NAVIGATION_ACTION);
    assert_eq!(
        abi.events().last(),
        Some(&AbiEvent::UsePolicyDecision(DECISION_PTR))
    );

    // Cross-origin redirect denied
    abi.state.borrow_mut().nav_info = NavigationRequestInfo {
        url: Some(cross_origin_url.clone()),
        method: Some("GET".into()),
    };
    abi.emit_decide_policy(POLICY_DECISION_NAVIGATION_ACTION);
    assert_eq!(
        abi.events().last(),
        Some(&AbiEvent::IgnorePolicyDecision(DECISION_PTR))
    );

    // New window action always denied
    abi.emit_decide_policy(POLICY_DECISION_NEW_WINDOW_ACTION);
    assert_eq!(
        abi.events().last(),
        Some(&AbiEvent::IgnorePolicyDecision(DECISION_PTR))
    );

    // Attachment download response denied even on same origin
    abi.state.borrow_mut().resp_info = ResponseInfo {
        url: Some(same_origin_redirect),
        mime_supported: true,
        content_disposition: Some("attachment; filename=payload.bin".into()),
    };
    abi.emit_decide_policy(POLICY_DECISION_RESPONSE);
    assert_eq!(
        abi.events().last(),
        Some(&AbiEvent::IgnorePolicyDecision(DECISION_PTR))
    );

    // Permission requests always denied
    assert_eq!(abi.emit_permission_request(), vec![ffi::TRUE]);
    assert_eq!(
        abi.events().last(),
        Some(&AbiEvent::DenyPermissionRequest(PERMISSION_PTR))
    );
}

#[test]
fn late_callbacks_and_reentrant_close_do_not_panic_or_deliver_after_close() {
    // Arrange
    let abi = FakeAbi::ready();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let closed_count = Rc::new(Cell::new(0usize));
    let session_slot: Rc<RefCell<Option<AutomationSession<FakeAbi>>>> = Rc::new(RefCell::new(None));

    let seen_cb = Rc::clone(&seen);
    let session_cb = Rc::clone(&session_slot);
    let abi_cb = abi.clone();
    let closed_cb = Rc::clone(&closed_count);

    let session = AutomationSession::open_with(
        abi.clone(),
        "agent-reentrant",
        DEFAULT_VIEWPORT,
        move |frame| {
            seen_cb.borrow_mut().push(frame.to_owned());
            // Reentrant message emission while inside callback must be ignored.
            abi_cb.emit_message(r#"{"id":99,"result":{}}"#);
            // Reentrant close while inside callback must retire safely without RefCell panic.
            if let Some(mut active) = session_cb.borrow_mut().take() {
                active.close();
            }
        },
        move || {
            closed_cb.set(closed_cb.get() + 1);
        },
    )
    .expect("session opens");
    *session_slot.borrow_mut() = Some(session);

    // Act
    abi.emit_message(r#"{"id":1,"result":{}}"#);
    // Late message and will-close after reentrant close must be ignored.
    abi.emit_message(r#"{"id":2,"result":{}}"#);
    abi.emit_will_close();

    // Assert
    assert_eq!(seen.borrow().as_slice(), &[r#"{"id":1,"result":{}}"#]);
    assert_eq!(closed_count.get(), 0);
    assert_eq!(abi.ref_count(VIEW_PTR), 0);
    assert_eq!(abi.ref_count(CONTEXT_PTR), 0);
    assert_eq!(abi.ref_count(SESSION_PTR), 0);
}

#[test]
fn native_will_close_fires_once_and_rejects_subsequent_dispatch() {
    // Arrange
    let abi = FakeAbi::ready();
    let closed_count = Rc::new(Cell::new(0usize));
    let closed_cb = Rc::clone(&closed_count);
    let mut session = AutomationSession::open_with(
        abi.clone(),
        "agent-will-close",
        DEFAULT_VIEWPORT,
        |_| {},
        move || closed_cb.set(closed_cb.get() + 1),
    )
    .expect("session opens");

    // Act
    abi.emit_will_close();
    abi.emit_will_close();
    let request = AutomationRequest::get_browsing_contexts(1).expect("valid request");
    let dispatch_result = session.dispatch(&request);

    // Assert
    assert_eq!(closed_count.get(), 1);
    assert!(!session.is_open());
    assert_eq!(dispatch_result, Err(AutomationError::ClosedSession));
    assert_eq!(abi.ref_count(VIEW_PTR), 0);
    assert_eq!(abi.ref_count(CONTEXT_PTR), 0);
    assert_eq!(abi.ref_count(SESSION_PTR), 0);
}

#[test]
fn partial_open_failure_unwinds_signals_and_gobjects_without_leaks() {
    // Arrange: controlled view creation fails after session signals were connected
    let abi = FakeAbi::ready();
    abi.state.borrow_mut().create_view_ok = false;

    // Act
    let err =
        AutomationSession::open_with(abi.clone(), "agent-unwind", DEFAULT_VIEWPORT, |_| {}, || {})
            .err();

    // Assert
    assert_eq!(err, Some(AutomationError::ControlledViewFailed));
    assert_eq!(abi.ref_count(CONTEXT_PTR), 0);
    assert_eq!(abi.ref_count(SESSION_PTR), 0);
    assert!(abi.state.borrow().slots.is_empty());
}

#[test]
fn typed_request_builders_reject_invalid_inputs_and_emit_bounded_json() {
    // Arrange & Act & Assert
    assert!(AutomationRequest::get_browsing_contexts(0).is_none());
    assert!(AutomationRequest::get_browsing_context(1, "").is_none());
    assert!(AutomationRequest::get_browsing_context(1, "bad\nhandle").is_none());

    let cred_url = Url::parse("https://user:pass@example.com/").expect("valid url");
    assert_eq!(
        AutomationRequest::navigate_browsing_context(1, "page-1", &cred_url, None, None),
        Err(AutomationError::InvalidUrl)
    );
    let file_url = Url::parse("file:///etc/passwd").expect("valid url");
    assert_eq!(
        AutomationRequest::navigate_browsing_context(1, "page-1", &file_url, None, None),
        Err(AutomationError::InvalidUrl)
    );

    let layout = AutomationRequest::compute_element_layout(
        3,
        "page-1",
        "node-1",
        CoordinateSystem::LayoutViewport,
        None,
        true,
    )
    .expect("valid layout request");
    assert!(layout.as_json().contains("Automation.computeElementLayout"));

    let mouse = AutomationRequest::perform_mouse_interaction(
        4,
        "page-1",
        10.0,
        20.0,
        MouseButton::Left,
        MouseInteraction::SingleClick,
        &[KeyModifier::Shift],
    )
    .expect("valid mouse request");
    assert!(
        mouse
            .as_json()
            .contains("Automation.performMouseInteraction")
    );
    assert!(
        AutomationRequest::perform_mouse_interaction(
            4,
            "page-1",
            f64::NAN,
            20.0,
            MouseButton::Left,
            MouseInteraction::SingleClick,
            &[],
        )
        .is_none()
    );

    let keys = AutomationRequest::perform_keyboard_interactions(
        5,
        "page-1",
        &[KeyboardInteraction::InsertByKey {
            text: "hello".into(),
        }],
    )
    .expect("valid keyboard request");
    assert!(
        keys.as_json()
            .contains("Automation.performKeyboardInteractions")
    );

    let inspector =
        InspectorCommand::for_target(6, "target-1", 7, InspectorObservationMethod::ConsoleEnable)
            .expect("valid target inspector command");
    let wrapped = AutomationRequest::send_inspector_message(8, "page-1", &inspector)
        .expect("valid wrapped inspector request");
    assert!(
        wrapped
            .as_json()
            .contains("Automation.sendInspectorMessage")
    );
    assert!(wrapped.as_json().contains("Target.sendMessageToTarget"));

    let wheel = AutomationRequest::perform_wheel_scroll(9, "page-1", 100.0, 200.0, 0, 120)
        .expect("valid wheel scroll request");
    assert!(
        wheel
            .as_json()
            .contains("Automation.performInteractionSequence")
    );
    assert!(wheel.as_json().contains(r#""sourceType":"Wheel""#));
    assert!(AutomationRequest::perform_wheel_scroll(9, "page-1", 100.0, 200.0, 0, 0).is_none());
    assert!(AutomationRequest::perform_wheel_scroll(9, "page-1", -1.0, 200.0, 0, 10).is_none());
}

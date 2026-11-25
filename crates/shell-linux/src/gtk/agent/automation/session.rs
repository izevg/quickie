//! Main-thread lifecycle, GObject ownership, and scoped callbacks for local Automation.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_ulong, c_void};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

use url::Url;

use crate::gtk::ffi::{self, Bool, Object};

use super::native::GtkAutomationAbi;
use super::{
    AutomationAvailability, AutomationError, AutomationRequest, NavigationGrant, NavigationPolicy,
    RequestKind, ViewportSize, is_valid_handle,
};

pub(super) const POLICY_DECISION_NAVIGATION_ACTION: c_int = 0;
pub(super) const POLICY_DECISION_NEW_WINDOW_ACTION: c_int = 1;
pub(super) const POLICY_DECISION_RESPONSE: c_int = 2;

/// C ABI callback signature for `WebKitAutomationSession::message-received`.
pub(crate) type MessageCallback = extern "C" fn(Object, *const c_char, Object);

/// C ABI callback signature for `WebKitAutomationSession::will-close`.
pub(crate) type WillCloseCallback = extern "C" fn(Object, Object);

/// C ABI callback signature for `WebKitAutomationSession::create-web-view`.
pub(crate) type CreateWebViewCallback = extern "C" fn(Object, Object) -> Object;

/// C ABI callback signature for `WebKitWebView::decide-policy`.
pub(crate) type DecidePolicyCallback = extern "C" fn(Object, Object, c_int, Object) -> Bool;

/// C ABI callback signature for `WebKitWebView::permission-request`.
pub(crate) type PermissionRequestCallback = extern "C" fn(Object, Object, Object) -> Bool;

/// C ABI destroy-notify callback for owned signal closure state.
pub(crate) type DestroyCallback = unsafe extern "C" fn(Object, Object);

/// Extracted request details from a navigation-action `WebKitPolicyDecision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationRequestInfo {
    pub(crate) url: Option<Url>,
    pub(crate) method: Option<String>,
}

/// Extracted response details from a response `WebKitPolicyDecision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseInfo {
    pub(crate) url: Option<Url>,
    pub(crate) mime_supported: bool,
    pub(crate) content_disposition: Option<String>,
}

/// Native ABI boundary abstracted for deterministic fake-ABI unit tests.
pub(crate) trait AutomationAbi {
    fn is_main_thread(&self) -> bool;
    fn has_local_automation_spi(&self) -> bool;
    fn create_local_context(&self, session_id: &CStr) -> Object;
    fn local_automation_session(&self, context: Object) -> Object;
    fn close_local_automation_session(&self, context: Object);
    fn send_message(&self, session: Object, message: &CStr) -> bool;
    fn create_controlled_view(&self, context: Object) -> Object;
    fn allocate_view(&self, view: Object, width: i32, height: i32);
    fn stop_loading(&self, view: Object);
    fn ref_object(&self, object: Object);
    fn unref_object(&self, object: Object);
    fn connect_message_signal(
        &self,
        session: Object,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn connect_will_close_signal(
        &self,
        session: Object,
        handler: WillCloseCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn connect_create_web_view_signal(
        &self,
        session: Object,
        handler: CreateWebViewCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn connect_decide_policy_signal(
        &self,
        view: Object,
        handler: DecidePolicyCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn connect_permission_request_signal(
        &self,
        view: Object,
        handler: PermissionRequestCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn disconnect_signal(&self, instance: Object, handler_id: c_ulong);
    fn navigation_request_info(&self, decision: Object) -> NavigationRequestInfo;
    fn response_info(&self, decision: Object) -> ResponseInfo;
    fn use_policy_decision(&self, decision: Object);
    fn ignore_policy_decision(&self, decision: Object);
    fn deny_permission_request(&self, request: Object);
}

/// Checks whether the current runtime supports native `WebKitGTK` local Automation.
pub(crate) fn availability() -> AutomationAvailability {
    availability_with(&GtkAutomationAbi)
}

/// Returns `true` only on the GTK main thread when the complete local Automation SPI is loaded.
pub(crate) fn is_available() -> bool {
    availability() == AutomationAvailability::Available
}

pub(super) fn availability_with<A: AutomationAbi>(abi: &A) -> AutomationAvailability {
    if !abi.is_main_thread() {
        return AutomationAvailability::WrongThread;
    }
    if !abi.has_local_automation_spi() {
        return AutomationAvailability::UnsupportedSpi;
    }
    AutomationAvailability::Available
}

struct ActiveResources {
    context: NonNull<c_void>,
    session: NonNull<c_void>,
    view: NonNull<c_void>,
    message_handler: c_ulong,
    will_close_handler: c_ulong,
    create_view_handler: c_ulong,
    policy_handler: c_ulong,
    permission_handler: c_ulong,
    viewport: ViewportSize,
}

struct CallbackState<A: AutomationAbi> {
    abi: A,
    active: Cell<bool>,
    closed: Cell<bool>,
    in_callback: Cell<bool>,
    retire_on_exit: Cell<bool>,
    policy: RefCell<NavigationPolicy>,
    on_message: RefCell<Option<Box<dyn FnMut(&str)>>>,
    on_close: RefCell<Option<Box<dyn FnMut()>>>,
}

impl<A: AutomationAbi> CallbackState<A> {
    fn new(abi: A, on_message: Box<dyn FnMut(&str)>, on_close: Box<dyn FnMut()>) -> Self {
        Self {
            abi,
            active: Cell::new(true),
            closed: Cell::new(false),
            in_callback: Cell::new(false),
            retire_on_exit: Cell::new(false),
            policy: RefCell::new(NavigationPolicy::locked()),
            on_message: RefCell::new(Some(on_message)),
            on_close: RefCell::new(Some(on_close)),
        }
    }

    fn retire(&self) {
        self.active.set(false);
        self.closed.set(true);
        if let Ok(mut policy) = self.policy.try_borrow_mut() {
            policy.revoke();
        }
        if self.in_callback.get() {
            self.retire_on_exit.set(true);
            return;
        }
        self.clear_callbacks();
    }

    fn clear_callbacks(&self) {
        if let Ok(mut slot) = self.on_message.try_borrow_mut() {
            *slot = None;
        }
        if let Ok(mut slot) = self.on_close.try_borrow_mut() {
            *slot = None;
        }
    }
}

struct ReentrancyGuard<'a, A: AutomationAbi>(&'a CallbackState<A>);

impl<A: AutomationAbi> Drop for ReentrancyGuard<'_, A> {
    fn drop(&mut self) {
        self.0.in_callback.set(false);
        if self.0.retire_on_exit.replace(false) {
            self.0.clear_callbacks();
        }
    }
}

/// Main-thread owner of one isolated `WebKitGTK` local Automation context, session, and offscreen
/// controlled `WebKitWebView`.
///
/// `!Send` and `!Sync`: constructed, used, and dropped strictly on the GTK main thread.
/// Never pumps a nested main loop, never starts a periodic timer, and forwards native
/// `message-received` frames directly to the protocol dispatcher callback.
pub(crate) struct AutomationSession<A: AutomationAbi + Clone = GtkAutomationAbi> {
    abi: A,
    session_id: String,
    active: Option<ActiveResources>,
    state: Rc<CallbackState<A>>,
    _main_thread: PhantomData<Rc<()>>,
}

impl AutomationSession<GtkAutomationAbi> {
    /// Opens an isolated native `WebKitGTK` local Automation session with an offscreen
    /// controlled `WebKitWebView` allocated to `viewport`.
    ///
    /// The controlled view starts locked against navigation until an explicit caller grant
    /// is installed via [`Self::grant_navigation`].
    pub(crate) fn open(
        session_id: impl Into<String>,
        viewport: ViewportSize,
        on_message: impl FnMut(&str) + 'static,
        on_close: impl FnMut() + 'static,
    ) -> Result<Self, AutomationError> {
        Self::open_with(GtkAutomationAbi, session_id, viewport, on_message, on_close)
    }
}

impl<A: AutomationAbi + Clone> AutomationSession<A> {
    #[expect(
        clippy::too_many_lines,
        reason = "Sequential GObject creation and reverse-order unwind keep ownership invariants explicit"
    )]
    pub(super) fn open_with(
        abi: A,
        session_id: impl Into<String>,
        viewport: ViewportSize,
        on_message: impl FnMut(&str) + 'static,
        on_close: impl FnMut() + 'static,
    ) -> Result<Self, AutomationError> {
        match availability_with(&abi) {
            AutomationAvailability::Available => {}
            AutomationAvailability::WrongThread => return Err(AutomationError::WrongThread),
            AutomationAvailability::UnsupportedSpi => {
                return Err(AutomationError::UnsupportedSpi);
            }
        }

        let session_id = session_id.into();
        if !is_valid_handle(&session_id) {
            return Err(AutomationError::InvalidSessionId);
        }
        let Ok(c_session_id) = CString::new(session_id.as_str()) else {
            return Err(AutomationError::InvalidSessionId);
        };

        let Some(context) = NonNull::new(abi.create_local_context(&c_session_id)) else {
            return Err(AutomationError::SessionCreationFailed);
        };

        let Some(session) = NonNull::new(abi.local_automation_session(context.as_ptr())) else {
            abi.unref_object(context.as_ptr());
            return Err(AutomationError::SessionCreationFailed);
        };

        // Acquire an owned ref on the session so it outlives context teardown and signal cleanup.
        abi.ref_object(session.as_ptr());

        let state = Rc::new(CallbackState::new(
            abi.clone(),
            Box::new(on_message),
            Box::new(on_close),
        ));

        let message_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let message_handler = abi.connect_message_signal(
            session.as_ptr(),
            handle_automation_message::<A>,
            message_data,
            destroy_callback_state::<A>,
        );
        if message_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(message_data.cast::<CallbackState<A>>()) });
            state.retire();
            abi.close_local_automation_session(context.as_ptr());
            abi.unref_object(context.as_ptr());
            abi.unref_object(session.as_ptr());
            return Err(AutomationError::UnsupportedSpi);
        }

        let will_close_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let will_close_handler = abi.connect_will_close_signal(
            session.as_ptr(),
            handle_session_will_close::<A>,
            will_close_data,
            destroy_callback_state::<A>,
        );
        if will_close_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(will_close_data.cast::<CallbackState<A>>()) });
            state.retire();
            abi.disconnect_signal(session.as_ptr(), message_handler);
            abi.close_local_automation_session(context.as_ptr());
            abi.unref_object(context.as_ptr());
            abi.unref_object(session.as_ptr());
            return Err(AutomationError::UnsupportedSpi);
        }

        let create_view_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let create_view_handler = abi.connect_create_web_view_signal(
            session.as_ptr(),
            handle_create_web_view::<A>,
            create_view_data,
            destroy_callback_state::<A>,
        );
        if create_view_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(create_view_data.cast::<CallbackState<A>>()) });
            state.retire();
            abi.disconnect_signal(session.as_ptr(), message_handler);
            abi.disconnect_signal(session.as_ptr(), will_close_handler);
            abi.close_local_automation_session(context.as_ptr());
            abi.unref_object(context.as_ptr());
            abi.unref_object(session.as_ptr());
            return Err(AutomationError::UnsupportedSpi);
        }

        let Some(view) = NonNull::new(abi.create_controlled_view(context.as_ptr())) else {
            state.retire();
            abi.disconnect_signal(session.as_ptr(), message_handler);
            abi.disconnect_signal(session.as_ptr(), will_close_handler);
            abi.disconnect_signal(session.as_ptr(), create_view_handler);
            abi.close_local_automation_session(context.as_ptr());
            abi.unref_object(context.as_ptr());
            abi.unref_object(session.as_ptr());
            return Err(AutomationError::ControlledViewFailed);
        };

        let policy_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let policy_handler = abi.connect_decide_policy_signal(
            view.as_ptr(),
            handle_decide_policy::<A>,
            policy_data,
            destroy_callback_state::<A>,
        );
        if policy_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(policy_data.cast::<CallbackState<A>>()) });
            state.retire();
            abi.disconnect_signal(session.as_ptr(), message_handler);
            abi.disconnect_signal(session.as_ptr(), will_close_handler);
            abi.disconnect_signal(session.as_ptr(), create_view_handler);
            abi.close_local_automation_session(context.as_ptr());
            abi.unref_object(view.as_ptr());
            abi.unref_object(context.as_ptr());
            abi.unref_object(session.as_ptr());
            return Err(AutomationError::ControlledViewFailed);
        }

        let permission_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let permission_handler = abi.connect_permission_request_signal(
            view.as_ptr(),
            handle_permission_request::<A>,
            permission_data,
            destroy_callback_state::<A>,
        );
        if permission_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(permission_data.cast::<CallbackState<A>>()) });
            state.retire();
            abi.disconnect_signal(view.as_ptr(), policy_handler);
            abi.disconnect_signal(session.as_ptr(), message_handler);
            abi.disconnect_signal(session.as_ptr(), will_close_handler);
            abi.disconnect_signal(session.as_ptr(), create_view_handler);
            abi.close_local_automation_session(context.as_ptr());
            abi.unref_object(view.as_ptr());
            abi.unref_object(context.as_ptr());
            abi.unref_object(session.as_ptr());
            return Err(AutomationError::ControlledViewFailed);
        }

        // Establish a bounded offscreen viewport before any screenshot without showing a window.
        abi.allocate_view(view.as_ptr(), viewport.width(), viewport.height());

        Ok(Self {
            abi,
            session_id,
            active: Some(ActiveResources {
                context,
                session,
                view,
                message_handler,
                will_close_handler,
                create_view_handler,
                policy_handler,
                permission_handler,
                viewport,
            }),
            state,
            _main_thread: PhantomData,
        })
    }

    /// Returns the session identifier bound to this isolated local Automation session.
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the currently allocated offscreen viewport size, if the session is still open.
    pub(crate) fn viewport(&self) -> Option<ViewportSize> {
        self.active.as_ref().map(|resources| resources.viewport)
    }

    /// Returns `true` while the local Automation session is open and able to dispatch requests.
    pub(crate) fn is_open(&self) -> bool {
        self.active.is_some() && self.state.active.get() && !self.state.closed.get()
    }

    /// Installs an explicit caller-approved origin grant for the controlled view.
    ///
    /// Without an active grant, both protocol navigation dispatch and `decide-policy`
    /// reject every load.
    pub(crate) fn grant_navigation(
        &mut self,
        url: &Url,
    ) -> Result<NavigationGrant, AutomationError> {
        if !self.abi.is_main_thread() {
            return Err(AutomationError::WrongThread);
        }
        if !self.is_open() {
            if self.active.is_some() {
                self.close();
            }
            return Err(AutomationError::ClosedSession);
        }
        self.state.policy.borrow_mut().grant_origin(url)
    }

    /// Revokes the active navigation grant and stops any in-flight load on the controlled view.
    pub(crate) fn revoke_navigation(&mut self) {
        self.state.policy.borrow_mut().revoke();
        if let Some(resources) = &self.active {
            self.abi.stop_loading(resources.view.as_ptr());
        }
    }

    /// Re-allocates the unrealized, unmapped controlled view to a new bounded viewport size.
    pub(crate) fn allocate_viewport(
        &mut self,
        viewport: ViewportSize,
    ) -> Result<(), AutomationError> {
        if !self.abi.is_main_thread() {
            return Err(AutomationError::WrongThread);
        }
        if !self.is_open() {
            if self.active.is_some() {
                self.close();
            }
            return Err(AutomationError::ClosedSession);
        }
        let Some(resources) = self.active.as_mut() else {
            return Err(AutomationError::ClosedSession);
        };
        self.abi
            .allocate_view(resources.view.as_ptr(), viewport.width(), viewport.height());
        resources.viewport = viewport;
        Ok(())
    }

    /// Dispatches one engine-generated, structurally bounded Automation protocol request.
    ///
    /// Replies and events arrive asynchronously via the main-loop `on_message` callback.
    pub(crate) fn dispatch(&mut self, request: &AutomationRequest) -> Result<(), AutomationError> {
        if !self.abi.is_main_thread() {
            return Err(AutomationError::WrongThread);
        }
        if !self.is_open() {
            if self.active.is_some() {
                self.close();
            }
            return Err(AutomationError::ClosedSession);
        }
        if let RequestKind::Navigate(url) = request.kind()
            && !self.state.policy.borrow().permits_approved_url(url)
        {
            return Err(AutomationError::NavigationNotApproved);
        }
        let Some(resources) = &self.active else {
            return Err(AutomationError::ClosedSession);
        };
        if matches!(request.kind(), RequestKind::Screenshot) {
            // Re-assert the bounded allocation before a native screenshot capture.
            self.abi.allocate_view(
                resources.view.as_ptr(),
                resources.viewport.width(),
                resources.viewport.height(),
            );
        }
        let Ok(payload) = CString::new(request.as_json()) else {
            return Err(AutomationError::DispatchFailed);
        };
        if self.abi.send_message(resources.session.as_ptr(), &payload) {
            Ok(())
        } else {
            Err(AutomationError::DispatchFailed)
        }
    }

    /// Disconnects and clears callbacks and view, closes the local Automation session on the
    /// context, and releases native GObjects in ownership order: view, context, then session.
    pub(crate) fn close(&mut self) {
        self.state.retire();
        let Some(resources) = self.active.take() else {
            return;
        };
        let view = resources.view.as_ptr();
        let context = resources.context.as_ptr();
        let session = resources.session.as_ptr();

        self.abi.stop_loading(view);

        self.abi.disconnect_signal(view, resources.policy_handler);
        self.abi
            .disconnect_signal(view, resources.permission_handler);

        self.abi
            .disconnect_signal(session, resources.message_handler);
        self.abi
            .disconnect_signal(session, resources.will_close_handler);
        self.abi
            .disconnect_signal(session, resources.create_view_handler);

        self.abi.close_local_automation_session(context);

        self.abi.unref_object(view);
        self.abi.unref_object(context);
        self.abi.unref_object(session);
    }

    /// Returns the raw controlled `WebKitWebView` pointer while the session is open.
    ///
    /// Strictly main-thread-only; used by the GTK MCP host for isolated-world read/locate
    /// scripts and lifecycle signal observation without exposing any `Profile::Main` view.
    pub(crate) fn controlled_view(&self) -> Option<Object> {
        if !self.is_open() {
            return None;
        }
        self.active
            .as_ref()
            .map(|resources| resources.view.as_ptr())
    }

    #[cfg(test)]
    pub(super) fn raw_view(&self) -> Option<Object> {
        self.active
            .as_ref()
            .map(|resources| resources.view.as_ptr())
    }
}

impl<A: AutomationAbi + Clone> Drop for AutomationSession<A> {
    fn drop(&mut self) {
        self.close();
    }
}

fn retain_callback_state<A: AutomationAbi>(data: Object) -> Rc<CallbackState<A>> {
    let raw = data.cast::<CallbackState<A>>();
    // SAFETY: GObject retains a live `Rc<CallbackState<A>>` until `destroy_callback_state` runs.
    unsafe { Rc::increment_strong_count(raw) };
    // SAFETY: Matches the strong-count increment immediately above so `CallbackState<A>` stays
    // alive even if a reentrant callback closes and drops `AutomationSession`.
    unsafe { Rc::from_raw(raw) }
}

extern "C" fn handle_automation_message<A: AutomationAbi>(
    _session: Object,
    message: *const c_char,
    data: Object,
) {
    if data.is_null() || message.is_null() {
        return;
    }
    let state = retain_callback_state::<A>(data);
    if !state.active.get() || state.closed.get() {
        return;
    }
    if state.in_callback.replace(true) {
        return;
    }
    let _guard = ReentrancyGuard(&state);
    // SAFETY: WebKit emits a live, NUL-terminated UTF-8 JSON message for the signal duration.
    let payload = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    if let Ok(mut slot) = state.on_message.try_borrow_mut()
        && let Some(callback) = slot.as_mut()
    {
        callback(payload.as_ref());
    }
}

extern "C" fn handle_session_will_close<A: AutomationAbi>(_session: Object, data: Object) {
    if data.is_null() {
        return;
    }
    let state = retain_callback_state::<A>(data);
    if !state.active.get() || state.closed.replace(true) {
        return;
    }
    state.active.set(false);
    if let Ok(mut policy) = state.policy.try_borrow_mut() {
        policy.revoke();
    }
    if state.in_callback.replace(true) {
        state.retire_on_exit.set(true);
        return;
    }
    let _guard = ReentrancyGuard(&state);
    if let Ok(mut slot) = state.on_close.try_borrow_mut()
        && let Some(callback) = slot.as_mut()
    {
        callback();
    }
    state.retire_on_exit.set(true);
}

extern "C" fn handle_create_web_view<A: AutomationAbi>(_session: Object, _data: Object) -> Object {
    // Reject unsolicited view creation so only the single task-owned controlled view exists.
    ptr::null_mut()
}

extern "C" fn handle_decide_policy<A: AutomationAbi>(
    _view: Object,
    decision: Object,
    decision_type: c_int,
    data: Object,
) -> Bool {
    if data.is_null() || decision.is_null() {
        return ffi::FALSE;
    }
    let state = retain_callback_state::<A>(data);
    if !state.active.get() || state.closed.get() {
        state.abi.ignore_policy_decision(decision);
        return ffi::TRUE;
    }
    let allowed = match decision_type {
        POLICY_DECISION_NAVIGATION_ACTION => {
            let info = state.abi.navigation_request_info(decision);
            state.policy.borrow().permits_navigation(
                info.url.as_ref(),
                info.method.as_deref(),
                false,
            )
        }
        POLICY_DECISION_NEW_WINDOW_ACTION => false,
        POLICY_DECISION_RESPONSE => {
            let info = state.abi.response_info(decision);
            state.policy.borrow().permits_response(
                info.url.as_ref(),
                info.mime_supported,
                info.content_disposition.as_deref(),
            )
        }
        _ => false,
    };
    if allowed {
        state.abi.use_policy_decision(decision);
    } else {
        state.abi.ignore_policy_decision(decision);
    }
    ffi::TRUE
}

extern "C" fn handle_permission_request<A: AutomationAbi>(
    _view: Object,
    request: Object,
    data: Object,
) -> Bool {
    if data.is_null() || request.is_null() {
        return ffi::FALSE;
    }
    let state = retain_callback_state::<A>(data);
    state.abi.deny_permission_request(request);
    ffi::TRUE
}

unsafe extern "C" fn destroy_callback_state<A: AutomationAbi>(data: Object, _closure: Object) {
    if data.is_null() {
        return;
    }
    // SAFETY: Reclaims the single `Rc<CallbackState<A>>` clone handed to `g_signal_connect_data`.
    drop(unsafe { Rc::from_raw(data.cast::<CallbackState<A>>()) });
}

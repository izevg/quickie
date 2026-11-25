//! Main-thread native Inspector observation lease for `WebKitGTK`.
//!
//! Binds at most one local diagnostic observer lease to an existing `WebKitWebView`'s
//! `WebKitWebInspector`, forwards raw protocol frames to the caller for shared decoding,
//! and dispatches only structurally bounded, allowlisted subscription commands.

#![expect(
    dead_code,
    reason = "Wired by the sibling MCP observation transport slice"
)]

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::{CStr, CString, c_char, c_ulong, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use quickie_domain_browsing::TabId;
use serde_json::json;

use crate::gtk::ffi::Object;

use super::native::GtkInspectorAbi;

/// Maximum target identifier length accepted by the fork's local diagnostic policy.
pub(crate) const MAX_TARGET_ID_BYTES: usize = 256;

/// C ABI callback signature for `WebKitWebInspector::diagnostic-message-received`.
pub(crate) type MessageCallback = extern "C" fn(Object, *const c_char, Object);

/// C ABI callback signature for `WebKitWebInspector::closed`.
pub(crate) type ClosedCallback = extern "C" fn(Object, Object);

/// C ABI destroy-notify callback for owned signal closure state.
pub(crate) type DestroyCallback = unsafe extern "C" fn(Object, Object);

thread_local! {
    /// Active native `WebKitWebInspector` pointers on the GTK main thread.
    static ACTIVE_INSPECTORS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// A caller-supplied, validated selected tab and its observation grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectedTabGrant {
    tab: TabId,
    grant: String,
}

impl SelectedTabGrant {
    /// Creates a validated tab grant, rejecting empty or whitespace-only grant tokens.
    pub(crate) fn new(tab: TabId, grant: impl Into<String>) -> Option<Self> {
        let grant = grant.into();
        if grant.trim().is_empty() {
            return None;
        }
        Some(Self { tab, grant })
    }

    pub(crate) const fn tab(&self) -> TabId {
        self.tab
    }

    pub(crate) fn grant(&self) -> &str {
        &self.grant
    }
}

/// Allowlisted direct read-only Inspector subscription methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectSubscription {
    NetworkEnable,
    NetworkDisable,
    PageEnable,
    PageDisable,
}

impl DirectSubscription {
    const fn method(self) -> &'static str {
        match self {
            Self::NetworkEnable => "Network.enable",
            Self::NetworkDisable => "Network.disable",
            Self::PageEnable => "Page.enable",
            Self::PageDisable => "Page.disable",
        }
    }
}

/// Allowlisted target-routed read-only Inspector subscription methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetSubscription {
    ConsoleEnable,
    ConsoleDisable,
    RuntimeEnable,
    RuntimeDisable,
    NetworkEnable,
    NetworkDisable,
    PageEnable,
    PageDisable,
}

impl TargetSubscription {
    const fn method(self) -> &'static str {
        match self {
            Self::ConsoleEnable => "Console.enable",
            Self::ConsoleDisable => "Console.disable",
            Self::RuntimeEnable => "Runtime.enable",
            Self::RuntimeDisable => "Runtime.disable",
            Self::NetworkEnable => "Network.enable",
            Self::NetworkDisable => "Network.disable",
            Self::PageEnable => "Page.enable",
            Self::PageDisable => "Page.disable",
        }
    }
}

/// A validated, generated, structurally bounded read-only observer subscription command.
///
/// Never constructed from arbitrary wire text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObserverCommand {
    payload: String,
}

impl ObserverCommand {
    /// Generates a direct subscription command for the inspected page controller.
    pub(crate) fn direct(id: u64, subscription: DirectSubscription) -> Self {
        let payload = json!({
            "id": id,
            "method": subscription.method(),
        })
        .to_string();
        Self { payload }
    }

    /// Generates a target-routed subscription command after validating `target_id`.
    pub(crate) fn for_target(
        id: u64,
        target_id: &str,
        inner_id: u64,
        subscription: TargetSubscription,
    ) -> Option<Self> {
        if target_id.is_empty()
            || target_id.len() > MAX_TARGET_ID_BYTES
            || target_id
                .bytes()
                .any(|byte| byte == 0 || byte.is_ascii_control())
        {
            return None;
        }
        let inner = json!({
            "id": inner_id,
            "method": subscription.method(),
        })
        .to_string();
        let payload = json!({
            "id": id,
            "method": "Target.sendMessageToTarget",
            "params": {
                "targetId": target_id,
                "message": inner,
            },
        })
        .to_string();
        Some(Self { payload })
    }

    pub(crate) fn as_json(&self) -> &str {
        &self.payload
    }
}

/// Availability of the fork's native local diagnostic observation SPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservationAvailability {
    Available,
    UnsupportedSpi,
    WrongThread,
}

/// Distinct failure modes for native Inspector observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservationError {
    /// The loaded `WebKitGTK` binary does not expose all required fork diagnostic symbols.
    UnsupportedSpi,
    /// The Inspector is occupied by a frontend, another diagnostic client, or an active lease.
    OccupiedInspector,
    /// The inspected page or native diagnostic stream is closed or unavailable.
    ClosedStream,
    /// Observation leases may only be created and driven on the GTK main thread.
    WrongThread,
    /// The native diagnostic channel rejected or failed to dispatch the subscription command.
    DispatchFailed,
}

impl fmt::Display for ObservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSpi => f.write_str("WebKitGTK local diagnostic SPI is unavailable"),
            Self::OccupiedInspector => {
                f.write_str("WebKit Inspector is already occupied by another client")
            }
            Self::ClosedStream => f.write_str("native Inspector observation stream is closed"),
            Self::WrongThread => {
                f.write_str("Inspector observation requires the GTK toolkit main thread")
            }
            Self::DispatchFailed => {
                f.write_str("WebKitGTK failed to dispatch the diagnostic subscription")
            }
        }
    }
}

impl std::error::Error for ObservationError {}

/// Native ABI boundary abstracted for deterministic fake-ABI unit tests.
pub(crate) trait InspectorAbi {
    fn is_main_thread(&self) -> bool;
    fn has_diagnostic_symbols(&self) -> bool;
    fn inspector_for_view(&self, view: Object) -> Object;
    fn ref_inspector(&self, inspector: Object);
    fn unref_inspector(&self, inspector: Object);
    fn connect_message_signal(
        &self,
        inspector: Object,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn connect_closed_signal(
        &self,
        inspector: Object,
        handler: ClosedCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong;
    fn disconnect_signal(&self, inspector: Object, handler_id: c_ulong);
    fn connect_local_diagnostic_client(&self, inspector: Object) -> bool;
    fn send_diagnostic_message(&self, inspector: Object, message: &CStr) -> bool;
    fn disconnect_local_diagnostic_client(&self, inspector: Object);
}

/// Checks whether the current runtime supports native Inspector observation.
pub(crate) fn availability() -> ObservationAvailability {
    availability_with(&GtkInspectorAbi)
}

/// Returns `true` only on the GTK main thread when all required fork symbols are loaded.
pub(crate) fn is_available() -> bool {
    availability() == ObservationAvailability::Available
}

pub(super) fn availability_with<A: InspectorAbi>(abi: &A) -> ObservationAvailability {
    if !abi.is_main_thread() {
        return ObservationAvailability::WrongThread;
    }
    if !abi.has_diagnostic_symbols() {
        return ObservationAvailability::UnsupportedSpi;
    }
    ObservationAvailability::Available
}

struct ActiveBinding {
    inspector: NonNull<c_void>,
    message_handler: c_ulong,
    closed_handler: c_ulong,
}

struct CallbackState {
    active: Cell<bool>,
    closed: Cell<bool>,
    in_callback: Cell<bool>,
    retire_on_exit: Cell<bool>,
    on_frame: RefCell<Option<Box<dyn FnMut(&str)>>>,
    on_close: RefCell<Option<Box<dyn FnMut()>>>,
}

impl CallbackState {
    fn new(on_frame: Box<dyn FnMut(&str)>, on_close: Box<dyn FnMut()>) -> Self {
        Self {
            active: Cell::new(true),
            closed: Cell::new(false),
            in_callback: Cell::new(false),
            retire_on_exit: Cell::new(false),
            on_frame: RefCell::new(Some(on_frame)),
            on_close: RefCell::new(Some(on_close)),
        }
    }

    fn retire(&self) {
        self.active.set(false);
        self.closed.set(true);
        if self.in_callback.get() {
            self.retire_on_exit.set(true);
            return;
        }
        self.clear_callbacks();
    }

    fn clear_callbacks(&self) {
        if let Ok(mut slot) = self.on_frame.try_borrow_mut() {
            *slot = None;
        }
        if let Ok(mut slot) = self.on_close.try_borrow_mut() {
            *slot = None;
        }
    }
}

struct ReentrancyGuard<'a>(&'a CallbackState);

impl Drop for ReentrancyGuard<'_> {
    fn drop(&mut self) {
        self.0.in_callback.set(false);
        if self.0.retire_on_exit.replace(false) {
            self.0.clear_callbacks();
        }
    }
}

/// Main-thread exclusive lease observing one existing `WebKitWebInspector`.
///
/// `!Send` and `!Sync`: constructed, used, and dropped strictly on the GTK main thread.
/// Retains no `WebKitWebView` at any point and releases the inspector upon `stop` or `Drop`.
pub(crate) struct ObservationLease<A: InspectorAbi = GtkInspectorAbi> {
    abi: A,
    target: SelectedTabGrant,
    active: Option<ActiveBinding>,
    state: Rc<CallbackState>,
    _main_thread: PhantomData<Rc<()>>,
}

impl ObservationLease<GtkInspectorAbi> {
    /// Attaches a local diagnostic observation lease to the existing inspector of `view`.
    ///
    /// Never presents or activates a window and never opens the graphical Inspector UI.
    pub(crate) fn attach(
        target: SelectedTabGrant,
        view: Object,
        on_frame: impl FnMut(&str) + 'static,
        on_close: impl FnMut() + 'static,
    ) -> Result<Self, ObservationError> {
        Self::attach_with(GtkInspectorAbi, target, view, on_frame, on_close)
    }
}

impl<A: InspectorAbi> ObservationLease<A> {
    pub(super) fn attach_with(
        abi: A,
        target: SelectedTabGrant,
        view: Object,
        on_frame: impl FnMut(&str) + 'static,
        on_close: impl FnMut() + 'static,
    ) -> Result<Self, ObservationError> {
        match availability_with(&abi) {
            ObservationAvailability::Available => {}
            ObservationAvailability::WrongThread => return Err(ObservationError::WrongThread),
            ObservationAvailability::UnsupportedSpi => {
                return Err(ObservationError::UnsupportedSpi);
            }
        }

        let Some(inspector) = NonNull::new(abi.inspector_for_view(view)) else {
            return Err(ObservationError::ClosedStream);
        };
        let inspector_key = inspector.addr().get();

        let reserved =
            ACTIVE_INSPECTORS.with(|registry| registry.borrow_mut().insert(inspector_key));
        if !reserved {
            return Err(ObservationError::OccupiedInspector);
        }

        abi.ref_inspector(inspector.as_ptr());

        let state = Rc::new(CallbackState::new(Box::new(on_frame), Box::new(on_close)));

        let message_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let message_handler = abi.connect_message_signal(
            inspector.as_ptr(),
            handle_diagnostic_message,
            message_data,
            destroy_callback_state,
        );
        if message_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(message_data.cast::<CallbackState>()) });
            state.retire();
            release_active_inspector(inspector_key);
            abi.unref_inspector(inspector.as_ptr());
            return Err(ObservationError::UnsupportedSpi);
        }

        let closed_data = Rc::into_raw(Rc::clone(&state)).cast_mut().cast();
        let closed_handler = abi.connect_closed_signal(
            inspector.as_ptr(),
            handle_inspector_closed,
            closed_data,
            destroy_callback_state,
        );
        if closed_handler == 0 {
            // SAFETY: Connection failed so GObject will not invoke `destroy_callback_state`.
            drop(unsafe { Rc::from_raw(closed_data.cast::<CallbackState>()) });
            state.retire();
            abi.disconnect_signal(inspector.as_ptr(), message_handler);
            release_active_inspector(inspector_key);
            abi.unref_inspector(inspector.as_ptr());
            return Err(ObservationError::UnsupportedSpi);
        }

        if !abi.connect_local_diagnostic_client(inspector.as_ptr()) {
            state.retire();
            abi.disconnect_signal(inspector.as_ptr(), message_handler);
            abi.disconnect_signal(inspector.as_ptr(), closed_handler);
            release_active_inspector(inspector_key);
            abi.unref_inspector(inspector.as_ptr());
            return Err(ObservationError::OccupiedInspector);
        }

        Ok(Self {
            abi,
            target,
            active: Some(ActiveBinding {
                inspector,
                message_handler,
                closed_handler,
            }),
            state,
            _main_thread: PhantomData,
        })
    }

    /// Returns the validated selected tab and grant bound to this lease.
    pub(crate) const fn target(&self) -> &SelectedTabGrant {
        &self.target
    }

    /// Returns `true` while the native observation stream is still open.
    pub(crate) fn is_open(&self) -> bool {
        self.active.is_some() && self.state.active.get() && !self.state.closed.get()
    }

    /// Dispatches one validated, structurally bounded subscription command.
    pub(crate) fn dispatch_subscription(
        &mut self,
        command: &ObserverCommand,
    ) -> Result<(), ObservationError> {
        if !self.is_open() {
            if self.active.is_some() {
                self.stop();
            }
            return Err(ObservationError::ClosedStream);
        }
        let Some(binding) = &self.active else {
            return Err(ObservationError::ClosedStream);
        };
        let Ok(message) = CString::new(command.as_json()) else {
            return Err(ObservationError::DispatchFailed);
        };
        if self
            .abi
            .send_diagnostic_message(binding.inspector.as_ptr(), &message)
        {
            Ok(())
        } else {
            Err(ObservationError::DispatchFailed)
        }
    }

    /// Stops the native diagnostic connection, disconnects signals, and releases the inspector.
    pub(crate) fn stop(&mut self) {
        self.state.retire();
        let Some(binding) = self.active.take() else {
            return;
        };
        let inspector = binding.inspector.as_ptr();
        self.abi.disconnect_local_diagnostic_client(inspector);
        self.abi
            .disconnect_signal(inspector, binding.message_handler);
        self.abi
            .disconnect_signal(inspector, binding.closed_handler);
        release_active_inspector(binding.inspector.addr().get());
        self.abi.unref_inspector(inspector);
    }
}

impl<A: InspectorAbi> Drop for ObservationLease<A> {
    fn drop(&mut self) {
        self.stop();
    }
}

fn release_active_inspector(inspector_key: usize) {
    ACTIVE_INSPECTORS.with(|registry| {
        registry.borrow_mut().remove(&inspector_key);
    });
}

extern "C" fn handle_diagnostic_message(_inspector: Object, message: *const c_char, data: Object) {
    if data.is_null() || message.is_null() {
        return;
    }
    // SAFETY: GObject retains an `Rc<CallbackState>` clone until `destroy_callback_state` runs.
    let state = unsafe { &*data.cast::<CallbackState>() };
    if !state.active.get() || state.closed.get() {
        return;
    }
    if state.in_callback.replace(true) {
        return;
    }
    let _guard = ReentrancyGuard(state);
    // SAFETY: WebKit emits a live, NUL-terminated UTF-8 string for the duration of this signal.
    let frame = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    if let Ok(mut slot) = state.on_frame.try_borrow_mut()
        && let Some(callback) = slot.as_mut()
    {
        callback(frame.as_ref());
    }
}

extern "C" fn handle_inspector_closed(_inspector: Object, data: Object) {
    if data.is_null() {
        return;
    }
    // SAFETY: GObject retains an `Rc<CallbackState>` clone until `destroy_callback_state` runs.
    let state = unsafe { &*data.cast::<CallbackState>() };
    if !state.active.get() || state.closed.replace(true) {
        return;
    }
    state.active.set(false);
    if state.in_callback.replace(true) {
        state.retire_on_exit.set(true);
        return;
    }
    let _guard = ReentrancyGuard(state);
    if let Ok(mut slot) = state.on_close.try_borrow_mut()
        && let Some(callback) = slot.as_mut()
    {
        callback();
    }
    state.retire_on_exit.set(true);
}

unsafe extern "C" fn destroy_callback_state(data: Object, _closure: Object) {
    if data.is_null() {
        return;
    }
    // SAFETY: Reclaims the single `Rc<CallbackState>` clone handed to `g_signal_connect_data`.
    drop(unsafe { Rc::from_raw(data.cast::<CallbackState>()) });
}

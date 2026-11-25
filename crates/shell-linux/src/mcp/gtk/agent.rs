//! Live `WebKitGTK` isolated Agent backend and standalone `GLib` stdio host.
//!
//! Wires `AgentManager` to `crate::gtk::agent::automation::AutomationSession`, fixed isolated-world
//! `JavaScriptCore` evaluation (`dev.quickie.mcp.agent`), `GLib` one-shot timers, and `web-process-terminated`
//! failure teardown in both GUI and standalone modes.

#![expect(
    unsafe_code,
    reason = "WebKitGTK isolated-world JS evaluation, GLib GMainLoop/GSource, and GTK lazy init FFI"
)]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_ulong};
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;

use quickie_application::mcp::{
    AgentContextId, ClientId, DiagnosticCapabilities, ObservationTarget,
};
use quickie_mcp::browser::{
    AgentKeyModifier, AgentMouseButton, AgentViewport, AgentWaitStage, BrowserHandler,
    BrowserOperation, BrowserStatus, ForkCapabilities, HostPlatform, RuntimeMode, ToolError,
    ToolErrorCode,
};
use quickie_mcp::{IsolatedStdio, serve_stdio};
use url::Url;

use crate::gtk::automation::{
    self, AutomationError, AutomationRequest, AutomationSession, InspectorCommand,
    InspectorObservationMethod, KeyModifier, KeyboardInteraction, MouseButton, MouseInteraction,
    PageLoadStrategy, ViewportSize,
};
use crate::gtk::ffi::{self, Bool, FALSE, Object};
use crate::mcp::agent::protocol::{AutomationWireCommand, WireKeyStep, WireMouseInteraction};
use crate::mcp::agent::{AgentApprovalPrompt, AgentBackend, AgentManager};
use crate::mcp::runtime::protocol::{DecodedSubscription, DirectMethod, TargetMethod};
use crate::mcp::runtime::{DispatchPort, HostEvent};

use super::GlibWakeup;
use super::consent::ConsentView;

#[derive(Clone, Copy)]
enum AgentDispatchTarget {
    Gui,
    Standalone,
}

#[derive(Clone, Copy)]
enum GtkAgentMode {
    Gui { consent: ConsentView },
    Standalone,
}

/// Live GTK `AgentBackend` shared by GUI (`GtkMcpHost`) and standalone (`quickie mcp`).
pub(super) struct GtkAgentBackend {
    mode: GtkAgentMode,
    gtk_ready: Cell<Option<bool>>,
}

impl GtkAgentBackend {
    pub(super) const fn gui(consent: ConsentView) -> Self {
        Self {
            mode: GtkAgentMode::Gui { consent },
            gtk_ready: Cell::new(Some(true)),
        }
    }

    const fn standalone() -> Self {
        Self {
            mode: GtkAgentMode::Standalone,
            gtk_ready: Cell::new(None),
        }
    }

    fn dispatch_target(&self) -> AgentDispatchTarget {
        match self.mode {
            GtkAgentMode::Gui { .. } => AgentDispatchTarget::Gui,
            GtkAgentMode::Standalone => AgentDispatchTarget::Standalone,
        }
    }

    /// Lazily initializes GTK in standalone mode without opening `Profile::Main` or creating any
    /// visible window. Returns `false` cleanly if no Wayland/display compositor is available.
    fn ensure_gtk_ready(&self) -> bool {
        if let Some(ready) = self.gtk_ready.get() {
            return ready;
        }
        // SAFETY: Called on the standalone main thread before creating any offscreen GTK widget.
        // `gtk_init_check` never aborts when no display compositor is present.
        let ok = unsafe { gtk_init_check() } != FALSE;
        self.gtk_ready.set(Some(ok));
        ok
    }
}

/// Retained live `AutomationSession` plus `web-process-terminated` crash watcher.
pub(super) struct GtkAgentSession {
    inner: AutomationSession,
    view_for_crash_signal: Option<Object>,
    crash_handler: c_ulong,
}

impl Drop for GtkAgentSession {
    fn drop(&mut self) {
        if let Some(view) = self.view_for_crash_signal.take()
            && self.crash_handler != 0
        {
            // SAFETY: `view` is still retained by `self.inner` until `self.inner` drops below.
            unsafe { g_signal_handler_disconnect(view, self.crash_handler) };
            self.crash_handler = 0;
        }
    }
}

/// Cancellable isolated-world script evaluation handle for Agent contexts.
pub(super) struct AgentScriptEval {
    cancel: Object,
}

impl Drop for AgentScriptEval {
    fn drop(&mut self) {
        // SAFETY: Cancels any in-flight evaluation and releases the owned GCancellable.
        unsafe {
            g_cancellable_cancel(self.cancel);
            ffi::g_object_unref(self.cancel);
        }
    }
}

/// Cancellable one-shot `GLib` timer handle.
pub(super) struct GlibTimerHandle {
    source_id: Rc<Cell<c_uint>>,
}

impl Drop for GlibTimerHandle {
    fn drop(&mut self) {
        let id = self.source_id.replace(0);
        if id != 0 {
            // SAFETY: Removes the pending GLib timeout source on the owning main thread.
            unsafe { ffi::g_source_remove(id) };
        }
    }
}

impl AgentBackend for GtkAgentBackend {
    type Session = GtkAgentSession;
    type EvalHandle = AgentScriptEval;
    type TimerHandle = GlibTimerHandle;

    fn is_available(&self) -> bool {
        self.ensure_gtk_ready() && automation::is_available()
    }

    fn open_session(
        &mut self,
        context_id: AgentContextId,
        session_id: &str,
        viewport: AgentViewport,
    ) -> Result<Self::Session, ToolError> {
        if !self.ensure_gtk_ready() {
            return Err(ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "isolated agent contexts require an active GTK display backend",
            ));
        }
        let width = i32::try_from(viewport.width).map_err(|_| {
            ToolError::new(ToolErrorCode::BoundsExceeded, "viewport width out of range")
        })?;
        let height = i32::try_from(viewport.height).map_err(|_| {
            ToolError::new(
                ToolErrorCode::BoundsExceeded,
                "viewport height out of range",
            )
        })?;
        let vp = ViewportSize::new(width, height).map_err(map_automation_error)?;
        let target = self.dispatch_target();
        let inner = AutomationSession::open(
            session_id,
            vp,
            move |frame| dispatch_agent_message(target, context_id, frame),
            move || dispatch_agent_closed(target, context_id),
        )
        .map_err(map_automation_error)?;

        let (view_for_crash_signal, crash_handler) = if let Some(view) = inner.controlled_view() {
            let payload = Box::into_raw(Box::new(CrashSignalData { target, context_id })).cast();
            // SAFETY: `view` is a live WebKitWebView owned by `inner`; `destroy_crash_data`
            // reclaims `payload` on disconnect or view destruction.
            let handler = unsafe {
                ffi::g_signal_connect_data(
                    view,
                    c"web-process-terminated".as_ptr(),
                    on_web_process_terminated as *const _,
                    payload,
                    Some(destroy_crash_data),
                    0,
                )
            };
            if handler == 0 {
                // SAFETY: Signal connect failed so GLib will not invoke `destroy_crash_data`.
                drop(unsafe { Box::from_raw(payload.cast::<CrashSignalData>()) });
                (None, 0)
            } else {
                (Some(view), handler)
            }
        } else {
            (None, 0)
        };

        Ok(GtkAgentSession {
            inner,
            view_for_crash_signal,
            crash_handler,
        })
    }

    fn close_session(&mut self, mut session: Self::Session) {
        if let Some(view) = session.view_for_crash_signal.take()
            && session.crash_handler != 0
        {
            // SAFETY: `view` is still owned by `session.inner` before `close()`.
            unsafe { g_signal_handler_disconnect(view, session.crash_handler) };
            session.crash_handler = 0;
        }
        session.inner.close();
    }

    fn grant_navigation(
        &mut self,
        session: &mut Self::Session,
        url: &Url,
    ) -> Result<(), ToolError> {
        session
            .inner
            .grant_navigation(url)
            .map(|_| ())
            .map_err(map_automation_error)
    }

    fn dispatch_command(
        &mut self,
        session: &mut Self::Session,
        request_id: u64,
        command: &AutomationWireCommand,
    ) -> Result<(), ToolError> {
        let request = build_automation_request(request_id, command)?;
        session
            .inner
            .dispatch(&request)
            .map_err(map_automation_error)
    }

    fn start_script(
        &mut self,
        eval_id: u64,
        session: &Self::Session,
        script: &str,
    ) -> Result<Self::EvalHandle, ToolError> {
        let Some(view) = session.inner.controlled_view() else {
            return Err(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
        };
        let c_script = CString::new(script).map_err(|_| {
            ToolError::new(ToolErrorCode::InvalidParams, "script contains NUL byte")
        })?;
        // SAFETY: Allocates an owned GCancellable on the GTK main thread.
        let cancel = unsafe { g_cancellable_new() };
        let payload = Box::into_raw(Box::new(ScriptCallbackData {
            target: self.dispatch_target(),
            eval_id,
        }))
        .cast();
        // SAFETY: WebKit copies `c_script` and invokes `on_script_finished` once on the GTK main
        // context; `payload` is reclaimed inside `on_script_finished`.
        unsafe {
            webkit_web_view_evaluate_javascript(
                view,
                c_script.as_ptr(),
                -1,
                c"dev.quickie.mcp.agent".as_ptr(),
                ptr::null(),
                cancel,
                on_script_finished,
                payload,
            );
        }
        Ok(AgentScriptEval { cancel })
    }

    fn schedule_timer(&mut self, timer_id: u64, delay_ms: u64) -> Self::TimerHandle {
        let millis = u32::try_from(delay_ms.max(1)).unwrap_or(u32::MAX);
        let source_id = Rc::new(Cell::new(0_u32));
        let payload = Box::into_raw(Box::new(TimerCallbackData {
            target: self.dispatch_target(),
            timer_id,
            source_id: Rc::clone(&source_id),
        }))
        .cast();
        // SAFETY: Schedules a one-shot GLib timeout on the default main context; `on_timer_cb`
        // returns FALSE and `destroy_timer_data` reclaims `payload` when removed or fired.
        let id = unsafe { g_timeout_add_full(0, millis, on_timer_cb, payload, destroy_timer_data) };
        source_id.set(id);
        GlibTimerHandle { source_id }
    }

    fn show_approval(&mut self, prompt: AgentApprovalPrompt) -> bool {
        match self.mode {
            GtkAgentMode::Gui { consent } => {
                consent.show_agent_approval(&prompt);
                true
            }
            GtkAgentMode::Standalone => false,
        }
    }

    fn hide_approval(&mut self) {
        if let GtkAgentMode::Gui { consent } = self.mode {
            consent.hide_prompt();
        }
    }
}

fn build_automation_request(
    id: u64,
    command: &AutomationWireCommand,
) -> Result<AutomationRequest, ToolError> {
    let invalid = || ToolError::new(ToolErrorCode::InvalidParams, "invalid automation command");
    match command {
        AutomationWireCommand::GetBrowsingContexts => {
            AutomationRequest::get_browsing_contexts(id).ok_or_else(invalid)
        }
        AutomationWireCommand::Navigate {
            handle,
            url,
            wait_stage,
            timeout_ms,
        } => {
            let strategy = match wait_stage {
                AgentWaitStage::None => PageLoadStrategy::None,
                AgentWaitStage::DomContentLoaded => PageLoadStrategy::Eager,
                AgentWaitStage::Load => PageLoadStrategy::Normal,
            };
            AutomationRequest::navigate_browsing_context(
                id,
                handle,
                url,
                Some(strategy),
                Some(*timeout_ms),
            )
            .map_err(map_automation_error)
        }
        AutomationWireCommand::MouseInteraction {
            handle,
            x,
            y,
            button,
            interaction,
            modifiers,
        } => {
            let btn = match button {
                AgentMouseButton::Left => MouseButton::Left,
                AgentMouseButton::Middle => MouseButton::Middle,
                AgentMouseButton::Right => MouseButton::Right,
            };
            let kind = match interaction {
                WireMouseInteraction::Move => MouseInteraction::Move,
                WireMouseInteraction::SingleClick => MouseInteraction::SingleClick,
                WireMouseInteraction::DoubleClick => MouseInteraction::DoubleClick,
            };
            let mods: Vec<KeyModifier> = modifiers
                .iter()
                .map(|m| match m {
                    AgentKeyModifier::Shift => KeyModifier::Shift,
                    AgentKeyModifier::Control => KeyModifier::Control,
                    AgentKeyModifier::Alt => KeyModifier::Alt,
                    AgentKeyModifier::Meta => KeyModifier::Meta,
                })
                .collect();
            AutomationRequest::perform_mouse_interaction(id, handle, *x, *y, btn, kind, &mods)
                .ok_or_else(invalid)
        }
        AutomationWireCommand::KeyboardInteractions { handle, steps } => {
            let wire_steps: Vec<KeyboardInteraction> = steps
                .iter()
                .map(|step| match step {
                    WireKeyStep::InsertByKey(text) => {
                        KeyboardInteraction::InsertByKey { text: text.clone() }
                    }
                    WireKeyStep::KeyPress(key) => KeyboardInteraction::KeyPress {
                        key: (*key).to_owned(),
                    },
                    WireKeyStep::KeyRelease(key) => KeyboardInteraction::KeyRelease {
                        key: (*key).to_owned(),
                    },
                })
                .collect();
            AutomationRequest::perform_keyboard_interactions(id, handle, &wire_steps)
                .ok_or_else(invalid)
        }
        AutomationWireCommand::WheelScroll {
            handle,
            x,
            y,
            delta_x,
            delta_y,
        } => AutomationRequest::perform_wheel_scroll(id, handle, *x, *y, *delta_x, *delta_y)
            .ok_or_else(invalid),
        AutomationWireCommand::TakeScreenshot {
            handle,
            clip_to_viewport,
        } => AutomationRequest::take_screenshot(id, handle, None, None, false, *clip_to_viewport)
            .ok_or_else(invalid),
        AutomationWireCommand::SendInspectorMessage { handle, command } => {
            let inspector_cmd = match command {
                DecodedSubscription::Direct { id: cmd_id, method } => {
                    let obs = match method {
                        DirectMethod::NetworkEnable => InspectorObservationMethod::NetworkEnable,
                        DirectMethod::NetworkDisable => InspectorObservationMethod::NetworkDisable,
                        DirectMethod::PageEnable => InspectorObservationMethod::PageEnable,
                        DirectMethod::PageDisable => InspectorObservationMethod::PageDisable,
                    };
                    InspectorCommand::direct(*cmd_id, obs).ok_or_else(invalid)?
                }
                DecodedSubscription::Target {
                    id: cmd_id,
                    target_id,
                    inner_id,
                    method,
                } => {
                    let obs = match method {
                        TargetMethod::ConsoleEnable => InspectorObservationMethod::ConsoleEnable,
                        TargetMethod::ConsoleDisable => InspectorObservationMethod::ConsoleDisable,
                        TargetMethod::RuntimeEnable => InspectorObservationMethod::RuntimeEnable,
                        TargetMethod::RuntimeDisable => InspectorObservationMethod::RuntimeDisable,
                        TargetMethod::NetworkEnable => InspectorObservationMethod::NetworkEnable,
                        TargetMethod::NetworkDisable => InspectorObservationMethod::NetworkDisable,
                        TargetMethod::PageEnable => InspectorObservationMethod::PageEnable,
                        TargetMethod::PageDisable => InspectorObservationMethod::PageDisable,
                    };
                    InspectorCommand::for_target(*cmd_id, target_id, *inner_id, obs)
                        .ok_or_else(invalid)?
                }
            };
            AutomationRequest::send_inspector_message(id, handle, &inspector_cmd)
                .ok_or_else(invalid)
        }
    }
}

fn map_automation_error(error: AutomationError) -> ToolError {
    match error {
        AutomationError::UnsupportedSpi => ToolError::new(
            ToolErrorCode::ToolNotAvailable,
            "isolated agent automation requires Quickie's WebKitGTK fork",
        ),
        AutomationError::WrongThread => ToolError::new(
            ToolErrorCode::Unavailable,
            "agent automation must run on the GTK main thread",
        ),
        AutomationError::InvalidSessionId | AutomationError::InvalidUrl => {
            ToolError::new(ToolErrorCode::InvalidParams, error.to_string())
        }
        AutomationError::InvalidViewport => {
            ToolError::new(ToolErrorCode::BoundsExceeded, error.to_string())
        }
        AutomationError::NavigationNotApproved => {
            ToolError::new(ToolErrorCode::ApprovalDenied, error.to_string())
        }
        AutomationError::ClosedSession => ToolError::new(
            ToolErrorCode::AttachmentUnavailable,
            "agent context is not available",
        ),
        AutomationError::SessionCreationFailed
        | AutomationError::ControlledViewFailed
        | AutomationError::DispatchFailed => {
            ToolError::new(ToolErrorCode::Unavailable, error.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone GLib main-loop host (never opens Profile::Main or any window)
// ---------------------------------------------------------------------------

struct StandaloneRuntime {
    product_version: String,
    port: Arc<DispatchPort<GlibWakeup>>,
    agent: AgentManager<GtkAgentBackend>,
    main_loop: Object,
}

impl StandaloneRuntime {
    fn handle_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::ClientClosed(client) => {
                self.agent.on_client_closed(client);
                // SAFETY: Quits the standalone GLib main loop cleanly on stdio EOF.
                unsafe { ffi::g_main_loop_quit(self.main_loop) };
            }
            HostEvent::Operation { client, operation } => {
                self.handle_operation(client, operation);
            }
        }
    }

    fn handle_operation(&mut self, client: ClientId, operation: BrowserOperation) {
        match operation {
            BrowserOperation::BrowserStatus { reply } => {
                if reply.is_closed() {
                    return;
                }
                let agent_ok = self.agent.is_available();
                let diagnostic = if agent_ok {
                    DiagnosticCapabilities {
                        dom: true,
                        accessibility: true,
                        console: true,
                        runtime_errors: true,
                        pending_network: true,
                        completed_network: true,
                        lifecycle: true,
                    }
                } else {
                    DiagnosticCapabilities::default()
                };
                reply.resolve(BrowserStatus {
                    platform: HostPlatform::Linux,
                    runtime_mode: RuntimeMode::Standalone,
                    product_version: self.product_version.clone(),
                    fork_capabilities: ForkCapabilities {
                        diagnostic,
                        native_accessibility: false,
                        visual_inspector_priority: false,
                        agent_automation: agent_ok,
                    },
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
            BrowserOperation::PageSnapshot {
                target,
                request,
                reply,
            } => match target {
                ObservationTarget::Diagnostic(_) => reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "diagnostic attachment is not available",
                )),
                ObservationTarget::Agent(context_id) => {
                    self.agent
                        .handle_page_snapshot(client, context_id, &request, reply);
                }
            },
            BrowserOperation::PageEvents {
                target,
                cursor,
                filter,
                limit,
                reply,
            } => match target {
                ObservationTarget::Diagnostic(_) => reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "diagnostic attachment is not available",
                )),
                ObservationTarget::Agent(context_id) => {
                    self.agent
                        .handle_page_events(client, context_id, cursor, &filter, limit, reply);
                }
            },
            BrowserOperation::Agent(operation) => {
                self.agent.handle_operation(client, operation);
            }
        }
    }
}

thread_local! {
    static STANDALONE_HOST: RefCell<Option<StandaloneRuntime>> = const { RefCell::new(None) };
}

/// Serves standalone `quickie mcp` over isolated stdio with a `GLib` main loop and lazy GTK init.
///
/// Never opens `Profile::Main` or presents a window, handles `browser_status` even without a
/// Wayland/display compositor, and shuts down all isolated Agent contexts cleanly on stdin EOF.
pub(crate) fn serve_standalone(
    isolated: IsolatedStdio,
    product_version: &str,
) -> Result<(), String> {
    let wakeup = GlibWakeup::default_context(drain_standalone_on_main_context)
        .ok_or_else(|| "could not acquire GLib default main context".to_owned())?;
    let port = Arc::new(DispatchPort::new(wakeup));
    // SAFETY: Creates a GMainLoop bound to the default GMainContext on this thread.
    let main_loop = unsafe { ffi::g_main_loop_new(ptr::null_mut(), FALSE) };
    if main_loop.is_null() {
        return Err("could not create GLib main loop".to_owned());
    }

    STANDALONE_HOST.with(|slot| {
        *slot.borrow_mut() = Some(StandaloneRuntime {
            product_version: product_version.to_owned(),
            port: Arc::clone(&port),
            agent: AgentManager::new(GtkAgentBackend::standalone()),
            main_loop,
        });
    });

    let handler = BrowserHandler::new(ClientId::new(1), Arc::clone(&port))
        .with_product_version(product_version);
    let server = match serve_stdio(isolated, handler) {
        Ok(handle) => handle,
        Err(err) => {
            STANDALONE_HOST.with(|slot| {
                if let Some(mut rt) = slot.borrow_mut().take() {
                    rt.agent.shutdown();
                }
            });
            port.close();
            // SAFETY: Releases the GMainLoop created above.
            unsafe { ffi::g_main_loop_unref(main_loop) };
            return Err(err.to_string());
        }
    };

    // SAFETY: Runs the standalone GLib main loop until EOF quits it cleanly.
    unsafe { ffi::g_main_loop_run(main_loop) };

    port.close();
    STANDALONE_HOST.with(|slot| {
        if let Some(mut rt) = slot.borrow_mut().take() {
            rt.agent.shutdown();
        }
    });
    server.wait();
    // SAFETY: Releases the GMainLoop created above after the loop has exited.
    unsafe { ffi::g_main_loop_unref(main_loop) };
    Ok(())
}

extern "C" fn drain_standalone_on_main_context(_data: Object) -> Bool {
    STANDALONE_HOST.with(|slot| {
        let Ok(mut guard) = slot.try_borrow_mut() else {
            return;
        };
        let Some(rt) = guard.as_mut() else {
            return;
        };
        let events = rt.port.drain();
        for event in events {
            rt.handle_event(event);
        }
    });
    FALSE
}

// ---------------------------------------------------------------------------
// Native callbacks routed to either GUI ACTIVE_HOST or STANDALONE_HOST
// ---------------------------------------------------------------------------

fn dispatch_agent_message(target: AgentDispatchTarget, context_id: AgentContextId, frame: &str) {
    match target {
        AgentDispatchTarget::Gui => super::on_agent_message(context_id, frame),
        AgentDispatchTarget::Standalone => {
            with_standalone(|agent| agent.on_automation_message(context_id, frame));
        }
    }
}

fn dispatch_agent_closed(target: AgentDispatchTarget, context_id: AgentContextId) {
    match target {
        AgentDispatchTarget::Gui => super::on_agent_closed(context_id),
        AgentDispatchTarget::Standalone => {
            with_standalone(|agent| agent.on_session_closed(context_id));
        }
    }
}

fn dispatch_script_completed(
    target: AgentDispatchTarget,
    eval_id: u64,
    result: Result<String, ToolError>,
) {
    match target {
        AgentDispatchTarget::Gui => super::on_agent_script_completed(eval_id, result),
        AgentDispatchTarget::Standalone => {
            with_standalone(|agent| agent.on_script_completed(eval_id, result));
        }
    }
}

fn dispatch_timer_fired(target: AgentDispatchTarget, timer_id: u64) {
    match target {
        AgentDispatchTarget::Gui => super::on_agent_timer_fired(timer_id),
        AgentDispatchTarget::Standalone => {
            with_standalone(|agent| agent.on_timer_fired(timer_id));
        }
    }
}

fn with_standalone(f: impl FnOnce(&mut AgentManager<GtkAgentBackend>)) {
    STANDALONE_HOST.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut()
            && let Some(rt) = guard.as_mut()
        {
            f(&mut rt.agent);
        }
    });
}

struct CrashSignalData {
    target: AgentDispatchTarget,
    context_id: AgentContextId,
}

extern "C" fn on_web_process_terminated(_view: Object, _reason: c_int, data: Object) {
    if data.is_null() {
        return;
    }
    // SAFETY: `data` stays alive until `destroy_crash_data` runs on disconnect.
    let info = unsafe { &*data.cast::<CrashSignalData>() };
    dispatch_agent_closed(info.target, info.context_id);
}

unsafe extern "C" fn destroy_crash_data(data: Object, _closure: Object) {
    if !data.is_null() {
        // SAFETY: Reclaims the unique boxed `CrashSignalData`.
        drop(unsafe { Box::from_raw(data.cast::<CrashSignalData>()) });
    }
}

struct ScriptCallbackData {
    target: AgentDispatchTarget,
    eval_id: u64,
}

extern "C" fn on_script_finished(view: Object, result: Object, data: Object) {
    // SAFETY: Reclaims the unique boxed `ScriptCallbackData` from `start_script`.
    let callback = unsafe { Box::from_raw(data.cast::<ScriptCallbackData>()) };
    // SAFETY: Omitting GError suppresses raw page script details from leaking across MCP.
    let value =
        unsafe { webkit_web_view_evaluate_javascript_finish(view, result, ptr::null_mut()) };
    if value.is_null() {
        dispatch_script_completed(
            callback.target,
            callback.eval_id,
            Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "isolated-world evaluation failed",
            )),
        );
        return;
    }
    // SAFETY: Transfers ownership of the JSCValue string allocation from WebKit.
    let text = unsafe { jsc_value_to_string(value) };
    // SAFETY: Releases the owned JSCValue returned by `evaluate_javascript_finish`.
    unsafe { ffi::g_object_unref(value) };
    let outcome = if text.is_null() {
        Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "could not decode isolated-world script result",
        ))
    } else {
        // SAFETY: Non-null GLib string is NUL-terminated and copied before freeing.
        let raw = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `jsc_value_to_string` allocates with GLib's allocator.
        unsafe { g_free(text.cast()) };
        Ok(raw)
    };
    dispatch_script_completed(callback.target, callback.eval_id, outcome);
}

struct TimerCallbackData {
    target: AgentDispatchTarget,
    timer_id: u64,
    source_id: Rc<Cell<c_uint>>,
}

extern "C" fn on_timer_cb(data: Object) -> Bool {
    if data.is_null() {
        return FALSE;
    }
    // SAFETY: `data` is alive for the duration of the GLib callback.
    let info = unsafe { &*data.cast::<TimerCallbackData>() };
    info.source_id.set(0);
    dispatch_timer_fired(info.target, info.timer_id);
    FALSE
}

extern "C" fn destroy_timer_data(data: Object) {
    if !data.is_null() {
        // SAFETY: Reclaims the unique boxed `TimerCallbackData` allocated in `schedule_timer`.
        drop(unsafe { Box::from_raw(data.cast::<TimerCallbackData>()) });
    }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_init_check() -> Bool;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_signal_handler_disconnect(instance: Object, handler_id: c_ulong);
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(memory: Object);
    fn g_timeout_add_full(
        priority: c_int,
        interval: c_uint,
        function: extern "C" fn(Object) -> Bool,
        data: Object,
        notify: extern "C" fn(Object),
    ) -> c_uint;
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_cancellable_new() -> Object;
    fn g_cancellable_cancel(cancel: Object);
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_evaluate_javascript(
        view: Object,
        script: *const c_char,
        length: isize,
        world: *const c_char,
        source_uri: *const c_char,
        cancel: Object,
        callback: extern "C" fn(Object, Object, Object),
        data: Object,
    );
    fn webkit_web_view_evaluate_javascript_finish(
        view: Object,
        result: Object,
        error: *mut *mut ffi::GError,
    ) -> Object;
}

#[link(name = "javascriptcoregtk-6.0")]
unsafe extern "C" {
    fn jsc_value_to_string(value: Object) -> *mut c_char;
}

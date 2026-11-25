//! GUI-owned Linux MCP Unix socket host and native `GLib` main-context dispatch bridge.
//!
//! Queues one-shot idle `GSource`s onto the retained default `GMainContext` for zero-polling,
//! thread-safe wakeups that always execute on the `GLib` loop's owning thread without sending any
//! `GObject`s across threads or leaving dangling source IDs on shutdown.

#![expect(unsafe_code, reason = "GLib GMainContext and GSource wakeup FFI")]

pub(crate) mod agent;
mod backend;
mod consent;

use std::cell::RefCell;
use std::ffi::{c_int, c_uint, c_void};
use std::io::{self, Write};
use std::ptr::{self, NonNull};
use std::sync::Arc;

use quickie_application::DiagnosticSnapshot;
use quickie_application::mcp::{AgentContextId, AttachmentId, DocumentBinding, NavigationKind};
use quickie_domain_browsing::TabId;
use quickie_mcp::{BrowserHandlerFactory, ServerHandle, SocketEndpoint, ToolError, serve_socket};

use crate::gtk::App;
use crate::gtk::ffi::{Bool, FALSE, Object};
use crate::mcp::product_version;
use crate::mcp::runtime::controller::McpController;
use crate::mcp::runtime::{DispatchPort, MainContextWakeup};

use self::agent::GtkAgentBackend;
use self::backend::GtkBackend;
use self::consent::ConsentView;

const G_PRIORITY_DEFAULT_IDLE: c_int = 200;

struct ActiveGtkHost {
    port: Arc<DispatchPort<GlibWakeup>>,
    controller: McpController<GtkBackend, GtkAgentBackend>,
}

thread_local! {
    /// Main-thread-only MCP controller and dispatch drain slot.
    ///
    /// Never crosses threads and is cleared synchronously during [`GtkMcpHost::shutdown`].
    static ACTIVE_HOST: RefCell<Option<ActiveGtkHost>> = const { RefCell::new(None) };
}

/// Thread-safe `GLib` `GMainContext` wakeup backed by an attached idle `GSource`.
///
/// Unlike `g_main_context_invoke_full`, attaching a fresh idle `GSource` never runs the callback
/// inline on a calling worker thread when the default context currently has no owner (for example,
/// before `g_main_loop_run` starts in standalone mode).
pub(super) struct GlibWakeup {
    context: NonNull<c_void>,
    callback: extern "C" fn(Object) -> Bool,
}

// SAFETY: GLib guarantees `g_idle_source_new`, `g_source_attach`, `g_source_unref`, and
// `g_main_context_unref` are thread-safe, and `callback` is a stateless function pointer.
unsafe impl Send for GlibWakeup {}
// SAFETY: GLib guarantees attaching a newly created `GSource` to a retained `GMainContext` is
// safe to call concurrently from any thread.
unsafe impl Sync for GlibWakeup {}

impl GlibWakeup {
    pub(super) fn default_context(callback: extern "C" fn(Object) -> Bool) -> Option<Self> {
        // SAFETY: GLib always provides a process-wide default main context.
        let raw = unsafe { g_main_context_default() };
        if raw.is_null() {
            return None;
        }
        // SAFETY: Acquires an owned reference balanced in `Drop`.
        let owned = unsafe { g_main_context_ref(raw) };
        NonNull::new(owned).map(|context| Self { context, callback })
    }
}

impl MainContextWakeup for GlibWakeup {
    fn wake(&self) {
        // SAFETY: `self.context` is a retained `GMainContext`. A freshly created idle `GSource`
        // is configured before attachment, attached thread-safely to `self.context` (which takes a
        // reference), and unreffed so GLib frees it once `self.callback` returns `FALSE`
        // (`G_SOURCE_REMOVE`). The callback takes no raw pointer state and always executes on the
        // thread running `self.context`.
        unsafe {
            let source = g_idle_source_new();
            if source.is_null() {
                return;
            }
            g_source_set_priority(source, G_PRIORITY_DEFAULT_IDLE);
            g_source_set_callback(source, self.callback, ptr::null_mut(), None);
            let _ = g_source_attach(source, self.context.as_ptr());
            g_source_unref(source);
        }
    }
}

impl Drop for GlibWakeup {
    fn drop(&mut self) {
        // SAFETY: Releases the reference acquired in `GlibWakeup::default_context`.
        unsafe { g_main_context_unref(self.context.as_ptr()) };
    }
}

/// GUI-owned MCP server handle and dispatch port stored inside [`crate::gtk::App`].
#[derive(Default)]
pub(crate) struct GtkMcpHost {
    server: RefCell<Option<ServerHandle>>,
    port: RefCell<Option<Arc<DispatchPort<GlibWakeup>>>>,
}

impl GtkMcpHost {
    /// Attaches the GTK consent UI and starts the GUI-owned private MCP Unix socket listener.
    ///
    /// Isolated smoke/update sessions do not bind the user's MCP socket.
    pub(crate) fn attach(&self, app: &App, overlay: Object, isolated: bool) {
        let Some(wakeup) = GlibWakeup::default_context(drain_on_main_context) else {
            return;
        };
        let port = Arc::new(DispatchPort::new(wakeup));
        let consent = ConsentView::new(overlay);
        let controller = McpController::with_agent(
            GtkBackend::new(ptr::from_ref(app), consent),
            GtkAgentBackend::gui(consent),
        );

        ACTIVE_HOST.with(|slot| {
            *slot.borrow_mut() = Some(ActiveGtkHost {
                port: Arc::clone(&port),
                controller,
            });
        });
        self.port.replace(Some(Arc::clone(&port)));

        if isolated {
            return;
        }

        match start_socket_server(port) {
            Ok(handle) => {
                self.server.replace(Some(handle));
            }
            Err(error) => {
                let _ = writeln!(io::stderr(), "Quickie MCP: {error}");
            }
        }
    }

    /// Shuts down the MCP socket server, revokes all active grants, and drops all native leases.
    pub(crate) fn shutdown(&self) {
        if let Some(port) = self.port.take() {
            port.close();
        }
        ACTIVE_HOST.with(|slot| {
            if let Some(mut host) = slot.borrow_mut().take() {
                host.controller.shutdown();
            }
        });
        if let Some(server) = self.server.take() {
            server.shutdown();
        }
    }
}

fn start_socket_server(port: Arc<DispatchPort<GlibWakeup>>) -> Result<ServerHandle, String> {
    let endpoint = SocketEndpoint::default_endpoint().map_err(|error| error.to_string())?;
    let factory =
        Arc::new(BrowserHandlerFactory::new(port).with_product_version(product_version()));
    serve_socket(&endpoint, move || factory.next_handler()).map_err(|error| error.to_string())
}

extern "C" fn drain_on_main_context(_data: Object) -> Bool {
    ACTIVE_HOST.with(|slot| {
        let Ok(mut guard) = slot.try_borrow_mut() else {
            return;
        };
        let Some(host) = guard.as_mut() else {
            return;
        };
        let events = host.port.drain();
        for event in events {
            host.controller.handle_event(event);
        }
    });
    FALSE
}

pub(crate) fn key_pressed(key: c_uint, modifiers: c_uint) -> bool {
    let open = ACTIVE_HOST.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|guard| {
                guard
                    .as_ref()
                    .map(|host| host.controller.has_pending_consent())
            })
            .unwrap_or(false)
    });
    if !open {
        return false;
    }
    consent::handle_key(key, modifiers)
}

pub(crate) fn on_active_tab_changed(active_tab: Option<TabId>) {
    with_controller(|controller| controller.on_active_tab_changed(active_tab));
}

pub(crate) fn on_navigation(tab: TabId, observed: &DocumentBinding, kind: NavigationKind) {
    with_controller(|controller| controller.on_navigation(tab, observed, kind));
}

pub(crate) fn on_tab_closed(tab: TabId) {
    with_controller(|controller| controller.on_tab_closed(tab));
}

pub(crate) fn on_process_terminated(tab: TabId) {
    with_controller(|controller| controller.on_process_terminated(tab));
}

pub(crate) fn on_inspector_opened(tab: TabId) {
    with_controller(|controller| controller.on_inspector_opened(tab));
}

pub(super) fn resolve_consent(allow: bool) {
    with_controller(|controller| controller.resolve_consent(allow));
}

pub(super) fn stop_active_grants() {
    with_controller(|controller| controller.stop_all_user_grants());
}

pub(super) fn on_native_frame(attachment: AttachmentId, frame: &str) {
    with_controller(|controller| controller.on_native_frame(attachment, frame));
}

pub(super) fn on_native_closed(attachment: AttachmentId) {
    with_controller(|controller| controller.on_native_closed(attachment));
}

pub(super) fn on_snapshot_completed(
    snapshot_id: u64,
    result: Result<DiagnosticSnapshot, ToolError>,
) {
    with_controller(|controller| controller.on_snapshot_completed(snapshot_id, result));
}

pub(super) fn on_agent_message(context_id: AgentContextId, frame: &str) {
    with_controller(|controller| controller.on_agent_message(context_id, frame));
}

pub(super) fn on_agent_closed(context_id: AgentContextId) {
    with_controller(|controller| controller.on_agent_closed(context_id));
}

pub(super) fn on_agent_script_completed(eval_id: u64, result: Result<String, ToolError>) {
    with_controller(|controller| controller.on_agent_script_completed(eval_id, result));
}

pub(super) fn on_agent_timer_fired(timer_id: u64) {
    with_controller(|controller| controller.on_agent_timer_fired(timer_id));
}

fn with_controller(operation: impl FnOnce(&mut McpController<GtkBackend, GtkAgentBackend>)) {
    ACTIVE_HOST.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut()
            && let Some(host) = guard.as_mut()
        {
            operation(&mut host.controller);
        }
    });
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_main_context_default() -> Object;
    fn g_main_context_ref(context: Object) -> Object;
    fn g_main_context_unref(context: Object);
    fn g_idle_source_new() -> Object;
    fn g_source_set_priority(source: Object, priority: c_int);
    fn g_source_set_callback(
        source: Object,
        function: extern "C" fn(Object) -> Bool,
        data: Object,
        notify: Option<unsafe extern "C" fn(Object)>,
    );
    fn g_source_attach(source: Object, context: Object) -> c_uint;
    fn g_source_unref(source: Object);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::c_uint;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread::{self, ThreadId};

    use quickie_application::mcp::ClientId;
    use quickie_mcp::BrowserHost;

    use crate::gtk::ffi::{self, Bool, FALSE, Object};
    use crate::mcp::runtime::{DispatchPort, HostEvent};

    use super::{
        G_PRIORITY_DEFAULT_IDLE, GlibWakeup, g_source_attach, g_source_set_callback,
        g_source_set_priority, g_source_unref,
    };

    struct TestSlot {
        port: Arc<DispatchPort<GlibWakeup>>,
        main_loop: Object,
        drained_client: Arc<Mutex<Option<ClientId>>>,
        callback_thread: Arc<Mutex<Option<ThreadId>>>,
    }

    thread_local! {
        static TEST_HOST: RefCell<Option<TestSlot>> = const { RefCell::new(None) };
    }

    static EMPTY_SLOT_HIT: AtomicBool = AtomicBool::new(false);

    extern "C" fn drain_test_host(_data: Object) -> Bool {
        TEST_HOST.with(|slot| {
            let Ok(mut guard) = slot.try_borrow_mut() else {
                return;
            };
            let Some(host) = guard.as_mut() else {
                EMPTY_SLOT_HIT.store(true, Ordering::SeqCst);
                return;
            };
            *host
                .callback_thread
                .lock()
                .expect("callback_thread lock poisoned") = Some(thread::current().id());
            for event in host.port.drain() {
                if let HostEvent::ClientClosed(client) = event {
                    *host
                        .drained_client
                        .lock()
                        .expect("drained_client lock poisoned") = Some(client);
                    // SAFETY: Quits the test main loop cleanly after draining ClientClosed.
                    unsafe { ffi::g_main_loop_quit(host.main_loop) };
                }
            }
        });
        FALSE
    }

    extern "C" fn quit_on_watchdog(main_loop: Object) -> Bool {
        // SAFETY: `main_loop` stays alive until `g_main_loop_run` returns in the test.
        unsafe { ffi::g_main_loop_quit(main_loop) };
        FALSE
    }

    #[link(name = "glib-2.0")]
    unsafe extern "C" {
        fn g_source_destroy(source: Object);
        fn g_timeout_source_new(interval: c_uint) -> Object;
    }

    #[test]
    fn worker_wake_before_main_loop_run_drains_client_closed_on_loop_thread() {
        // Arrange
        EMPTY_SLOT_HIT.store(false, Ordering::SeqCst);
        let main_thread = thread::current().id();
        let wakeup =
            GlibWakeup::default_context(drain_test_host).expect("GLib default context available");
        let port = Arc::new(DispatchPort::new(wakeup));
        // SAFETY: Creates a GMainLoop bound to the default GMainContext on this thread.
        let main_loop = unsafe { ffi::g_main_loop_new(ptr::null_mut(), FALSE) };
        assert!(!main_loop.is_null(), "GLib main loop created");

        let drained_client = Arc::new(Mutex::new(None));
        let callback_thread = Arc::new(Mutex::new(None));
        TEST_HOST.with(|slot| {
            *slot.borrow_mut() = Some(TestSlot {
                port: Arc::clone(&port),
                main_loop,
                drained_client: Arc::clone(&drained_client),
                callback_thread: Arc::clone(&callback_thread),
            });
        });

        // Act: Worker thread emits ClientClosed before the main thread enters g_main_loop_run.
        let wake_finished = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&wake_finished);
        let worker_port = Arc::clone(&port);
        let worker = thread::spawn(move || {
            worker_port.client_closed(ClientId::new(1));
            worker_barrier.wait();
        });
        wake_finished.wait();
        worker.join().expect("worker thread joined");

        // SAFETY: Attaches a lower-priority watchdog after the worker finishes so a lost wakeup
        // fails promptly without racing the queued idle source under load. Retain `watchdog` and
        // destroy/unref it before `main_loop` is freed so it can never fire later with a dangling
        // `main_loop` pointer.
        let watchdog = unsafe {
            let source = g_timeout_source_new(500);
            assert!(!source.is_null(), "GLib watchdog source created");
            g_source_set_priority(source, G_PRIORITY_DEFAULT_IDLE + 1);
            g_source_set_callback(source, quit_on_watchdog, main_loop, None);
            let _ = g_source_attach(source, ptr::null_mut());
            source
        };

        // SAFETY: Runs the GLib main loop until the queued idle source drains ClientClosed.
        unsafe { ffi::g_main_loop_run(main_loop) };

        TEST_HOST.with(|slot| {
            *slot.borrow_mut() = None;
        });
        // SAFETY: Detaches and releases the watchdog before freeing `main_loop`.
        unsafe {
            g_source_destroy(watchdog);
            g_source_unref(watchdog);
            ffi::g_main_loop_unref(main_loop);
        }

        // Assert
        assert!(
            !EMPTY_SLOT_HIT.load(Ordering::SeqCst),
            "wakeup callback must not run inline on the worker thread"
        );
        assert_eq!(
            *callback_thread
                .lock()
                .expect("callback_thread lock poisoned"),
            Some(main_thread),
            "wakeup callback must execute on the GLib main loop thread"
        );
        assert_eq!(
            *drained_client.lock().expect("drained_client lock poisoned"),
            Some(ClientId::new(1)),
            "early ClientClosed must be drained once the main loop starts"
        );
    }
}

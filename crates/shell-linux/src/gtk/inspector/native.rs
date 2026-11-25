//! Native `GTK4` and `WebKitGTK` dynamic ABI for local Inspector observation.

#![expect(
    dead_code,
    reason = "Wired by the sibling MCP observation transport slice"
)]

use std::ffi::{CStr, c_char, c_ulong, c_void};
use std::ptr;

use crate::gtk::ffi::{self, Bool, Object};

use super::observation::{ClosedCallback, DestroyCallback, InspectorAbi, MessageCallback};

type ConnectDiagnostic = unsafe extern "C" fn(Object) -> Bool;
type SendDiagnostic = unsafe extern "C" fn(Object, *const c_char) -> Bool;
type DisconnectDiagnostic = unsafe extern "C" fn(Object);

const DIAGNOSTIC_MESSAGE_SIGNAL: &CStr = c"diagnostic-message-received";
const CLOSED_SIGNAL: &CStr = c"closed";

#[derive(Clone, Copy)]
struct DiagnosticSymbols {
    connect: ConnectDiagnostic,
    send: SendDiagnostic,
    disconnect: DisconnectDiagnostic,
}

// A partial API must never be used, so stock WebKitGTK keeps starting cleanly for UI tests.
macro_rules! resolve_symbol {
    ($name:literal, $signature:ty) => {{
        // SAFETY: RTLD_DEFAULT searches libraries already loaded in this process.
        let raw = unsafe { dlsym(ptr::null_mut(), $name.as_ptr()) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: A non-null lookup of the named WebKitGTK fork symbol has the declared C ABI.
        unsafe { std::mem::transmute::<Object, $signature>(raw) }
    }};
}

impl DiagnosticSymbols {
    fn resolve() -> Option<Self> {
        Some(Self {
            connect: resolve_symbol!(
                c"webkit_web_inspector_connect_local_diagnostic_client",
                ConnectDiagnostic
            ),
            send: resolve_symbol!(
                c"webkit_web_inspector_send_diagnostic_message",
                SendDiagnostic
            ),
            disconnect: resolve_symbol!(
                c"webkit_web_inspector_disconnect_local_diagnostic_client",
                DisconnectDiagnostic
            ),
        })
    }
}

/// Real `WebKitGTK` and `GObject` ABI implementation for the GTK main thread.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GtkInspectorAbi;

impl InspectorAbi for GtkInspectorAbi {
    fn is_main_thread(&self) -> bool {
        // SAFETY: GLib always provides a process-wide default main context.
        let context = unsafe { g_main_context_default() };
        if context.is_null() {
            return false;
        }
        // SAFETY: The borrowed default GMainContext is valid for the lifetime of the process.
        unsafe { g_main_context_is_owner(context) == ffi::TRUE }
    }

    fn has_diagnostic_symbols(&self) -> bool {
        DiagnosticSymbols::resolve().is_some()
    }

    fn inspector_for_view(&self, view: Object) -> Object {
        if view.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: The caller supplies a live WebKitWebView on the GTK main thread.
        unsafe { ffi::webkit_web_view_get_inspector(view) }
    }

    fn ref_inspector(&self, inspector: Object) {
        // SAFETY: The inspector is a live GObject on the GTK main thread.
        let _ = unsafe { g_object_ref(inspector) };
    }

    fn unref_inspector(&self, inspector: Object) {
        // SAFETY: Releases the reference acquired by `ref_inspector`.
        unsafe { ffi::g_object_unref(inspector) };
    }

    fn connect_message_signal(
        &self,
        inspector: Object,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `inspector` is a live WebKitWebInspector, `handler` matches the
        // `diagnostic-message-received` signal ABI, and `destroy` frees `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                inspector,
                DIAGNOSTIC_MESSAGE_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn connect_closed_signal(
        &self,
        inspector: Object,
        handler: ClosedCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `inspector` is a live WebKitWebInspector, `handler` matches the
        // `closed` signal ABI, and `destroy` frees `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                inspector,
                CLOSED_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn disconnect_signal(&self, inspector: Object, handler_id: c_ulong) {
        if handler_id == 0 {
            return;
        }
        // SAFETY: `inspector` is kept alive by our ref and `handler_id` was returned by connect.
        unsafe { g_signal_handler_disconnect(inspector, handler_id) };
    }

    fn connect_local_diagnostic_client(&self, inspector: Object) -> bool {
        let Some(symbols) = DiagnosticSymbols::resolve() else {
            return false;
        };
        // SAFETY: `inspector` is a live WebKitWebInspector on the GTK main thread.
        unsafe { (symbols.connect)(inspector) == ffi::TRUE }
    }

    fn send_diagnostic_message(&self, inspector: Object, message: &CStr) -> bool {
        let Some(symbols) = DiagnosticSymbols::resolve() else {
            return false;
        };
        // SAFETY: `inspector` is live on the GTK main thread and `message` is NUL-terminated.
        unsafe { (symbols.send)(inspector, message.as_ptr()) == ffi::TRUE }
    }

    fn disconnect_local_diagnostic_client(&self, inspector: Object) {
        let Some(symbols) = DiagnosticSymbols::resolve() else {
            return;
        };
        // SAFETY: `inspector` is a live WebKitWebInspector on the GTK main thread.
        unsafe { (symbols.disconnect)(inspector) };
    }
}

unsafe extern "C" {
    fn dlsym(handle: Object, name: *const c_char) -> Object;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_main_context_default() -> Object;
    fn g_main_context_is_owner(context: Object) -> Bool;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref(object: Object) -> Object;
    fn g_signal_handler_disconnect(instance: Object, handler_id: c_ulong);
}

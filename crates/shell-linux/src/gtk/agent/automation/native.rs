//! Native `GTK4` and `WebKitGTK` dynamic ABI for isolated local Automation sessions.

use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;

use url::Url;

use crate::gtk::ffi::{self, Bool, Object};

use super::session::{
    AutomationAbi, CreateWebViewCallback, DecidePolicyCallback, DestroyCallback, MessageCallback,
    NavigationRequestInfo, PermissionRequestCallback, ResponseInfo, WillCloseCallback,
};

type NewLocalContext = unsafe extern "C" fn(*const c_char) -> Object;
type GetLocalSession = unsafe extern "C" fn(Object) -> Object;
type CloseLocalSession = unsafe extern "C" fn(Object);
type SendAutomationMessage = unsafe extern "C" fn(Object, *const c_char) -> Bool;

const MESSAGE_RECEIVED_SIGNAL: &CStr = c"message-received";
const WILL_CLOSE_SIGNAL: &CStr = c"will-close";
const CREATE_WEB_VIEW_SIGNAL: &CStr = c"create-web-view";
const DECIDE_POLICY_SIGNAL: &CStr = c"decide-policy";
const PERMISSION_REQUEST_SIGNAL: &CStr = c"permission-request";

#[derive(Clone, Copy)]
struct LocalAutomationSymbols {
    new_context: NewLocalContext,
    get_session: GetLocalSession,
    close_session: CloseLocalSession,
    send_message: SendAutomationMessage,
}

// A partial SPI must never be used, so stock Debian WebKitGTK reports unavailable cleanly.
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

impl LocalAutomationSymbols {
    fn resolve() -> Option<Self> {
        Some(Self {
            new_context: resolve_symbol!(
                c"webkit_web_context_new_with_local_automation",
                NewLocalContext
            ),
            get_session: resolve_symbol!(
                c"webkit_web_context_get_local_automation_session",
                GetLocalSession
            ),
            close_session: resolve_symbol!(
                c"webkit_web_context_close_local_automation_session",
                CloseLocalSession
            ),
            send_message: resolve_symbol!(
                c"webkit_automation_session_send_message",
                SendAutomationMessage
            ),
        })
    }
}

/// Real `WebKitGTK` and `GObject` ABI implementation for the GTK main thread.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GtkAutomationAbi;

impl AutomationAbi for GtkAutomationAbi {
    fn is_main_thread(&self) -> bool {
        // SAFETY: GLib always provides a process-wide default main context.
        let context = unsafe { g_main_context_default() };
        if context.is_null() {
            return false;
        }
        // SAFETY: The borrowed default GMainContext is valid for the lifetime of the process.
        unsafe { g_main_context_is_owner(context) == ffi::TRUE }
    }

    fn has_local_automation_spi(&self) -> bool {
        if LocalAutomationSymbols::resolve().is_none() {
            return false;
        }
        // SAFETY: Returns the registered GType for WebKitAutomationSession.
        let session_type = unsafe { webkit_automation_session_get_type() };
        if session_type == 0 {
            return false;
        }
        // SAFETY: Ensures WebKitAutomationSession's class_init has installed its signals.
        let klass = unsafe { g_type_class_ref(session_type) };
        if klass.is_null() {
            return false;
        }
        // SAFETY: `session_type` is a valid initialized GObject type and the signal name is static.
        let signal_id = unsafe { g_signal_lookup(MESSAGE_RECEIVED_SIGNAL.as_ptr(), session_type) };
        // SAFETY: Releases the class reference acquired above.
        unsafe { g_type_class_unref(klass) };
        signal_id != 0
    }

    fn create_local_context(&self, session_id: &CStr) -> Object {
        let Some(symbols) = LocalAutomationSymbols::resolve() else {
            return ptr::null_mut();
        };
        // SAFETY: Called on the GTK main thread with a NUL-terminated UTF-8 session ID.
        unsafe { (symbols.new_context)(session_id.as_ptr()) }
    }

    fn local_automation_session(&self, context: Object) -> Object {
        let Some(symbols) = LocalAutomationSymbols::resolve() else {
            return ptr::null_mut();
        };
        if context.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: `context` is a live WebKitWebContext on the GTK main thread.
        unsafe { (symbols.get_session)(context) }
    }

    fn close_local_automation_session(&self, context: Object) {
        let Some(symbols) = LocalAutomationSymbols::resolve() else {
            return;
        };
        if context.is_null() {
            return;
        }
        // SAFETY: `context` is a live WebKitWebContext on the GTK main thread.
        unsafe { (symbols.close_session)(context) };
    }

    fn send_message(&self, session: Object, message: &CStr) -> bool {
        let Some(symbols) = LocalAutomationSymbols::resolve() else {
            return false;
        };
        if session.is_null() {
            return false;
        }
        // SAFETY: `session` is a live WebKitAutomationSession and `message` is NUL-terminated.
        unsafe { (symbols.send_message)(session, message.as_ptr()) == ffi::TRUE }
    }

    fn create_controlled_view(&self, context: Object) -> Object {
        if context.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: Returns the registered GType for WebKitWebView.
        let view_type = unsafe { webkit_web_view_get_type() };
        // SAFETY: `context` is a live WebKitWebContext and the property list is NULL-terminated.
        let raw_view = unsafe {
            g_object_new(
                view_type,
                c"web-context".as_ptr(),
                context,
                c"is-controlled-by-automation".as_ptr(),
                ffi::TRUE,
                ptr::null::<c_char>(),
            )
        };
        if raw_view.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: Newly constructed GtkWidget carries a floating reference to sink.
        let view = unsafe { g_object_ref_sink(raw_view) };
        if view.is_null() {
            return ptr::null_mut();
        }

        // Verify the view is strictly bound to our local context, controlled by automation,
        // and backed by the context's ephemeral network session.
        // SAFETY: `view` is a live WebKitWebView retained above.
        let bound_context = unsafe { webkit_web_view_get_context(view) };
        // SAFETY: `view` is a live WebKitWebView retained above.
        let is_controlled = unsafe { webkit_web_view_is_controlled_by_automation(view) };
        // SAFETY: `view` is a live WebKitWebView retained above.
        let network_session = unsafe { webkit_web_view_get_network_session(view) };
        let is_ephemeral = !network_session.is_null()
            // SAFETY: `network_session` is a live WebKitNetworkSession owned by the view/context.
            && unsafe { webkit_network_session_is_ephemeral(network_session) == ffi::TRUE };

        if bound_context != context || is_controlled != ffi::TRUE || !is_ephemeral {
            // SAFETY: Releases the reference acquired by `g_object_ref_sink`.
            unsafe { ffi::g_object_unref(view) };
            return ptr::null_mut();
        }

        // SAFETY: The new view owns its settings on this thread.
        let settings = unsafe { ffi::webkit_web_view_get_settings(view) };
        if !settings.is_null() {
            // SAFETY: Settings stay live for the view; disable unsolicited popup windows.
            unsafe {
                webkit_settings_set_javascript_can_open_windows_automatically(settings, ffi::FALSE);
            }
        }
        view
    }

    fn allocate_view(&self, view: Object, width: i32, height: i32) {
        if view.is_null() {
            return;
        }
        // SAFETY: Establishes a bounded offscreen viewport without realizing or mapping the widget.
        unsafe { gtk_widget_allocate(view, width, height, -1, ptr::null_mut()) };
    }

    fn stop_loading(&self, view: Object) {
        if view.is_null() {
            return;
        }
        // SAFETY: `view` is a live WebKitWebView retained by this session on the GTK main thread.
        unsafe { webkit_web_view_stop_loading(view) };
    }

    fn ref_object(&self, object: Object) {
        if object.is_null() {
            return;
        }
        // SAFETY: `object` is a live GObject on the GTK main thread.
        let _ = unsafe { g_object_ref(object) };
    }

    fn unref_object(&self, object: Object) {
        if object.is_null() {
            return;
        }
        // SAFETY: Releases one owned GObject reference on the GTK main thread.
        unsafe { ffi::g_object_unref(object) };
    }

    fn connect_message_signal(
        &self,
        session: Object,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `session` is a live WebKitAutomationSession, `handler` matches
        // `message-received`, and `destroy` releases `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                session,
                MESSAGE_RECEIVED_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn connect_will_close_signal(
        &self,
        session: Object,
        handler: WillCloseCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `session` is a live WebKitAutomationSession, `handler` matches `will-close`,
        // and `destroy` releases `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                session,
                WILL_CLOSE_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn connect_create_web_view_signal(
        &self,
        session: Object,
        handler: CreateWebViewCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `session` is a live WebKitAutomationSession, `handler` matches `create-web-view`,
        // and `destroy` releases `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                session,
                CREATE_WEB_VIEW_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn connect_decide_policy_signal(
        &self,
        view: Object,
        handler: DecidePolicyCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `view` is a live WebKitWebView, `handler` matches `decide-policy`,
        // and `destroy` releases `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                view,
                DECIDE_POLICY_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn connect_permission_request_signal(
        &self,
        view: Object,
        handler: PermissionRequestCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let handler_ptr: *const c_void = handler as *const c_void;
        // SAFETY: `view` is a live WebKitWebView, `handler` matches `permission-request`,
        // and `destroy` releases `data` on disconnect.
        unsafe {
            ffi::g_signal_connect_data(
                view,
                PERMISSION_REQUEST_SIGNAL.as_ptr(),
                handler_ptr,
                data,
                Some(destroy),
                0,
            )
        }
    }

    fn disconnect_signal(&self, instance: Object, handler_id: c_ulong) {
        if instance.is_null() || handler_id == 0 {
            return;
        }
        // SAFETY: `instance` is kept alive by our ref and `handler_id` was returned by connect.
        unsafe { g_signal_handler_disconnect(instance, handler_id) };
    }

    fn navigation_request_info(&self, decision: Object) -> NavigationRequestInfo {
        if decision.is_null() {
            return NavigationRequestInfo {
                url: None,
                method: None,
            };
        }
        // SAFETY: Supplied by WebKit during a navigation-action `decide-policy` emission.
        let action = unsafe { webkit_navigation_policy_decision_get_navigation_action(decision) };
        if action.is_null() {
            return NavigationRequestInfo {
                url: None,
                method: None,
            };
        }
        // SAFETY: The live navigation action owns its request for the signal duration.
        let request = unsafe { webkit_navigation_action_get_request(action) };
        if request.is_null() {
            return NavigationRequestInfo {
                url: None,
                method: None,
            };
        }
        // SAFETY: WebKit owns the returned nullable UTF-8 URI string for this callback.
        let uri_ptr = unsafe { webkit_uri_request_get_uri(request) };
        // SAFETY: WebKit owns the returned nullable UTF-8 HTTP method string for this callback.
        let method_ptr = unsafe { webkit_uri_request_get_http_method(request) };
        NavigationRequestInfo {
            url: cstr_to_string(uri_ptr).and_then(|uri| Url::parse(&uri).ok()),
            method: cstr_to_string(method_ptr),
        }
    }

    fn response_info(&self, decision: Object) -> ResponseInfo {
        if decision.is_null() {
            return ResponseInfo {
                url: None,
                mime_supported: false,
                content_disposition: None,
            };
        }
        // SAFETY: Supplied by WebKit during a response `decide-policy` emission.
        let response = unsafe { webkit_response_policy_decision_get_response(decision) };
        // SAFETY: Queries the live response policy decision.
        let mime_supported =
            unsafe { webkit_response_policy_decision_is_mime_type_supported(decision) }
                != ffi::FALSE;
        if response.is_null() {
            return ResponseInfo {
                url: None,
                mime_supported,
                content_disposition: None,
            };
        }
        // SAFETY: WebKit owns the returned nullable UTF-8 URI string for this callback.
        let uri_ptr = unsafe { webkit_uri_response_get_uri(response) };
        // SAFETY: Returns borrowed SoupMessageHeaders or null if non-HTTP.
        let headers = unsafe { webkit_uri_response_get_http_headers(response) };
        let content_disposition = if headers.is_null() {
            None
        } else {
            // SAFETY: `headers` is a live SoupMessageHeaders and the header name is static.
            let header =
                unsafe { soup_message_headers_get_one(headers, c"Content-Disposition".as_ptr()) };
            cstr_to_string(header)
        };
        ResponseInfo {
            url: cstr_to_string(uri_ptr).and_then(|uri| Url::parse(&uri).ok()),
            mime_supported,
            content_disposition,
        }
    }

    fn use_policy_decision(&self, decision: Object) {
        if decision.is_null() {
            return;
        }
        // SAFETY: `decision` is a live WebKitPolicyDecision during `decide-policy`.
        unsafe { webkit_policy_decision_use(decision) };
    }

    fn ignore_policy_decision(&self, decision: Object) {
        if decision.is_null() {
            return;
        }
        // SAFETY: `decision` is a live WebKitPolicyDecision during `decide-policy`.
        unsafe { webkit_policy_decision_ignore(decision) };
    }

    fn deny_permission_request(&self, request: Object) {
        if request.is_null() {
            return;
        }
        // SAFETY: `request` is a live WebKitPermissionRequest during `permission-request`.
        unsafe { webkit_permission_request_deny(request) };
    }
}

fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Caller guarantees `ptr` is a live NUL-terminated string for this callback.
    let value = unsafe { CStr::from_ptr(ptr) };
    Some(value.to_string_lossy().into_owned())
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
    fn g_object_new(object_type: usize, first_property_name: *const c_char, ...) -> Object;
    fn g_object_ref(object: Object) -> Object;
    fn g_object_ref_sink(object: Object) -> Object;
    fn g_signal_handler_disconnect(instance: Object, handler_id: c_ulong);
    fn g_signal_lookup(name: *const c_char, itype: usize) -> c_uint;
    fn g_type_class_ref(type_: usize) -> Object;
    fn g_type_class_unref(g_class: Object);
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_allocate(
        widget: Object,
        width: c_int,
        height: c_int,
        baseline: c_int,
        transform: Object,
    );
}

#[link(name = "soup-3.0")]
unsafe extern "C" {
    fn soup_message_headers_get_one(headers: Object, name: *const c_char) -> *const c_char;
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_automation_session_get_type() -> usize;
    fn webkit_web_view_get_type() -> usize;
    fn webkit_web_view_get_context(view: Object) -> Object;
    fn webkit_web_view_is_controlled_by_automation(view: Object) -> Bool;
    fn webkit_web_view_get_network_session(view: Object) -> Object;
    fn webkit_network_session_is_ephemeral(session: Object) -> Bool;
    fn webkit_web_view_stop_loading(view: Object);
    fn webkit_settings_set_javascript_can_open_windows_automatically(
        settings: Object,
        enabled: Bool,
    );
    fn webkit_navigation_policy_decision_get_navigation_action(decision: Object) -> Object;
    fn webkit_navigation_action_get_request(action: Object) -> Object;
    fn webkit_uri_request_get_uri(request: Object) -> *const c_char;
    fn webkit_uri_request_get_http_method(request: Object) -> *const c_char;
    fn webkit_response_policy_decision_get_response(decision: Object) -> Object;
    fn webkit_response_policy_decision_is_mime_type_supported(decision: Object) -> Bool;
    fn webkit_uri_response_get_uri(response: Object) -> *const c_char;
    fn webkit_uri_response_get_http_headers(response: Object) -> Object;
    fn webkit_policy_decision_use(decision: Object);
    fn webkit_policy_decision_ignore(decision: Object);
    fn webkit_permission_request_deny(request: Object);
}

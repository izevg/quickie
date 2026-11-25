//! The same inspector shortcut in the browser and the detached `WebKit` window.

use std::ffi::{CStr, c_uint};

use quickie_domain_commands::{FocusedView, Resolution, WhenContext};

use super::App;
use super::ffi::{self, Bool, Object};
use super::{connect, keys};

const OBSERVING_ROOT: &CStr = c"quickie-inspector-observing-root";
const APP: &CStr = c"quickie-inspector-application";
const TOGGLE_COMMAND: &str = "quickie.devtools.toggle";

pub(super) fn toggle(view: Object) {
    // SAFETY: The live WebKitWebView owns its inspector.
    let inspector = unsafe { ffi::webkit_web_view_get_inspector(view) };
    toggle_inspector(inspector);
}

pub(super) fn observe_detachment(view: Object, data: Object) {
    // SAFETY: The live WebKitWebView owns its inspector.
    let inspector = unsafe { ffi::webkit_web_view_get_inspector(view) };
    connect(
        inspector,
        c"open-window",
        observe_frontend_root as *const _,
        data,
    );
}

/// A docked Inspector whose native frontend has the keyboard owns its editor commands.
pub(super) fn docked_key_pressed(
    app: &App,
    view: Object,
    controller: Object,
    key: c_uint,
    hardware: c_uint,
    modifiers: c_uint,
) -> Option<Bool> {
    // SAFETY: The live page owns its Inspector.
    let inspector = unsafe { ffi::webkit_web_view_get_inspector(view) };
    // SAFETY: An open Inspector borrows its live frontend widget to the caller.
    let frontend = unsafe { ffi::webkit_web_inspector_get_web_view(inspector) };
    if frontend.is_null() {
        return None;
    }
    // SAFETY: The live page reports its owning GtkRoot.
    let root = unsafe { ffi::gtk_widget_get_root(view) };
    // SAFETY: The live frontend reports its GtkRoot if it is mounted.
    let frontend_root = unsafe { ffi::gtk_widget_get_root(frontend) };
    if root.is_null() || root != frontend_root {
        return None;
    }
    // SAFETY: The live GtkRoot owns its current focus widget, if any.
    let focus = unsafe { gtk_root_get_focus(root) };
    if focus.is_null() {
        return None;
    }
    // SAFETY: Both pointers refer to live widgets in the same root.
    let within_frontend = unsafe { gtk_widget_is_ancestor(focus, frontend) } != ffi::FALSE;
    (focus == frontend || within_frontend)
        .then(|| shortcut_pressed(app, inspector, controller, key, hardware, modifiers))
}

fn toggle_inspector(inspector: Object) {
    // SAFETY: The inspector belongs to a live WebKitWebView on this thread.
    let frontend = unsafe { ffi::webkit_web_inspector_get_web_view(inspector) };
    if frontend.is_null() {
        // SAFETY: The live inspector can create its own frontend on the GTK thread.
        unsafe { ffi::webkit_web_inspector_show(inspector) };
    } else {
        // SAFETY: The live inspector owns the frontend and handles its destruction.
        unsafe { ffi::webkit_web_inspector_close(inspector) };
    }
}

extern "C" fn observe_frontend_root(inspector: Object, data: Object) -> Bool {
    // SAFETY: WebKit emits open-window on the live inspector just before parenting its frontend.
    let frontend = unsafe { ffi::webkit_web_inspector_get_web_view(inspector) };
    if frontend.is_null() {
        return ffi::FALSE;
    }
    // SAFETY: The inspector belongs to a page whose App survives its signal handlers.
    unsafe { ffi::g_object_set_data(inspector, APP.as_ptr(), data) };
    // SAFETY: The borrowed frontend is a live GObject. The data key is static.
    let observing = unsafe { ffi::g_object_get_data(frontend, OBSERVING_ROOT.as_ptr()) };
    if observing.is_null() {
        // SAFETY: The live frontend uses its own pointer as an opaque marker for its lifetime.
        unsafe { ffi::g_object_set_data(frontend, OBSERVING_ROOT.as_ptr(), frontend) };
        // SAFETY: The frontend is a live GtkWidget, the handler has the notify signal ABI, and
        // GObject disconnects the handler when the inspector is destroyed.
        unsafe {
            ffi::g_signal_connect_object(
                frontend,
                c"notify::root".as_ptr(),
                frontend_root_changed as *const _,
                inspector,
                0,
            )
        };
    }
    ffi::FALSE // WebKit creates, docks, and destroys its own frontend window.
}

extern "C" fn frontend_root_changed(frontend: Object, _: Object, inspector: Object) {
    // SAFETY: This signal originates from the live GtkWidget.
    let root = unsafe { ffi::gtk_widget_get_root(frontend) };
    // SAFETY: The signal closure retains the live WebKit inspector during the callback.
    let attached = unsafe { ffi::webkit_web_inspector_is_attached(inspector) };
    if root.is_null() || attached == ffi::TRUE {
        return;
    }
    let controller = keys::capture_controller();
    // SAFETY: The key handler uses the GTK signal ABI and the closure tracks the live inspector.
    unsafe {
        ffi::g_signal_connect_object(
            controller,
            c"key-pressed".as_ptr(),
            detached_key_pressed as *const _,
            inspector,
            0,
        )
    };
    // SAFETY: GTK has rooted the frontend in its detached window, which adopts the controller.
    unsafe { ffi::gtk_widget_add_controller(root, controller) };
}

extern "C" fn detached_key_pressed(
    controller: Object,
    key: c_uint,
    hardware: c_uint,
    modifiers: c_uint,
    inspector: Object,
) -> Bool {
    // SAFETY: The live inspector stores the App that owns its originating page.
    let data = unsafe { ffi::g_object_get_data(inspector, APP.as_ptr()) };
    if data.is_null() {
        return ffi::FALSE;
    }
    // SAFETY: The App stays live until its page and its native callbacks are released.
    let app = unsafe { &*data.cast::<App>() };
    shortcut_pressed(app, inspector, controller, key, hardware, modifiers)
}

fn shortcut_pressed(
    app: &App,
    inspector: Object,
    controller: Object,
    key: c_uint,
    hardware: c_uint,
    modifiers: c_uint,
) -> Bool {
    if keys::is_modifier(key) {
        return ffi::FALSE;
    }
    let Some(stroke) = keys::read(controller, key, hardware, modifiers) else {
        app.keys.cancel_chord();
        return ffi::FALSE;
    };
    let context = WhenContext::new().in_view(FocusedView::Page);
    match app.keys.resolve_only(stroke, &context, TOGGLE_COMMAND) {
        Some(Resolution::Command(_)) => {
            toggle_inspector(inspector);
            ffi::TRUE
        }
        Some(Resolution::Pending(_) | Resolution::Unbound) => ffi::TRUE,
        None => ffi::FALSE,
    }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_root_get_focus(root: Object) -> Object;
    fn gtk_widget_is_ancestor(widget: Object, ancestor: Object) -> Bool;
}

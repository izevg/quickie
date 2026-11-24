//! The same inspector shortcut in the browser and the detached `WebKit` window.

use std::ffi::{CStr, c_uint};
use std::ptr;

use super::ffi::{self, Bool, Object};
use super::{connect, keys};

const OBSERVING_ROOT: &CStr = c"quickie-inspector-observing-root";
const CONTROL_SHIFT: c_uint = ffi::CONTROL | ffi::SHIFT;

pub(super) fn is_toggle(key: c_uint, modifiers: c_uint) -> bool {
    matches!(
        (key, modifiers & keys::SHORTCUT_MODIFIERS),
        (ffi::KEY_F12, 0) | (0x49 | 0x69, CONTROL_SHIFT)
    )
}

pub(super) fn toggle(view: Object) {
    // SAFETY: The live WebKitWebView owns its inspector.
    let inspector = unsafe { ffi::webkit_web_view_get_inspector(view) };
    toggle_inspector(inspector);
}

pub(super) fn observe_detachment(view: Object) {
    // SAFETY: The live WebKitWebView owns its inspector.
    let inspector = unsafe { ffi::webkit_web_view_get_inspector(view) };
    connect(
        inspector,
        c"open-window",
        observe_frontend_root as *const _,
        ptr::null_mut(),
    );
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

extern "C" fn observe_frontend_root(inspector: Object, _: Object) -> Bool {
    // SAFETY: WebKit emits open-window on the live inspector just before parenting its frontend.
    let frontend = unsafe { ffi::webkit_web_inspector_get_web_view(inspector) };
    if frontend.is_null() {
        return ffi::FALSE;
    }
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
    _: Object,
    key: c_uint,
    _: c_uint,
    modifiers: c_uint,
    inspector: Object,
) -> Bool {
    if !is_toggle(key, modifiers) {
        return ffi::FALSE;
    }
    toggle_inspector(inspector);
    ffi::TRUE
}

#[cfg(test)]
mod tests {
    use super::{CONTROL_SHIFT, ffi, is_toggle};

    #[test]
    fn f12_toggles_the_inspector_without_other_shortcut_modifiers() {
        assert!(is_toggle(ffi::KEY_F12, 0));
        for modifier in [ffi::CONTROL, ffi::SHIFT, ffi::ALT, ffi::SUPER] {
            assert!(!is_toggle(ffi::KEY_F12, modifier));
        }
    }

    #[test]
    fn the_existing_inspector_shortcut_works_with_either_letter_case() {
        assert!(is_toggle(u32::from(b'I'), CONTROL_SHIFT));
        assert!(is_toggle(u32::from(b'i'), CONTROL_SHIFT));
        assert!(!is_toggle(u32::from(b'i'), ffi::CONTROL));
    }
}

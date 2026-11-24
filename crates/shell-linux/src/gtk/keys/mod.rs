//! GTK key controllers and the modifiers used by browser shortcuts.

use std::ffi::c_uint;

use super::connect;
use super::ffi::{self, Bool, Object};

pub(super) const SHORTCUT_MODIFIERS: c_uint =
    ffi::SHIFT | ffi::CONTROL | ffi::ALT | ffi::SUPER | ffi::HYPER | ffi::META;

pub(super) fn capture_controller() -> Object {
    // SAFETY: Callers are on the GTK thread and transfer the new controller to a widget.
    let controller = unsafe { ffi::gtk_event_controller_key_new() };
    // SAFETY: The new controller is live and can be configured before attachment.
    unsafe { ffi::gtk_event_controller_set_propagation_phase(controller, ffi::PHASE_CAPTURE) };
    controller
}

pub(crate) fn dismiss_on_escape(window: Object) {
    let controller = capture_controller();
    connect(controller, c"key-pressed", dismiss as *const _, window);
    // SAFETY: The live window owns the controller and remains valid during its key events.
    unsafe { ffi::gtk_widget_add_controller(window, controller) };
}

extern "C" fn dismiss(
    _: Object,
    key: c_uint,
    _: c_uint,
    modifiers: c_uint,
    window: Object,
) -> Bool {
    if key != ffi::KEY_ESCAPE || modifiers & SHORTCUT_MODIFIERS != 0 {
        return ffi::FALSE;
    }
    // SAFETY: The window owns this controller and is live on the GTK thread during the callback.
    unsafe { ffi::gtk_window_close(window) };
    ffi::TRUE
}

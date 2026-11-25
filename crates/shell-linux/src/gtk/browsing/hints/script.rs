//! Runs link hints in an isolated `WebKit` script world.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use crate::gtk::ffi::{self, Object};

const WORLD: &CStr = c"dev.quickie.page.hints";
const TOP_FRAME: c_int = 1;
const DOCUMENT_START: c_int = 0;

pub(super) fn observe(view: Object) {
    // A document script keeps this named world alive across the two evaluations. This
    // preserves the actual DOM nodes chosen by collect even if the page has since changed.
    // SAFETY: The live view owns its content manager.
    let manager = unsafe { webkit_web_view_get_user_content_manager(view) };
    // SAFETY: WebKit copies the source and world strings. Null URL lists allow all origins.
    let script = unsafe {
        webkit_user_script_new_for_world(
            c"globalThis.__quickieHints = []".as_ptr(),
            TOP_FRAME,
            DOCUMENT_START,
            WORLD.as_ptr(),
            ptr::null(),
            ptr::null(),
        )
    };
    // SAFETY: The live manager retains the new script.
    unsafe { webkit_user_content_manager_add_script(manager, script) };
    // SAFETY: This releases the one reference still owned by this function.
    unsafe { webkit_user_script_unref(script) };
}

pub(super) fn evaluate<F: FnOnce(Object, &str) + 'static>(
    view: Object,
    source: &str,
    complete: F,
) -> bool {
    let Ok(source) = CString::new(source) else {
        return false;
    };
    let state = Box::into_raw(Box::new(complete));
    // SAFETY: The live view owns the GTK thread. WebKit copies the source and calls back once.
    unsafe {
        webkit_web_view_evaluate_javascript(
            view,
            source.as_ptr(),
            -1,
            WORLD.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            completed::<F>,
            state.cast(),
        );
    }
    true
}

extern "C" fn completed<F: FnOnce(Object, &str)>(view: Object, result: Object, data: Object) {
    // SAFETY: This callback takes back the boxed state supplied to the one-shot evaluation.
    let complete = unsafe { Box::from_raw(data.cast::<F>()) };
    // SAFETY: This is the live asynchronous result for this WebKit view.
    let value =
        unsafe { webkit_web_view_evaluate_javascript_finish(view, result, ptr::null_mut()) };
    if value.is_null() {
        complete(view, "");
        return;
    }
    // SAFETY: JSC returns an independent string that remains owned after releasing the value.
    let text = unsafe { jsc_value_to_string(value) };
    // SAFETY: The finish function transfers ownership of the non-null value.
    unsafe { ffi::g_object_unref(value) };
    if text.is_null() {
        complete(view, "");
        return;
    }
    // SAFETY: The live string is NUL-terminated and remains owned through the callback.
    let answer = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    complete(view, &answer);
    // SAFETY: JSC allocated the string with GLib's allocator.
    unsafe { g_free(text.cast()) };
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_get_user_content_manager(view: Object) -> Object;
    fn webkit_user_script_new_for_world(
        source: *const c_char,
        frames: c_int,
        time: c_int,
        world: *const c_char,
        allow: *const *const c_char,
        block: *const *const c_char,
    ) -> Object;
    fn webkit_user_content_manager_add_script(manager: Object, script: Object);
    fn webkit_user_script_unref(script: Object);
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

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(text: Object);
}

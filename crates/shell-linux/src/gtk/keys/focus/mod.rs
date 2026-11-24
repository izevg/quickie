//! Cached editable focus for each live `WebKit` view, reported from its isolated script world.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use crate::gtk::ffi::{self, Bool, Object};

const TRACKER: &CStr = c"quickie-keyboard-focus";
const WORLD: &CStr = c"dev.quickie.keyboard-focus";
const ALL_FRAMES: c_int = 0;
const DOCUMENT_START: c_int = 0;
type Frames = RefCell<BTreeSet<String>>;

pub(in crate::gtk) fn observe(view: Object) {
    let Ok(source) = CString::new(include_str!("watch.js")) else {
        return;
    };
    // SAFETY: The live web view owns its user content manager on this GTK thread.
    let manager = unsafe { webkit_web_view_get_user_content_manager(view) };
    // SAFETY: The live manager copies the static handler and isolated-world names.
    let registered = unsafe {
        webkit_user_content_manager_register_script_message_handler(
            manager,
            c"quickieKeyboardFocus".as_ptr(),
            WORLD.as_ptr(),
        )
    };
    if registered == ffi::FALSE {
        return;
    }
    let frames = Box::into_raw(Box::new(Frames::default()));
    // SAFETY: The view owns the editable-frame state until GTK calls drop_frames.
    unsafe { g_object_set_data_full(view, TRACKER.as_ptr(), frames.cast(), drop_frames) };
    // SAFETY: GTK disconnects this signal when the callback's bound view is destroyed.
    unsafe {
        ffi::g_signal_connect_object(
            manager,
            c"script-message-received::quickieKeyboardFocus".as_ptr(),
            reported as *const _,
            view,
            0,
        )
    };
    // SAFETY: WebKit copies the script and world strings, with both optional URL lists absent.
    let script = unsafe {
        webkit_user_script_new_for_world(
            source.as_ptr(),
            ALL_FRAMES,
            DOCUMENT_START,
            WORLD.as_ptr(),
            ptr::null(),
            ptr::null(),
        )
    };
    // SAFETY: The manager retains the script before we release our constructor reference.
    unsafe { webkit_user_content_manager_add_script(manager, script) };
    // SAFETY: This releases the one script reference still owned by this function.
    unsafe { webkit_user_script_unref(script) };
}

pub(in crate::gtk) fn editable(view: Object) -> bool {
    let frames = frames(view);
    if frames.is_null() {
        return false;
    }
    // SAFETY: The live view owns its frame set throughout this synchronous query.
    !unsafe { &*frames }.borrow().is_empty()
}

pub(in crate::gtk) fn reset(view: Object) {
    let frames = frames(view);
    if !frames.is_null() {
        // SAFETY: The live view owns its frame set throughout the navigation callback.
        unsafe { &*frames }.borrow_mut().clear();
    }
}

fn frames(view: Object) -> *const Frames {
    // SAFETY: The live view owns the pointer installed by observe, or has no tracker.
    unsafe { ffi::g_object_get_data(view, TRACKER.as_ptr()).cast::<Frames>() }
}

extern "C" fn reported(_: Object, value: Object, view: Object) {
    let frames = frames(view);
    if frames.is_null() {
        return;
    }
    // SAFETY: The live message value can provide an owned NUL-terminated string.
    let message = unsafe { jsc_value_to_string(value) };
    if message.is_null() {
        return;
    }
    // SAFETY: The non-null message is an owned string until g_free below.
    if let Ok(report) = unsafe { CStr::from_ptr(message) }.to_str() {
        // SAFETY: The current view owns this tracker for the callback.
        update(&mut unsafe { &*frames }.borrow_mut(), report);
    }
    // SAFETY: This releases the string allocated by JSC.
    unsafe { g_free(message.cast()) };
}

fn update(frames: &mut BTreeSet<String>, message: &str) {
    let mut parts = message.chars();
    let (Some(reset), Some(focused)) = (parts.next(), parts.next()) else {
        return;
    };
    let frame = parts.as_str();
    if !matches!(reset, 'R' | '-')
        || !matches!(focused, '0' | '1')
        || frame.is_empty()
        || frame.len() > 64
        || !frame.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return;
    }
    if reset == 'R' {
        frames.clear();
    }
    if focused == '1' {
        frames.insert(frame.to_owned());
    } else {
        frames.remove(frame);
    }
}

extern "C" fn drop_frames(frames: Object) {
    // SAFETY: GTK calls this once for the boxed tracker it owns.
    drop(unsafe { Box::from_raw(frames.cast::<Frames>()) });
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_get_user_content_manager(view: Object) -> Object;
    fn webkit_user_content_manager_register_script_message_handler(
        manager: Object,
        name: *const c_char,
        world: *const c_char,
    ) -> Bool;
    fn webkit_user_script_new_for_world(
        script: *const c_char,
        frames: c_int,
        when: c_int,
        world: *const c_char,
        allow_list: *const *const c_char,
        block_list: *const *const c_char,
    ) -> Object;
    fn webkit_user_content_manager_add_script(manager: Object, script: Object);
    fn webkit_user_script_unref(script: Object);
}

#[link(name = "javascriptcoregtk-6.0")]
unsafe extern "C" {
    fn jsc_value_to_string(value: Object) -> *mut c_char;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_set_data_full(
        object: Object,
        key: *const c_char,
        value: Object,
        drop: extern "C" fn(Object),
    );
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(allocation: Object);
}

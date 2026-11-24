//! Owned native objects for the document-start scripts and their isolated reply channel.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use quickie_engine_webkit::cosmetic_rules::CHANNEL;
use url::Url;

use crate::gtk::connect;
use crate::gtk::ffi::{self, Bool, Object};

const WORLD: &CStr = c"quickie-shields";
pub(super) const NAVIGATION: c_int = 0;
const ALL_FRAMES: c_int = 0;
const DOCUMENT_START: c_int = 0;

pub(super) struct Script(Object);

impl Script {
    pub(super) fn new(source: &str, isolated: bool) -> Option<Self> {
        let source = CString::new(source).ok()?;
        let script = if isolated {
            // SAFETY: Source and world are NUL-terminated. Both enums are supported.
            unsafe {
                webkit_user_script_new_for_world(
                    source.as_ptr(),
                    ALL_FRAMES,
                    DOCUMENT_START,
                    WORLD.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                )
            }
        } else {
            // SAFETY: Source is NUL-terminated. There are no match list restrictions.
            unsafe {
                webkit_user_script_new(
                    source.as_ptr(),
                    ALL_FRAMES,
                    DOCUMENT_START,
                    ptr::null(),
                    ptr::null(),
                )
            }
        };
        if script.is_null() {
            None
        } else {
            Some(Self(script))
        }
    }

    pub(super) fn add_to(&self, manager: Object) {
        // SAFETY: This object owns a live script and the view retains its manager.
        unsafe { webkit_user_content_manager_add_script(manager, self.0) };
    }

    pub(super) fn remove_from(&self, manager: Object) {
        // SAFETY: This live manager releases only its own reference to the script.
        unsafe { webkit_user_content_manager_remove_script(manager, self.0) };
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        // SAFETY: This drops the reference returned by the WebKit script constructor.
        unsafe { webkit_user_script_unref(self.0) };
    }
}

pub(super) struct Decision(Object);

impl Decision {
    pub(super) fn hold(decision: Object) -> Self {
        // SAFETY: The emitting policy signal owns a live GObject for this decision.
        Self(unsafe { g_object_ref(decision) })
    }

    pub(super) fn allow(self) {
        // SAFETY: The response has been retained and is marked used once.
        unsafe { webkit_policy_decision_use(self.0) };
    }

    pub(super) fn discard(self) {
        // SAFETY: The response has been retained and is marked ignored once.
        unsafe { webkit_policy_decision_ignore(self.0) };
    }
}

impl Drop for Decision {
    fn drop(&mut self) {
        // SAFETY: This releases only the retained decision reference.
        unsafe { ffi::g_object_unref(self.0) };
    }
}

pub(super) fn navigation(decision: Object) -> Option<Url> {
    // SAFETY: The caller checks the signal's NavigationAction decision kind.
    let action = unsafe { webkit_navigation_policy_decision_get_navigation_action(decision) };
    // SAFETY: The live action retains its request for the policy decision lifetime.
    let request = unsafe { webkit_navigation_action_get_request(action) };
    // SAFETY: The live request owns its URI text.
    let uri = unsafe { webkit_uri_request_get_uri(request) };
    if uri.is_null() {
        return None;
    }
    // SAFETY: The non-null pointer is readable until the WebKit request is released.
    let uri = unsafe { CStr::from_ptr(uri) }.to_str().ok()?;
    Url::parse(uri).ok()
}

pub(super) fn register(manager: Object, callback: *const std::ffi::c_void, data: Object) {
    let Ok(channel) = CString::new(CHANNEL) else {
        return;
    };
    let Ok(signal) = CString::new(format!("script-message-with-reply-received::{CHANNEL}")) else {
        return;
    };
    connect(manager, &signal, callback, data);
    // SAFETY: The live manager registers this channel exclusively for the named isolated world.
    unsafe {
        webkit_user_content_manager_register_script_message_handler_with_reply(
            manager,
            channel.as_ptr(),
            WORLD.as_ptr(),
        )
    };
}

pub(super) fn text(value: Object) -> Option<String> {
    // SAFETY: The reply signal supplies a live JSCValue on its original context.
    if unsafe { jsc_value_is_string(value) } == ffi::FALSE {
        return None;
    }
    // SAFETY: The confirmed string value returns an owned GLib string.
    let string = unsafe { jsc_value_to_string(value) };
    if string.is_null() {
        return None;
    }
    // SAFETY: The GLib string has a NUL terminator and remains live until g_free.
    let text = unsafe { CStr::from_ptr(string) }
        .to_str()
        .ok()
        .map(str::to_owned);
    // SAFETY: Release the source string allocated by JavaScriptCore.
    unsafe { g_free(string.cast()) };
    text
}

pub(super) struct Reply {
    reply: Object,
    context: Object,
}

impl Reply {
    pub(super) fn hold(value: Object, reply: Object) -> Self {
        // SAFETY: The emitted JSCValue retains a live context for this callback.
        let context = unsafe { jsc_value_get_context(value) };
        // SAFETY: Hold the context until the reply is sent from the UI thread.
        let context = unsafe { g_object_ref(context) };
        // SAFETY: This reply belongs to the message-with-reply signal and accepts a reference.
        let reply = unsafe { webkit_script_message_reply_ref(reply) };
        Self { reply, context }
    }

    pub(super) fn send(self, text: &str) {
        let text = CString::new(text).ok();
        let text = text.as_deref().unwrap_or(c"[]");
        // SAFETY: WebKit supplied the retained JSCContext and copies the UTF-8 string.
        let value = unsafe { jsc_value_new_string(self.context, text.as_ptr()) };
        // SAFETY: JS value and reply are live on their original GTK thread.
        unsafe { webkit_script_message_reply_return_value(self.reply, value) };
        // SAFETY: Release the new JSCValue after WebKit has encoded its return value.
        unsafe { ffi::g_object_unref(value) };
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        // SAFETY: Both handles were retained in hold and are released on the GTK thread.
        unsafe { webkit_script_message_reply_unref(self.reply) };
        // SAFETY: This is the owned reference to the original JSCContext.
        unsafe { ffi::g_object_unref(self.context) };
    }
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_user_script_new(
        source: *const c_char,
        frames: c_int,
        time: c_int,
        allow: *const *const c_char,
        block: *const *const c_char,
    ) -> Object;
    fn webkit_user_script_new_for_world(
        source: *const c_char,
        frames: c_int,
        time: c_int,
        world: *const c_char,
        allow: *const *const c_char,
        block: *const *const c_char,
    ) -> Object;
    fn webkit_user_script_unref(script: Object);
    fn webkit_user_content_manager_add_script(manager: Object, script: Object);
    fn webkit_user_content_manager_remove_script(manager: Object, script: Object);
    fn webkit_user_content_manager_register_script_message_handler_with_reply(
        manager: Object,
        name: *const c_char,
        world: *const c_char,
    ) -> Bool;
    fn webkit_script_message_reply_ref(reply: Object) -> Object;
    fn webkit_script_message_reply_unref(reply: Object);
    fn webkit_script_message_reply_return_value(reply: Object, value: Object);
    fn webkit_navigation_policy_decision_get_navigation_action(decision: Object) -> Object;
    fn webkit_navigation_action_get_request(action: Object) -> Object;
    fn webkit_uri_request_get_uri(request: Object) -> *const c_char;
    fn webkit_policy_decision_use(decision: Object);
    fn webkit_policy_decision_ignore(decision: Object);
}

#[link(name = "javascriptcoregtk-6.0")]
unsafe extern "C" {
    fn jsc_value_is_string(value: Object) -> Bool;
    fn jsc_value_to_string(value: Object) -> *mut c_char;
    fn jsc_value_get_context(value: Object) -> Object;
    fn jsc_value_new_string(context: Object, text: *const c_char) -> Object;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref(object: Object) -> Object;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(memory: Object);
}

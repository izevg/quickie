//! Reports and controls for the main frame's video in a private `WebKit` script world.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use quickie_domain_browsing::Video;

use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{App, connect};

const WORLD: &CStr = c"dev.quickie.page.video";
const APP: &CStr = c"quickie-media-app";
const TOP_FRAME: c_int = 1;
const DOCUMENT_START: c_int = 0;
const VIDEO: &str =
    include_str!("../../../../../adapters/engine-webkit/src/page/scripts/video/video.js");

pub(super) fn bind(view: Object, data: Object) {
    let Ok(source) = CString::new(VIDEO) else {
        return;
    };
    // SAFETY: The new view owns its live user content manager.
    let manager = unsafe { webkit_web_view_get_user_content_manager(view) };
    // SAFETY: The live application owns data for the lifetime of the connected signal.
    unsafe { ffi::g_object_set_data(view, APP.as_ptr(), data) };
    // SAFETY: The manager uses the callback ABI of this signal and disconnects it when the
    // bound view is destroyed.
    unsafe {
        ffi::g_signal_connect_object(
            manager,
            c"script-message-received::quickieVideo".as_ptr(),
            reported as *const _,
            view,
            0,
        )
    };
    // SAFETY: The manager is live and WebKit copies both static strings.
    if unsafe {
        webkit_user_content_manager_register_script_message_handler(
            manager,
            c"quickieVideo".as_ptr(),
            WORLD.as_ptr(),
        )
    } == ffi::FALSE
    {
        return;
    }
    // SAFETY: WebKit copies the live strings. Null allow and block lists permit all origins.
    let script = unsafe {
        webkit_user_script_new_for_world(
            source.as_ptr(),
            TOP_FRAME,
            DOCUMENT_START,
            WORLD.as_ptr(),
            ptr::null(),
            ptr::null(),
        )
    };
    // SAFETY: The live manager retains the new script.
    unsafe { webkit_user_content_manager_add_script(manager, script) };
    // SAFETY: This releases the reference from the constructor.
    unsafe { webkit_user_script_unref(script) };
    connect(
        view,
        c"web-process-terminated",
        terminated as *const _,
        data,
    );
}

pub(super) fn enter_player(view: Object, app: &App, revision: u64) {
    let context = Box::new(FloatRequest {
        app: ptr::from_ref(app),
        revision,
    });
    evaluate(
        view,
        c"Boolean(globalThis.quickieVideo?.float())",
        floated,
        Box::into_raw(context).cast(),
    );
}

pub(super) fn dock(view: Object) {
    run(view, c"globalThis.quickieVideo?.dock(); void 0");
}

pub(super) fn play(view: Object, play: bool) {
    let script = if play {
        c"globalThis.quickieVideo?.play(true); void 0"
    } else {
        c"globalThis.quickieVideo?.play(false); void 0"
    };
    run(view, script);
}

pub(super) fn exit_fullscreen(view: Object) {
    run(
        view,
        c"if (document.fullscreenElement) void document.exitFullscreen()",
    );
}

fn run(view: Object, script: &CStr) {
    evaluate(view, script, discard, ptr::null_mut());
}

fn evaluate(
    view: Object,
    source: &CStr,
    finish: extern "C" fn(Object, Object, Object),
    data: Object,
) {
    // SAFETY: The live view's owning thread issues the evaluation. WebKit copies the script,
    // then invokes the completion callback once with its data.
    unsafe {
        webkit_web_view_evaluate_javascript(
            view,
            source.as_ptr(),
            -1,
            WORLD.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            finish,
            data,
        );
    }
}

struct FloatRequest {
    app: *const App,
    revision: u64,
}

extern "C" fn floated(view: Object, result: Object, context: Object) {
    // SAFETY: This callback owns the evaluation's boxed context. App lives on the GTK thread
    // for the duration of its loop, which is the only context that dispatches this callback.
    let request = unsafe { Box::from_raw(context.cast::<FloatRequest>()) };
    // SAFETY: The result is live and its new value, when present, belongs to the caller.
    let value =
        unsafe { webkit_web_view_evaluate_javascript_finish(view, result, ptr::null_mut()) };
    let ready = if value.is_null() {
        false
    } else {
        // SAFETY: The live value represents the Boolean result of the isolated script.
        let ready = unsafe { jsc_value_to_boolean(value) != ffi::FALSE };
        // SAFETY: This consumes the finish call's reference.
        unsafe { ffi::g_object_unref(value) };
        ready
    };
    // SAFETY: The originating App outlives this callback on its loop.
    let app = unsafe { &*request.app };
    app.browsing
        .media
        .player
        .float_finished(view, request.revision, ready);
}

extern "C" fn discard(view: Object, result: Object, _: Object) {
    // SAFETY: The result is live and its new value, when present, belongs to the caller.
    let value =
        unsafe { webkit_web_view_evaluate_javascript_finish(view, result, ptr::null_mut()) };
    if !value.is_null() {
        // SAFETY: This consumes the finish call's reference.
        unsafe { ffi::g_object_unref(value) };
    }
}

extern "C" fn reported(_: Object, value: Object, view: Object) {
    // SAFETY: WebKit's private script sends strings. JSC returns an owned NUL-terminated copy.
    let message = unsafe { jsc_value_to_string(value) };
    if message.is_null() {
        return;
    }
    // SAFETY: The owned string lasts until it is freed just below.
    let video = match unsafe { CStr::from_ptr(message) }.to_bytes().first() {
        Some(b'A') => Some(Some(Video::Playing { audible: true })),
        Some(b'M') => Some(Some(Video::Playing { audible: false })),
        Some(b'P') => Some(Some(Video::Paused)),
        Some(b'N' | b'X') => Some(None),
        _ => None,
    };
    // SAFETY: JSC allocated this string using GLib's allocator.
    unsafe { g_free(message.cast()) };
    if let Some(video) = video {
        // SAFETY: The weak connection is tied to the live view and runs on its GTK thread.
        let data = unsafe { ffi::g_object_get_data(view, APP.as_ptr()) };
        report(view, data, video);
    }
}

extern "C" fn terminated(view: Object, _: c_int, data: Object) {
    report(view, data, None);
}

fn report(view: Object, data: Object, video: Option<Video>) {
    if !data.is_null() {
        // SAFETY: The view was bound with the live App pointer on its GTK thread.
        let app = unsafe { &*data.cast::<App>() };
        if let Some(tab) = app.browsing.pages.tab(view) {
            app.browsing.media.player.reported(tab, video);
        }
    }
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
    fn webkit_web_view_evaluate_javascript(
        view: Object,
        script: *const c_char,
        length: isize,
        world: *const c_char,
        source_uri: *const c_char,
        cancellable: Object,
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
    fn jsc_value_to_boolean(value: Object) -> Bool;
    fn jsc_value_to_string(value: Object) -> *mut c_char;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(memory: Object);
}

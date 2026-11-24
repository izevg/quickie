//! Reads the rendered main-frame text in a `WebKit` script world separate from the page.

use std::ffi::{CStr, CString, c_char};
use std::ptr;
use std::sync::mpsc::{self, Receiver, Sender};

use quickie_application::PageReader;
use url::Url;

use crate::gtk::ffi::{self, Bool, Object};

type ResultText = Result<String, String>;

pub(super) struct Read {
    cancel: Object,
    pub(super) results: Receiver<ResultText>,
}

impl Read {
    pub(super) fn start(view: Object) -> Result<Self, String> {
        let script = format!(
            "String((document.body || document.documentElement)?.innerText || '').slice(0, {})",
            PageReader::MAX_CHARS,
        );
        let script = CString::new(script).map_err(|_| "Could not prepare the page read")?;
        // SAFETY: This creates an owned cancellable for the single evaluation.
        let cancel = unsafe { g_cancellable_new() };
        let (send, results) = mpsc::channel::<ResultText>();
        let data = Box::into_raw(Box::new(send)).cast();
        // SAFETY: WebKit copies the script and calls completed once on the GTK context.
        // The closure owns only a Sender and can safely outlive the agent workspace.
        unsafe {
            webkit_web_view_evaluate_javascript(
                view,
                script.as_ptr(),
                -1,
                c"dev.quickie.agent.read".as_ptr(),
                ptr::null(),
                cancel,
                completed,
                data,
            );
        }
        Ok(Self { cancel, results })
    }
}

impl Drop for Read {
    fn drop(&mut self) {
        // SAFETY: The live cancellable belongs to this evaluation.
        unsafe { g_cancellable_cancel(self.cancel) };
        // SAFETY: This releases our cancellable reference after signalling cancellation.
        unsafe { ffi::g_object_unref(self.cancel) };
    }
}

pub(super) fn location(view: Object) -> Option<Url> {
    // SAFETY: The live WebKit view supplies a nullable borrowed URI on its owning GTK thread.
    let uri = unsafe { ffi::webkit_web_view_get_uri(view) };
    if uri.is_null() {
        return None;
    }
    // SAFETY: The non-null NUL-terminated URI is borrowed synchronously.
    Url::parse(unsafe { CStr::from_ptr(uri) }.to_str().ok()?).ok()
}

pub(super) fn source(view: Object) -> Result<(Url, String), String> {
    // SAFETY: The live view is on its GTK thread.
    if unsafe { webkit_web_view_is_loading(view) } != ffi::FALSE {
        return Err("Wait for the source page to finish loading".into());
    }
    let url = location(view).ok_or("Open a page to ask about it")?;
    // SAFETY: The live WebKit view owns the nullable title on its GTK thread.
    let title = unsafe { ffi::webkit_web_view_get_title(view) };
    let title = if title.is_null() {
        String::new()
    } else {
        // SAFETY: The non-null title is copied before returning to WebKit.
        unsafe { CStr::from_ptr(title) }
            .to_string_lossy()
            .into_owned()
    };
    Ok((url, title))
}

extern "C" fn completed(view: Object, result: Object, data: Object) {
    // SAFETY: The evaluation callback owns the unique boxed sender from start.
    let send = unsafe { Box::from_raw(data.cast::<Sender<ResultText>>()) };
    // SAFETY: Both are live callback arguments. Omitting the error suppresses script details.
    let value =
        unsafe { webkit_web_view_evaluate_javascript_finish(view, result, ptr::null_mut()) };
    if value.is_null() {
        let _ = send.send(Err("Could not read the source page".into()));
        return;
    }
    // SAFETY: The finish method transfers ownership of the JSCValue. JSC creates a new string.
    let text = unsafe { jsc_value_to_string(value) };
    // SAFETY: This is the owned JSCValue returned by the finish method.
    unsafe { ffi::g_object_unref(value) };
    let outcome = if text.is_null() {
        Err("Could not read the page text".into())
    } else {
        // SAFETY: The owned string is NUL-terminated and is freed after copying.
        let copy = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: JSC allocated this string with GLib's allocator.
        unsafe { g_free(text.cast()) };
        Ok(copy)
    };
    let _ = send.send(outcome);
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_is_loading(view: Object) -> Bool;
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
    fn g_free(memory: Object);
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_cancellable_new() -> Object;
    fn g_cancellable_cancel(cancel: Object);
}

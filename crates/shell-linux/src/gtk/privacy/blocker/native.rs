use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::ptr;
use std::rc::Rc;

use tempfile::TempDir;

use super::Built;
use crate::gtk::ffi::{GError, Object, g_object_unref};

/// Holds one reference to a compiled `WebKit` content filter, on the GTK thread.
pub(super) struct Filter(Object);

impl Filter {
    pub(super) fn add_to(&self, manager: Object) {
        // SAFETY: This filter owns its reference and the view owns its live content manager.
        unsafe { webkit_user_content_manager_add_filter(manager, self.0) };
    }

    pub(super) fn remove_from(&self, manager: Object) {
        // SAFETY: Both objects are live. The content manager drops only its own reference.
        unsafe { webkit_user_content_manager_remove_filter(manager, self.0) };
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        // SAFETY: This releases the owned reference returned by WebKit's finish API.
        unsafe { webkit_user_content_filter_unref(self.0) };
    }
}

struct Batch {
    remaining: Cell<usize>,
    filters: RefCell<Vec<Option<Filter>>>,
    error: RefCell<Option<String>>,
    result: Built,
    _directory: TempDir,
}

struct Part {
    batch: Rc<Batch>,
    index: usize,
}

pub(super) fn content_manager(view: Object) -> Object {
    // SAFETY: The live view retains its user content manager for its lifetime.
    unsafe { webkit_web_view_get_user_content_manager(view) }
}

pub(super) fn compile(name: &str, source: Vec<(String, usize)>) -> Built {
    let result = Rc::new(RefCell::new(None));
    match start(name, source, result.clone()) {
        Ok(()) => {}
        Err(error) => *result.borrow_mut() = Some(Err(error)),
    }
    result
}

fn start(name: &str, source: Vec<(String, usize)>, result: Built) -> Result<(), String> {
    let directory = tempfile::Builder::new()
        .prefix("quickie-shields-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let path =
        CString::new(directory.path().as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    let documents: Result<Vec<_>, _> = source
        .into_iter()
        .enumerate()
        .map(|(index, (json, _))| {
            CString::new(format!("quickie-{name}-{index}")).map(|id| (id, json))
        })
        .collect();
    let documents = documents.map_err(|error| error.to_string())?;
    let count = documents.len();
    let batch = Rc::new(Batch {
        remaining: Cell::new(count),
        filters: RefCell::new((0..count).map(|_| None).collect()),
        error: RefCell::new(None),
        result,
        _directory: directory,
    });
    // SAFETY: GLib copies the NUL-terminated path to the private directory.
    let store = unsafe { webkit_user_content_filter_store_new(path.as_ptr()) };
    for (index, (id, json)) in documents.into_iter().enumerate() {
        let part = Box::into_raw(Box::new(Part {
            batch: batch.clone(),
            index,
        }));
        // SAFETY: GBytes copies these bytes before the JSON string goes out of scope.
        let bytes = unsafe { g_bytes_new(json.as_ptr().cast(), json.len()) };
        // SAFETY: WebKit retains the live store and bytes for this async task.
        unsafe {
            webkit_user_content_filter_store_save(
                store,
                id.as_ptr(),
                bytes,
                ptr::null_mut(),
                compiled,
                part.cast(),
            );
        }
        // SAFETY: This releases the GLib reference created for this iteration.
        unsafe { g_bytes_unref(bytes) };
    }
    // SAFETY: Each running GAsyncResult owns its store reference until its callback completes.
    unsafe { g_object_unref(store) };
    Ok(())
}

extern "C" fn compiled(store: Object, result: Object, part: Object) {
    // SAFETY: WebKit returns each Box exactly once through its async callback.
    let part = unsafe { Box::from_raw(part.cast::<Part>()) };
    let mut error = ptr::null_mut();
    // SAFETY: WebKit supplied the original store and result, and error is a writable out-pointer.
    let filter =
        unsafe { webkit_user_content_filter_store_save_finish(store, result, &raw mut error) };
    let batch = &part.batch;
    if filter.is_null() {
        *batch.error.borrow_mut() = Some(error_message(error));
    } else {
        batch.filters.borrow_mut()[part.index] = Some(Filter(filter));
    }
    let remaining = batch.remaining.get() - 1;
    batch.remaining.set(remaining);
    if remaining == 0 {
        let outcome = batch.error.borrow_mut().take().map_or_else(
            || Ok(batch.filters.take().into_iter().flatten().collect()),
            Err,
        );
        *batch.result.borrow_mut() = Some(outcome);
    }
}

fn error_message(error: *mut GError) -> String {
    if error.is_null() {
        return "WebKit could not compile the filter rules".to_owned();
    }
    // SAFETY: The non-null GError is owned by this caller until freed.
    let message = unsafe { (*error).message };
    // SAFETY: GLib's GError includes a NUL-terminated message until freed.
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: Release the caller-owned GError exactly once.
    unsafe { g_error_free(error) };
    message
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_bytes_new(data: *const c_void, size: usize) -> Object;
    fn g_bytes_unref(bytes: Object);
    fn g_error_free(error: *mut GError);
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_get_user_content_manager(view: Object) -> Object;
    fn webkit_user_content_manager_add_filter(manager: Object, filter: Object);
    fn webkit_user_content_manager_remove_filter(manager: Object, filter: Object);
    fn webkit_user_content_filter_unref(filter: Object);
    fn webkit_user_content_filter_store_new(path: *const c_char) -> Object;
    fn webkit_user_content_filter_store_save(
        store: Object,
        identifier: *const c_char,
        source: Object,
        cancellable: Object,
        callback: extern "C" fn(Object, Object, Object),
        data: Object,
    );
    fn webkit_user_content_filter_store_save_finish(
        store: Object,
        result: Object,
        error: *mut *mut GError,
    ) -> Object;
}

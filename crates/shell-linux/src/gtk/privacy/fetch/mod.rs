//! Fetches approved list URLs through an ephemeral `WebKit` network session.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_uint};
use std::fs::File;
use std::io::Read as _;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use quickie_application::{FetchedList, HttpError, ListFetch};
use quickie_domain_shields::FilterListId;
use tempfile::TempDir;

use crate::gtk::{connect, ffi};
use ffi::{Bool, GError, Object};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

type Body = Result<Vec<u8>, HttpError>;

#[derive(Default)]
pub(super) struct Fetches {
    downloads: Vec<Download>,
    session: Option<Session>,
}

impl Fetches {
    pub(super) fn start(&mut self, fetches: Vec<ListFetch>) -> Vec<FetchedList> {
        if fetches.is_empty() {
            return Vec::new();
        }
        let session = self.session.get_or_insert_with(|| {
            // SAFETY: This runs on the GTK thread after WebKit initialization.
            Session(unsafe { webkit_network_session_new_ephemeral() })
        });
        let mut errors = Vec::new();
        for fetch in fetches {
            if self
                .downloads
                .iter()
                .any(|download| download.list == fetch.list)
            {
                continue;
            }
            match Download::start(session.0, &fetch) {
                Ok(download) => self.downloads.push(download),
                Err(error) => errors.push(FetchedList {
                    list: fetch.list,
                    body: Err(error),
                }),
            }
        }
        errors
    }

    pub(super) fn active(&self) -> bool {
        !self.downloads.is_empty()
    }

    pub(super) fn poll(&mut self) -> Vec<FetchedList> {
        let mut done = Vec::new();
        self.downloads.retain(|download| {
            if let Some(body) = download.result() {
                done.push(FetchedList {
                    list: download.list,
                    body,
                });
                false
            } else {
                true
            }
        });
        done
    }
}

struct Session(Object);

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: This releases the owned ephemeral network session on the GTK thread.
        unsafe { ffi::g_object_unref(self.0) };
    }
}

struct Download {
    object: Object,
    data: Rc<Request>,
    list: FilterListId,
    started: Instant,
}

struct Request {
    destination: CString,
    directory: RefCell<Option<TempDir>>,
    failure: RefCell<Option<HttpError>>,
    reader: RefCell<Option<Receiver<Body>>>,
    finished: Cell<bool>,
}

impl Download {
    fn start(session: Object, fetch: &ListFetch) -> Result<Self, HttpError> {
        use std::os::unix::ffi::OsStrExt as _;
        let directory = tempfile::Builder::new()
            .prefix("quickie-filter-list-")
            .tempdir()
            .map_err(|error| HttpError(error.to_string()))?;
        let destination = CString::new(directory.path().join("list").as_os_str().as_bytes())
            .map_err(|error| HttpError(error.to_string()))?;
        let address =
            CString::new(fetch.url.as_str()).map_err(|error| HttpError(error.to_string()))?;
        // SAFETY: A new download in this live ephemeral session owns a copy of the HTTPS URI.
        let object = unsafe { webkit_network_session_download_uri(session, address.as_ptr()) };
        if object.is_null() {
            return Err(HttpError("WebKit could not start the download".to_owned()));
        }
        let data = Rc::new(Request {
            destination,
            directory: RefCell::new(Some(directory)),
            failure: RefCell::new(None),
            reader: RefCell::new(None),
            finished: Cell::new(false),
        });
        let callback = Rc::into_raw(data.clone()).cast_mut().cast();
        // SAFETY: The live download owns one Rc strong reference for as long as it can emit a signal.
        unsafe {
            g_object_set_data_full(object, c"quickie-filter-list".as_ptr(), callback, release);
        };
        connect(
            object,
            c"decide-destination",
            decide_destination as *const _,
            callback,
        );
        connect(object, c"received-data", received as *const _, callback);
        connect(object, c"failed", failed as *const _, callback);
        connect(object, c"finished", finished as *const _, callback);
        Ok(Self {
            object,
            data,
            list: fetch.list,
            started: Instant::now(),
        })
    }

    fn result(&self) -> Option<Body> {
        if let Some(error) = self.data.failure.borrow_mut().take() {
            return Some(Err(error));
        }
        if let Some(reader) = self.data.reader.borrow().as_ref() {
            match reader.try_recv() {
                Ok(body) => return Some(body),
                Err(TryRecvError::Disconnected) => {
                    return Some(Err(HttpError("The list reader stopped".to_owned())));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        (self.started.elapsed() >= TIMEOUT)
            .then(|| Err(HttpError("The filter list request timed out".to_owned())))
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        if !self.data.finished.get() {
            // SAFETY: The download and callback data are live until the reference is released.
            unsafe { webkit_download_cancel(self.object) };
        }
        // SAFETY: This releases the owned download reference on the GTK thread.
        unsafe { ffi::g_object_unref(self.object) };
    }
}

extern "C" fn release(data: Object) {
    // SAFETY: GObject passes the strong reference installed with g_object_set_data_full once.
    drop(unsafe { Rc::from_raw(data.cast::<Request>()) });
}

extern "C" fn decide_destination(download: Object, _: *const c_char, data: Object) -> Bool {
    // SAFETY: The connected download owns this live request through every callback.
    let request = unsafe { &*data.cast::<Request>() };
    // SAFETY: The emitting download owns the response and its URI for this callback.
    let response = unsafe { webkit_download_get_response(download) };
    if response.is_null() {
        request.failure.replace(Some(HttpError(
            "The filter list response is missing".to_owned(),
        )));
    } else {
        // SAFETY: This is the live response of the emitting download.
        let status = unsafe { webkit_uri_response_get_status_code(response) };
        // SAFETY: This is the live response of the emitting download.
        let uri = unsafe { webkit_uri_response_get_uri(response) };
        // SAFETY: For a non-null pointer the response owns the NUL-terminated URI.
        let secure = !uri.is_null()
            && unsafe { CStr::from_ptr(uri) }
                .to_bytes()
                .starts_with(b"https://");
        if (200..300).contains(&status) && secure {
            // SAFETY: WebKitGTK 6.0 copies the private native path as its destination.
            unsafe { webkit_download_set_destination(download, request.destination.as_ptr()) };
            return ffi::TRUE;
        }
        request.failure.replace(Some(HttpError(format!(
            "The HTTPS request returned status {status}"
        ))));
    }
    // SAFETY: The current request cannot be used, so stop it on the live emitting download.
    unsafe { webkit_download_cancel(download) };
    ffi::TRUE
}

extern "C" fn received(download: Object, _: u64, data: Object) {
    // SAFETY: The connected download owns this live request through every callback.
    let request = unsafe { &*data.cast::<Request>() };
    // SAFETY: WebKit supplies the byte count of the live emitting download.
    if unsafe { webkit_download_get_received_data_length(download) } > MAX_BYTES {
        request
            .failure
            .replace(Some(HttpError("The filter list exceeds 16 MiB".to_owned())));
        // SAFETY: Stop this live download before it writes any more bytes.
        unsafe { webkit_download_cancel(download) };
    }
}

extern "C" fn failed(_: Object, error: *const GError, data: Object) {
    // SAFETY: The connected download owns this live request through every callback.
    let request = unsafe { &*data.cast::<Request>() };
    // SAFETY: When supplied, the GError and its message are borrowed for this callback.
    let detail = unsafe { error.as_ref() }.map_or_else(
        || "The filter list request failed".to_owned(),
        |error| {
            if error.message.is_null() {
                return "The filter list request failed".to_owned();
            }
            // SAFETY: GLib supplies the NUL-terminated message with its live error.
            unsafe { CStr::from_ptr(error.message) }
                .to_string_lossy()
                .into_owned()
        },
    );
    request
        .failure
        .borrow_mut()
        .get_or_insert(HttpError(detail));
}

extern "C" fn finished(_: Object, data: Object) {
    // SAFETY: The connected download owns this live request through every callback.
    let request = unsafe { &*data.cast::<Request>() };
    request.finished.set(true);
    if request.failure.borrow().is_some() {
        return;
    }
    let Some(directory) = request.directory.borrow_mut().take() else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    match std::thread::Builder::new()
        .name("quickie-filter-list".to_owned())
        .spawn(move || {
            let body = read(&directory);
            let _ = sender.send(body);
        }) {
        Ok(_) => *request.reader.borrow_mut() = Some(receiver),
        Err(error) => *request.failure.borrow_mut() = Some(HttpError(error.to_string())),
    }
}

fn read(directory: &TempDir) -> Body {
    let file =
        File::open(directory.path().join("list")).map_err(|error| HttpError(error.to_string()))?;
    let mut body = Vec::new();
    let count = file
        .take(MAX_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| HttpError(error.to_string()))?;
    if count as u64 > MAX_BYTES {
        return Err(HttpError("The filter list exceeds 16 MiB".to_owned()));
    }
    Ok(body)
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_set_data_full(
        object: Object,
        key: *const c_char,
        data: Object,
        destroy: extern "C" fn(Object),
    );
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_network_session_new_ephemeral() -> Object;
    fn webkit_network_session_download_uri(session: Object, uri: *const c_char) -> Object;
    fn webkit_download_get_response(download: Object) -> Object;
    fn webkit_uri_response_get_status_code(response: Object) -> c_uint;
    fn webkit_uri_response_get_uri(response: Object) -> *const c_char;
    fn webkit_download_set_destination(download: Object, path: *const c_char);
    fn webkit_download_get_received_data_length(download: Object) -> u64;
    fn webkit_download_cancel(download: Object);
}

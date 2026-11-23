//! Isolated update requests through the system libsoup stack used by `WebKitGTK`.

#![expect(unsafe_code, reason = "libsoup and GIO are native libraries")]

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::io;
use std::ptr::{self, NonNull};

use quickie_updater::{Response, Transport};
use url::Url;

const NO_REDIRECT: c_uint = 1 << 1;
const TIMEOUT_SECONDS: c_uint = 30;

/// Shared cancellation for the GTK thread and one background transfer.
pub struct Cancellation(Native);

// SAFETY: GCancellable's API is thread-safe, and the shared Rust owner retains its reference.
unsafe impl Send for Cancellation {}
// SAFETY: GCancellable may be cancelled and queried concurrently.
unsafe impl Sync for Cancellation {}

impl Cancellation {
    pub fn new() -> Result<Self, String> {
        // SAFETY: GIO creates an owned GCancellable without caller-provided state.
        Native::owned(unsafe { g_cancellable_new() })
            .map(Self)
            .ok_or_else(|| "could not create an update cancellation handle".into())
    }

    pub fn cancel(&self) {
        // SAFETY: This is a live GCancellable, which may be cancelled on any thread.
        unsafe { g_cancellable_cancel(self.pointer()) };
    }

    pub fn is_cancelled(&self) -> bool {
        // SAFETY: This is a live GCancellable, which may be queried on any thread.
        unsafe { g_cancellable_is_cancelled(self.pointer()) != 0 }
    }

    fn pointer(&self) -> *mut c_void {
        self.0.pointer()
    }
}

pub struct SoupTransport<'a>(pub &'a Cancellation);

impl Transport for SoupTransport<'_> {
    fn get(
        &self,
        url: &Url,
        accept: &str,
        body: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<Response, String> {
        if self.0.is_cancelled() {
            return Err("the update request was cancelled".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("update requests cannot contain credentials".into());
        }
        let url = CString::new(url.as_str()).map_err(|_| "invalid update request URL")?;
        let accept = CString::new(accept).map_err(|_| "invalid update request headers")?;
        let session = session()?;
        let message = message(&url, &accept)?;
        // SAFETY: Session and message are live and local to this worker. The cancellable
        // outlives the call. Passing a null error slot discards potentially sensitive URLs.
        let stream = unsafe {
            soup_session_send(
                session.pointer(),
                message.pointer(),
                self.0.pointer(),
                ptr::null_mut(),
            )
        };
        let stream = Native::owned(stream).ok_or("could not connect to the update server")?;
        // SAFETY: Sending the live message has populated its response status.
        let status = u16::try_from(unsafe { soup_message_get_status(message.pointer()) })
            .map_err(|_| "invalid update response status")?;
        let location = if (300..400).contains(&status) {
            redirect_location(&message)
        } else {
            None
        };
        if status == 200 {
            self.read_body(&stream, body)?;
        }
        Ok(Response { status, location })
    }
}

impl SoupTransport<'_> {
    fn read_body(
        &self,
        stream: &Native,
        body: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<(), String> {
        let mut buffer = [0_u8; 8192];
        loop {
            // SAFETY: The live input stream writes at most buffer.len() bytes and neither it
            // nor the cancellable retains the writable buffer. GIO owns any error it discards.
            let count = unsafe {
                g_input_stream_read(
                    stream.pointer(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    self.0.pointer(),
                    ptr::null_mut(),
                )
            };
            let count = usize::try_from(count).map_err(|_| "could not read the update response")?;
            if count == 0 {
                return Ok(());
            }
            body(&buffer[..count]).map_err(|error| error.to_string())?;
        }
    }
}

fn session() -> Result<Native, String> {
    // SAFETY: This creates an owned session used only for a single request on its worker.
    let session = Native::owned(unsafe { soup_session_new() })
        .ok_or("could not initialize the update request")?;
    // SAFETY: This obtains libsoup's registered authentication feature type.
    let auth_manager = unsafe { soup_auth_manager_get_type() };
    // SAFETY: The new session has no cookie jar. Removing the auth feature disables challenges.
    unsafe { soup_session_remove_feature_by_type(session.pointer(), auth_manager) };
    // SAFETY: The live session has not established a connection.
    unsafe { soup_session_set_timeout(session.pointer(), TIMEOUT_SECONDS) };
    // SAFETY: The live session copies the static user agent.
    unsafe { soup_session_set_user_agent(session.pointer(), c"Quickie".as_ptr()) };
    Ok(session)
}

fn message(url: &CStr, accept: &CStr) -> Result<Native, String> {
    // SAFETY: The NUL-terminated method and URL outlive construction, which copies them.
    let message = Native::owned(unsafe { soup_message_new(c"GET".as_ptr(), url.as_ptr()) })
        .ok_or("could not prepare the update request")?;
    // SAFETY: The live message has not been queued.
    unsafe { soup_message_set_flags(message.pointer(), NO_REDIRECT) };
    // SAFETY: The headers are owned by the live message.
    let headers = unsafe { soup_message_get_request_headers(message.pointer()) };
    // SAFETY: The live headers copy the NUL-terminated name and value.
    unsafe { soup_message_headers_replace(headers, c"Accept".as_ptr(), accept.as_ptr()) };
    Ok(message)
}

fn redirect_location(message: &Native) -> Option<String> {
    // SAFETY: The response headers are owned by the live message.
    let headers = unsafe { soup_message_get_response_headers(message.pointer()) };
    // SAFETY: The live response headers accept the NUL-terminated static name.
    let location = unsafe { soup_message_headers_get_one(headers, c"Location".as_ptr()) };
    if location.is_null() {
        return None;
    }
    // SAFETY: The nonnull header value belongs to the live message and is NUL-terminated.
    unsafe { CStr::from_ptr(location) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

struct Native(NonNull<c_void>);

impl Native {
    fn owned(pointer: *mut c_void) -> Option<Self> {
        NonNull::new(pointer).map(Self)
    }

    fn pointer(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: Construction takes ownership of one live GObject reference.
        unsafe { g_object_unref(self.pointer()) };
    }
}

#[link(name = "soup-3.0")]
unsafe extern "C" {
    fn soup_session_new() -> *mut c_void;
    fn soup_session_set_timeout(session: *mut c_void, timeout: c_uint);
    fn soup_session_set_user_agent(session: *mut c_void, agent: *const c_char);
    fn soup_auth_manager_get_type() -> usize;
    fn soup_session_remove_feature_by_type(session: *mut c_void, feature: usize);
    fn soup_message_new(method: *const c_char, uri: *const c_char) -> *mut c_void;
    fn soup_message_set_flags(message: *mut c_void, flags: c_uint);
    fn soup_message_get_request_headers(message: *mut c_void) -> *mut c_void;
    fn soup_message_get_response_headers(message: *mut c_void) -> *mut c_void;
    fn soup_message_headers_replace(
        headers: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    );
    fn soup_message_headers_get_one(headers: *mut c_void, name: *const c_char) -> *const c_char;
    fn soup_message_get_status(message: *mut c_void) -> c_uint;
    fn soup_session_send(
        session: *mut c_void,
        message: *mut c_void,
        cancellable: *mut c_void,
        error: *mut *mut c_void,
    ) -> *mut c_void;
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_cancellable_new() -> *mut c_void;
    fn g_cancellable_cancel(cancellable: *mut c_void);
    fn g_cancellable_is_cancelled(cancellable: *mut c_void) -> c_int;
    fn g_input_stream_read(
        stream: *mut c_void,
        buffer: *mut c_void,
        count: usize,
        cancellable: *mut c_void,
        error: *mut *mut c_void,
    ) -> isize;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_unref(object: *mut c_void);
}

#[cfg(test)]
mod tests;

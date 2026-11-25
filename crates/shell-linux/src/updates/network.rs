//! Isolated browser requests through the system libsoup stack used by `WebKitGTK`.

#![expect(unsafe_code, reason = "libsoup and GIO are native libraries")]

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::io;
use std::ptr::{self, NonNull};

mod http;

use quickie_application::{Ended, Flow, HttpMethod};
use quickie_updater::{Response, Transport};
use url::{Host, Url};

pub(crate) use http::WebKitHttp;

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
        let headers = [("Accept".to_owned(), accept.to_owned())];
        let response = transfer(url, &HttpMethod::Get, &headers, self.0)?;
        let location = if (300..400).contains(&response.status) {
            redirect_location(&response.message)
        } else {
            None
        };
        if response.status == 200 {
            let ended = response.read_body(self.0, &mut |bytes| {
                body(bytes)
                    .map(|()| Flow::Continue)
                    .map_err(|error| error.to_string())
            })?;
            if ended == Ended::Stopped {
                return Err("the update request was cancelled".into());
            }
        }
        Ok(Response {
            status: response.status,
            location,
        })
    }
}

struct Transfer {
    stream: Native,
    message: Native,
    _session: Native,
    status: u16,
}

impl Transfer {
    fn read_body(
        &self,
        cancellation: &Cancellation,
        body: &mut dyn FnMut(&[u8]) -> Result<Flow, String>,
    ) -> Result<Ended, String> {
        let mut buffer = [0_u8; 8192];
        while !cancellation.is_cancelled() {
            // SAFETY: The live input stream writes at most buffer.len() bytes and neither it
            // nor the cancellable retains the writable buffer. GIO owns any error it discards.
            let count = unsafe {
                g_input_stream_read(
                    self.stream.pointer(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    cancellation.pointer(),
                    ptr::null_mut(),
                )
            };
            let count = usize::try_from(count).map_err(|_| "could not read the response")?;
            if count == 0 {
                return Ok(Ended::Complete);
            }
            if body(&buffer[..count])? == Flow::Stop {
                return Ok(Ended::Stopped);
            }
        }
        Ok(Ended::Stopped)
    }
}

fn transfer(
    url: &Url,
    method: &HttpMethod,
    headers: &[(String, String)],
    cancellation: &Cancellation,
) -> Result<Transfer, String> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("invalid request URL".into());
    }
    let session = session()?;
    configure_proxy(&session, url);
    let address = CString::new(url.as_str()).map_err(|_| "invalid request URL")?;
    let message = message(&address, method, headers)?;
    // SAFETY: Session and message are live on this worker. The cancellable outlives the call.
    // A null error slot suppresses errors containing the request URL.
    let stream = unsafe {
        soup_session_send(
            session.pointer(),
            message.pointer(),
            cancellation.pointer(),
            ptr::null_mut(),
        )
    };
    let stream = Native::owned(stream).ok_or("could not connect to the server")?;
    // SAFETY: Sending the live message has populated its response status.
    let status = u16::try_from(unsafe { soup_message_get_status(message.pointer()) })
        .map_err(|_| "invalid response status")?;
    Ok(Transfer {
        stream,
        message,
        _session: session,
        status,
    })
}

fn configure_proxy(session: &Native, url: &Url) {
    let loopback = match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(name)) => name == "localhost",
        None => false,
    };
    if loopback {
        // SAFETY: The live private session has no connections. NULL selects direct access.
        unsafe { soup_session_set_proxy_resolver(session.pointer(), ptr::null_mut()) };
    }
}

fn session() -> Result<Native, String> {
    // SAFETY: This creates an owned session used only for a single request on its worker.
    let session =
        Native::owned(unsafe { soup_session_new() }).ok_or("could not initialize the request")?;
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

fn message(
    url: &CStr,
    method: &HttpMethod,
    headers: &[(String, String)],
) -> Result<Native, String> {
    let name = match method {
        HttpMethod::Get => c"GET",
        HttpMethod::Post(_) => c"POST",
    };
    // SAFETY: The NUL-terminated method and URL outlive construction, which copies them.
    let message = Native::owned(unsafe { soup_message_new(name.as_ptr(), url.as_ptr()) })
        .ok_or("could not prepare the request")?;
    // SAFETY: The live message has not been queued.
    unsafe { soup_message_set_flags(message.pointer(), NO_REDIRECT) };
    // SAFETY: The headers are owned by the live message.
    let request_headers = unsafe { soup_message_get_request_headers(message.pointer()) };
    if let HttpMethod::Post(body) = method {
        // SAFETY: GBytes copies this live slice.
        let bytes = unsafe { g_bytes_new(body.as_ptr().cast(), body.len()) };
        // SAFETY: The live message retains the bytes. A null content type keeps explicit headers.
        unsafe { soup_message_set_request_body_from_bytes(message.pointer(), ptr::null(), bytes) };
        // SAFETY: We release the single reference acquired from g_bytes_new.
        unsafe { g_bytes_unref(bytes) };
    }
    for (name, value) in headers {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.contains(['\r', '\n'])
        {
            return Err("invalid request headers".into());
        }
        let name = CString::new(name.as_str()).map_err(|_| "invalid request headers")?;
        let value = CString::new(value.as_str()).map_err(|_| "invalid request headers")?;
        // SAFETY: The live headers copy the NUL-terminated name and value.
        unsafe { soup_message_headers_replace(request_headers, name.as_ptr(), value.as_ptr()) };
    }
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
    fn soup_session_set_proxy_resolver(session: *mut c_void, resolver: *mut c_void);
    fn soup_auth_manager_get_type() -> usize;
    fn soup_session_remove_feature_by_type(session: *mut c_void, feature: usize);
    fn soup_message_new(method: *const c_char, uri: *const c_char) -> *mut c_void;
    fn soup_message_set_flags(message: *mut c_void, flags: c_uint);
    fn soup_message_set_request_body_from_bytes(
        message: *mut c_void,
        content_type: *const c_char,
        bytes: *mut c_void,
    );
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

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_bytes_new(data: *const c_void, size: usize) -> *mut c_void;
    fn g_bytes_unref(bytes: *mut c_void);
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

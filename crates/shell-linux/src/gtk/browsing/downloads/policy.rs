//! Hand user-initiated downloads from `WebKit` policy to its native download object.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong};
use std::ptr;
use std::time::{Duration, Instant};

use quickie_engine_webkit::download_name::wants_download;
use url::Url;

use super::Source;
use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{App, connect};

const NAVIGATION: c_int = 0;
const RESPONSE: c_int = 2;
const GESTURE_WINDOW: Duration = Duration::from_secs(60);
const START_WINDOW: Duration = Duration::from_secs(5);
const APPLICATION_DATA: &CStr = c"quickie-downloads-app";

struct Grant {
    url: Url,
    expires: Instant,
}

#[derive(Default)]
pub(super) struct Policy {
    signal: Cell<Option<(Object, c_ulong)>>,
    navigations: RefCell<HashMap<Object, Grant>>,
    decisions: RefCell<HashMap<Object, Grant>>,
}

impl Policy {
    pub(super) fn attach(&self, application: Object, data: Object) {
        // SAFETY: The GTK application and App state stay live until detach clears this slot.
        unsafe { ffi::g_object_set_data(application, APPLICATION_DATA.as_ptr(), data) };
        // SAFETY: The default session is available on the GTK thread after initialization.
        let session = unsafe { webkit_network_session_get_default() };
        // SAFETY: This callback uses the WebKitNetworkSession::download-started ABI. Detach
        // disconnects it before the App reference goes out of scope.
        let id = unsafe {
            ffi::g_signal_connect_data(
                session,
                c"download-started".as_ptr(),
                started as *const _,
                data,
                None,
                0,
            )
        };
        self.signal.set(Some((session, id)));
    }

    pub(super) fn detach(&self, application: Object) {
        // SAFETY: GTK retains the application until after this detach, and the cleared slot
        // prevents any later network session signal from reaching the Rust App state.
        unsafe { ffi::g_object_set_data(application, APPLICATION_DATA.as_ptr(), ptr::null_mut()) };
        if let Some((session, id)) = self.signal.take() {
            // SAFETY: The default session is live and this handler is still connected.
            unsafe { g_signal_handler_disconnect(session, id) };
        }
    }

    pub(super) fn forget(&self, view: Object) {
        self.navigations.borrow_mut().remove(&view);
        self.decisions.borrow_mut().remove(&view);
    }

    fn navigation(&self, view: Object, decision: Object) {
        // SAFETY: Navigation policy decisions supply a live navigation action.
        let action = unsafe { webkit_navigation_policy_decision_get_navigation_action(decision) };
        // SAFETY: The action owns this request throughout the policy signal.
        let request = unsafe { webkit_navigation_action_get_request(action) };
        let url = request_url(request);
        // SAFETY: The policy signal keeps the navigation action alive.
        let gesture = unsafe { webkit_navigation_action_is_user_gesture(action) } != ffi::FALSE;
        // SAFETY: The same action indicates whether this navigation is a server redirect.
        let redirect = unsafe { webkit_navigation_action_is_redirect(action) } != ffi::FALSE;
        let mut navigations = self.navigations.borrow_mut();
        let previous = navigations.remove(&view);
        let expires = if gesture {
            Some(Instant::now() + GESTURE_WINDOW)
        } else {
            previous.filter(|_| redirect).map(|grant| grant.expires)
        };
        if let Some((url, expires)) = url.zip(expires) {
            navigations.insert(view, Grant { url, expires });
        }
    }

    fn response(&self, view: Object, decision: Object) -> bool {
        // SAFETY: This subtype is supplied for response decisions only.
        let main = unsafe { webkit_response_policy_decision_is_main_frame_main_resource(decision) };
        // SAFETY: The live decision owns its response.
        let response = unsafe { webkit_response_policy_decision_get_response(decision) };
        let disposition = response_header(response, c"Content-Disposition");
        // SAFETY: These queries read the live decision and its borrowed response.
        let can_show = unsafe { webkit_response_policy_decision_is_mime_type_supported(decision) };
        // SAFETY: The response belongs to the live policy decision.
        let status = unsafe { webkit_uri_response_get_status_code(response) };
        // SAFETY: The request belongs to the live policy decision.
        let request = unsafe { webkit_response_policy_decision_get_request(decision) };
        let url = request_url(request);
        let grant = if main == ffi::FALSE {
            None
        } else {
            self.navigations.borrow_mut().remove(&view)
        };
        if !wants_download(can_show != ffi::FALSE, disposition.as_deref()) || status == 204 {
            return false;
        }
        if (200..300).contains(&status)
            && let Some((url, grant)) = url.zip(grant)
            && grant.url == url
            && Instant::now() <= grant.expires
        {
            self.decisions.borrow_mut().insert(
                view,
                Grant {
                    url,
                    expires: Instant::now() + START_WINDOW,
                },
            );
            // SAFETY: The policy signal keeps this decision alive. The approved grant is already
            // set in case WebKit delivers download-started before returning.
            unsafe { webkit_policy_decision_download(decision) };
        } else {
            // SAFETY: The response decision is live. Stop WebKit's default attachment handling.
            unsafe { webkit_policy_decision_ignore(decision) };
        }
        true
    }

    fn claim(&self, view: Object, url: &Url) -> bool {
        let now = Instant::now();
        [&self.decisions, &self.navigations]
            .into_iter()
            .any(|grants| {
                let mut grants = grants.borrow_mut();
                if grants
                    .get(&view)
                    .is_some_and(|grant| grant.url == *url && now <= grant.expires)
                {
                    grants.remove(&view);
                    true
                } else {
                    false
                }
            })
    }
}

pub(in crate::gtk::browsing) fn observe(view: Object, data: Object) {
    // SAFETY: Every observed view was constructed with a live network session.
    let session = unsafe { webkit_web_view_get_network_session(view) };
    // SAFETY: The engine owns the view and supplies the live App on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let enabled = app.browsing.downloads.policy.signal.get().is_some();
    if enabled {
        connect(view, c"decide-policy", decide as *const _, data);
    }
    // SAFETY: The default session is live throughout the GTK application.
    if session != unsafe { webkit_network_session_get_default() } {
        let key = c"quickie-observes-downloads";
        // SAFETY: The ephemeral session is live on the GTK thread.
        if unsafe { ffi::g_object_get_data(session, key.as_ptr()) }.is_null() {
            if enabled {
                // SAFETY: The connection does not retain the application; GObject disconnects
                // it when the application disappears, and the session can end independently.
                unsafe {
                    ffi::g_signal_connect_object(
                        session,
                        c"download-started".as_ptr(),
                        started_in_jar as *const _,
                        app.application,
                        0,
                    )
                };
            } else {
                connect(
                    session,
                    c"download-started",
                    decline as *const _,
                    ptr::null_mut(),
                );
            }
            // SAFETY: The marker is only checked for NULL and is never dereferenced or freed.
            unsafe { ffi::g_object_set_data(session, key.as_ptr(), ptr::dangling_mut()) };
        }
    }
}

extern "C" fn decide(view: Object, decision: Object, kind: c_int, data: Object) -> Bool {
    // SAFETY: The App registered on this view outlives its policy signals.
    let app = unsafe { &*data.cast::<App>() };
    let policy = &app.browsing.downloads.policy;
    match kind {
        NAVIGATION => policy.navigation(view, decision),
        RESPONSE => return Bool::from(policy.response(view, decision)),
        _ => {}
    }
    ffi::FALSE
}

extern "C" fn started(_: Object, download: Object, data: Object) {
    // SAFETY: The default session's handler is disconnected before the GTK App is dropped.
    let app = unsafe { &*data.cast::<App>() };
    let downloads = &app.browsing.downloads;
    if downloads.transfers.contains(download) {
        return;
    }
    // SAFETY: WebKit keeps the download and its initiating view live during this signal.
    let view = unsafe { webkit_download_get_web_view(download) };
    // SAFETY: WebKit owns the returned request for the duration of the signal.
    let request = unsafe { webkit_download_get_request(download) };
    if let Some(url) = request_url(request)
        && let Some(source) = Source::from_view(app, view)
        && downloads.policy.claim(view, &url)
        && downloads.transfers.begin(download, source, url, data)
    {
        return;
    }
    decline(ptr::null_mut(), download, ptr::null_mut());
}

extern "C" fn started_in_jar(session: Object, download: Object, application: Object) {
    // SAFETY: The GtkApplication is the receiver of this signal and remains live here.
    let data = unsafe { ffi::g_object_get_data(application, APPLICATION_DATA.as_ptr()) };
    if data.is_null() {
        decline(session, download, data);
    } else {
        started(session, download, data);
    }
}

extern "C" fn decline(_: Object, download: Object, _: Object) {
    // SAFETY: WebKit's live download has not yet created a destination.
    unsafe { webkit_download_cancel(download) };
}

fn request_url(request: Object) -> Option<Url> {
    if request.is_null() {
        return None;
    }
    // SAFETY: The live request owns the borrowed method and URI strings.
    let method = unsafe { webkit_uri_request_get_http_method(request) };
    if copy_text(method).as_deref() != Some("GET") {
        return None;
    }
    // SAFETY: The live request owns its URI until the signal returns.
    let uri = unsafe { webkit_uri_request_get_uri(request) };
    let url = Url::parse(&copy_text(uri)?).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

pub(super) fn copy_text(text: *const c_char) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: Only called for NUL-terminated strings borrowed from a live GObject or GLib.
    Some(
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned(),
    )
}

pub(super) fn response_header(response: Object, name: &CStr) -> Option<String> {
    if response.is_null() {
        return None;
    }
    // SAFETY: The live response owns its HTTP headers, or returns NULL for other resources.
    let headers = unsafe { webkit_uri_response_get_http_headers(response) };
    if headers.is_null() {
        return None;
    }
    // SAFETY: Libsoup owns the header string for the lifetime of the borrowed header list.
    copy_text(unsafe { soup_message_headers_get_one(headers, name.as_ptr()) })
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_signal_handler_disconnect(instance: Object, id: c_ulong);
}

#[link(name = "soup-3.0")]
unsafe extern "C" {
    fn soup_message_headers_get_one(headers: Object, name: *const c_char) -> *const c_char;
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_network_session_get_default() -> Object;
    fn webkit_web_view_get_network_session(view: Object) -> Object;
    fn webkit_navigation_policy_decision_get_navigation_action(decision: Object) -> Object;
    fn webkit_navigation_action_get_request(action: Object) -> Object;
    fn webkit_navigation_action_is_user_gesture(action: Object) -> Bool;
    fn webkit_navigation_action_is_redirect(action: Object) -> Bool;
    fn webkit_response_policy_decision_get_request(decision: Object) -> Object;
    fn webkit_response_policy_decision_get_response(decision: Object) -> Object;
    fn webkit_response_policy_decision_is_main_frame_main_resource(decision: Object) -> Bool;
    fn webkit_response_policy_decision_is_mime_type_supported(decision: Object) -> Bool;
    fn webkit_uri_request_get_http_method(request: Object) -> *const c_char;
    fn webkit_uri_request_get_uri(request: Object) -> *const c_char;
    fn webkit_uri_response_get_http_headers(response: Object) -> Object;
    fn webkit_uri_response_get_status_code(response: Object) -> c_uint;
    fn webkit_policy_decision_download(decision: Object);
    fn webkit_policy_decision_ignore(decision: Object);
    fn webkit_download_get_web_view(download: Object) -> Object;
    fn webkit_download_get_request(download: Object) -> Object;
    fn webkit_download_cancel(download: Object);
}

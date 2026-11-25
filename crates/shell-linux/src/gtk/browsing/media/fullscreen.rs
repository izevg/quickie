//! The page's real fullscreen element fills the browser's existing native window.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_int, c_uint};
use std::ptr;

use super::script;
use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{App, browsing, connect, find, palette, privacy};
use url::Url;

const ALIGN_START: c_int = 1;
const NOTICE_SECONDS: c_uint = 3;

#[derive(Default)]
pub(super) struct Fullscreen {
    notice: Cell<Object>,
    timer: Cell<c_uint>,
    page: Cell<Object>,
}

impl Fullscreen {
    pub(super) fn is_active(&self) -> bool {
        !self.page.get().is_null()
    }

    pub(super) fn attach(&self, overlay: Object) {
        // SAFETY: GTK is initialized on this thread and accepts a null initial label.
        let label = unsafe { ffi::gtk_label_new(ptr::null()) };
        // SAFETY: The live label accepts this alignment.
        unsafe { gtk_widget_set_halign(label, ffi::ALIGN_CENTER) };
        // SAFETY: The live label accepts this alignment.
        unsafe { ffi::gtk_widget_set_valign(label, ALIGN_START) };
        // SAFETY: The live label accepts this positive margin.
        unsafe { ffi::gtk_widget_set_margin_top(label, 22) };
        // SAFETY: GTK copies the static class name onto the live label.
        unsafe { ffi::gtk_widget_add_css_class(label, c"quickie-fullscreen-notice".as_ptr()) };
        // SAFETY: The live notice leaves input with the page underneath.
        unsafe { gtk_widget_set_can_target(label, ffi::FALSE) };
        // SAFETY: The notice starts hidden while the page is windowed.
        unsafe { ffi::gtk_widget_set_visible(label, ffi::FALSE) };
        // SAFETY: Fullscreen adopts the floating reference and releases it at shutdown.
        unsafe { g_object_ref_sink(label) };
        // SAFETY: The live overlay retains the detached label.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, label) };
        self.notice.set(label);
    }

    pub(super) fn active_view_changed(&self, previous: Object) {
        if self.page.get() == previous && !previous.is_null() {
            self.clear();
            script::exit_fullscreen(previous);
        }
    }

    pub(super) fn shutdown(&self) {
        self.stop_timer();
        let label = self.notice.replace(ptr::null_mut());
        if !label.is_null() {
            // SAFETY: This releases our retained reference after the GTK loop has ended.
            unsafe { ffi::g_object_unref(label) };
        }
    }

    fn clear(&self) {
        self.page.set(ptr::null_mut());
        self.stop_timer();
        self.hide_notice();
    }

    fn stop_timer(&self) {
        let timer = self.timer.replace(0);
        if timer != 0 {
            // SAFETY: This registered source has not yet run.
            unsafe { ffi::g_source_remove(timer) };
        }
    }

    fn hide_notice(&self) {
        let label = self.notice.get();
        if !label.is_null() {
            // SAFETY: GTK and Fullscreen own the live label on the main thread.
            unsafe { ffi::gtk_widget_set_visible(label, ffi::FALSE) };
        }
    }

    fn show(&self, view: Object, data: Object) {
        self.page.set(view);
        let label = self.notice.get();
        if label.is_null() {
            return;
        }
        let origin = origin(view);
        let text = CString::new(format!("{origin} is fullscreen · Press Esc to exit"));
        if let Ok(text) = text {
            // SAFETY: The live label copies plain text.
            unsafe { ffi::gtk_label_set_text(label, text.as_ptr()) };
            // SAFETY: The live label is attached to the overlay.
            unsafe { ffi::gtk_widget_set_visible(label, ffi::TRUE) };
        }
        self.stop_timer();
        // SAFETY: App outlives the registered source, removed before shutdown.
        self.timer
            .set(unsafe { ffi::g_timeout_add_seconds(NOTICE_SECONDS, expire, data) });
    }
}

pub(super) fn bind(view: Object, data: Object) {
    connect(view, c"enter-fullscreen", enter as *const _, data);
    connect(view, c"leave-fullscreen", leave as *const _, data);
}

extern "C" fn enter(view: Object, data: Object) -> Bool {
    // SAFETY: The view receives the live App reference on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get().filter(|widgets| widgets.view == view) {
        browsing::hide_page_overlays(app);
        browsing::dismiss_panels(app);
        palette::hide(app, &widgets, false);
        find::hide(app, &widgets, false);
        privacy::close(app, &widgets, false);
        app.browsing.settings.set_page_fullscreen(true);
        // SAFETY: The requesting active view is a live widget in the browser window.
        unsafe { ffi::gtk_widget_grab_focus(view) };
        app.browsing.media.fullscreen.show(view, data);
    } else {
        script::exit_fullscreen(view);
    }
    // WebKit has entered the Fullscreen API transition before emitting this signal.
    // Handling it skips the default native toplevel resize and keeps the DOM transition.
    ffi::TRUE
}

extern "C" fn leave(view: Object, data: Object) -> Bool {
    // SAFETY: The view receives the live App reference on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let fullscreen = &app.browsing.media.fullscreen;
    if fullscreen.page.get() == view {
        fullscreen.clear();
        app.browsing.settings.set_page_fullscreen(false);
    }
    // The default handler completes WebKit's private fullscreen state transition.
    ffi::FALSE
}

extern "C" fn expire(data: Object) -> Bool {
    // SAFETY: The timer was attached with the live App reference on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let fullscreen = &app.browsing.media.fullscreen;
    fullscreen.timer.set(0);
    fullscreen.hide_notice();
    ffi::FALSE
}

fn origin(view: Object) -> String {
    // SAFETY: WebKit owns the borrowed nullable URI while the view is live.
    let uri = unsafe { ffi::webkit_web_view_get_uri(view) };
    if !uri.is_null() {
        // SAFETY: The non-null URI is NUL-terminated and read synchronously.
        if let Ok(url) = Url::parse(&unsafe { CStr::from_ptr(uri) }.to_string_lossy()) {
            let origin = url.origin().ascii_serialization();
            if origin != "null" {
                return origin;
            }
        }
    }
    "This page".into()
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_set_halign(widget: Object, align: c_int);
    fn gtk_widget_set_can_target(widget: Object, can_target: Bool);
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref_sink(object: Object) -> Object;
}

//! General settings and the window's native page frame.

mod appearance;
mod desktop;
mod view;

use std::cell::{Cell, RefCell};
use std::ffi::c_uint;
use std::path::Path;
use std::ptr;

use quickie_domain_settings::PageLayout;

use super::data_home;
use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{App, connect};
use appearance::Writer;
use view::View;

const PAGE_INSET: i32 = 8;
const SAVE_POLL_MILLIS: c_uint = 30;

#[derive(Default)]
pub(in crate::gtk) struct Settings {
    parent: Cell<Object>,
    frame: Cell<Object>,
    window: Cell<Option<View>>,
    appearance: RefCell<Option<Writer>>,
    layout: Cell<PageLayout>,
    save_poll: Cell<c_uint>,
    page_fullscreen: Cell<bool>,
    synchronizing: Cell<bool>,
}

impl Settings {
    pub(in crate::gtk) fn attach(
        &self,
        parent: Object,
        overlay: Object,
        data: Object,
        isolated: bool,
    ) {
        self.parent.set(parent);
        view::install_style(parent);
        // SAFETY: GTK is initialized. The browser overlay will own this frame widget.
        let frame = unsafe { ffi::gtk_overlay_new() };
        self.frame.set(frame);
        // SAFETY: The live browser overlay adopts the newly created frame.
        unsafe { ffi::gtk_overlay_set_child(overlay, frame) };
        connect(
            parent,
            c"notify::fullscreened",
            fullscreen_changed as *const _,
            data,
        );
        connect(parent, c"close-request", parent_closing as *const _, data);

        match Writer::start(isolated) {
            Ok((writer, layout)) => {
                self.layout.set(layout);
                self.appearance.replace(Some(writer));
            }
            Err(error) => super::report(&error),
        }
        self.update_frame();
    }

    pub(in crate::gtk) fn mount(&self, page: Object) {
        // SAFETY: Both widgets are live. The engine retains the existing and incoming pages,
        // and the frame parents only the active page.
        unsafe { ffi::gtk_overlay_set_child(self.frame.get(), page) };
    }

    pub(in crate::gtk) fn set_page_fullscreen(&self, fullscreen: bool) {
        self.page_fullscreen.set(fullscreen);
        self.update_frame();
    }

    pub(in crate::gtk) fn detach(&self) {
        self.stop_polling();
        drop(self.appearance.take());
    }

    fn stop_polling(&self) {
        let poll = self.save_poll.replace(0);
        if poll != 0 {
            // SAFETY: This source is registered with the live App and can be removed once.
            unsafe { ffi::g_source_remove(poll) };
        }
    }

    fn update_frame(&self) {
        let parent = self.parent.get();
        if parent.is_null() {
            return;
        }
        // SAFETY: The attached parent remains live throughout these GTK callbacks.
        let window_fullscreen = unsafe { gtk_window_is_fullscreen(parent) } != ffi::FALSE;
        let inset = self.layout.get() == PageLayout::Inset
            && !self.page_fullscreen.get()
            && !window_fullscreen;
        let frame = self.frame.get();
        let margin = if inset { PAGE_INSET } else { 0 };
        view::margins(frame, margin, margin, margin, margin);
        view::set_framed(frame, inset);
    }

    fn sync_appearance(&self, view: View) {
        self.synchronizing.set(true);
        view.set_inset(
            self.layout.get() == PageLayout::Inset,
            self.appearance.borrow().is_some(),
            self.save_poll.get() != 0,
        );
        self.synchronizing.set(false);
    }

    fn show_save_error(&self, view: View, error: &str) {
        super::report(&error);
        self.sync_appearance(view);
        view.appearance_error("Quickie could not save the appearance setting.");
    }
}

pub(in crate::gtk) fn show(app: &App, data: Object) {
    let settings = &app.browsing.settings;
    let parent = settings.parent.get();
    if parent.is_null() {
        return;
    }
    let view = settings.window.get().unwrap_or_else(|| {
        let view = View::new(parent, data);
        settings.window.set(Some(view));
        view
    });
    view.set_browser(&desktop::browser_status());
    view.set_downloads(app.browsing.downloads.folder());
    settings.sync_appearance(view);
    view.present();
}

extern "C" fn fullscreen_changed(_: Object, _: Object, data: Object) {
    // SAFETY: This signal belongs to the live main-thread browser window.
    let app = unsafe { &*data.cast::<App>() };
    app.browsing.settings.update_frame();
}

extern "C" fn parent_closing(_: Object, data: Object) -> Bool {
    // SAFETY: This signal belongs to the live browser window, just before destruction.
    let app = unsafe { &*data.cast::<App>() };
    let settings = &app.browsing.settings;
    settings.stop_polling();
    settings.window.set(None);
    settings.parent.set(ptr::null_mut());
    ffi::FALSE
}

extern "C" fn make_default_clicked(_: Object, data: Object) {
    // SAFETY: This signal belongs to a button of the live main-thread settings window.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(view) = app.browsing.settings.window.get() {
        let result = desktop::make_default();
        let status = desktop::browser_status();
        view.set_browser(&status);
        if let Err(error) = result {
            view.browser_error(&format!("Could not make Quickie the default. {error}"));
        } else if status.can_make_default {
            view.browser_error(
                "The desktop did not make Quickie the default for both web link types.",
            );
        }
    }
}

extern "C" fn downloads_clicked(_: Object, data: Object) {
    // SAFETY: This signal belongs to a button of the live main-thread settings window.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(view) = app.browsing.settings.window.get() {
        open_downloads(view, app.browsing.downloads.folder());
    }
}

fn open_downloads(view: View, path: Option<&Path>) {
    if let Some(path) = path {
        view.clear_downloads_error();
        if let Err(error) = desktop::open_downloads(path, view.window(), view.downloads_status()) {
            view.downloads_error(&error);
        }
    }
}

extern "C" fn inset_changed(toggle: Object, _: Object, data: Object) {
    // SAFETY: This signal belongs to the switch of the live main-thread settings window.
    let app = unsafe { &*data.cast::<App>() };
    let settings = &app.browsing.settings;
    let Some(view) = settings.window.get() else {
        return;
    };
    if settings.synchronizing.get() || settings.save_poll.get() != 0 {
        return;
    }
    // SAFETY: GTK passes the live switch whose active state changed.
    let inset = unsafe { ffi::gtk_switch_get_active(toggle) } != ffi::FALSE;
    let layout = if inset {
        PageLayout::Inset
    } else {
        PageLayout::EdgeToEdge
    };
    let result = settings
        .appearance
        .borrow()
        .as_ref()
        .map(|appearance| appearance.save(layout));
    match result {
        Some(Ok(())) => {
            view.saving();
            // SAFETY: The source receives the live App and is removed before App is dropped.
            let source = unsafe { ffi::g_timeout_add(SAVE_POLL_MILLIS, appearance_saved, data) };
            settings.save_poll.set(source);
        }
        Some(Err(error)) => settings.show_save_error(view, &error),
        None => settings.sync_appearance(view),
    }
}

extern "C" fn appearance_saved(data: Object) -> Bool {
    // SAFETY: The main-thread source passes the App, which outlives the callback.
    let app = unsafe { &*data.cast::<App>() };
    let settings = &app.browsing.settings;
    let outcome = settings
        .appearance
        .borrow()
        .as_ref()
        .and_then(Writer::outcome);
    let Some(outcome) = outcome else {
        return ffi::TRUE;
    };
    settings.save_poll.set(0);
    if let Some(view) = settings.window.get() {
        match outcome {
            Ok(layout) => {
                settings.layout.set(layout);
                settings.sync_appearance(view);
                settings.update_frame();
            }
            Err(error) => settings.show_save_error(view, &error),
        }
    }
    ffi::FALSE
}

extern "C" fn updates_clicked(_: Object, data: Object) {
    // SAFETY: This signal belongs to a button of the live main-thread settings window.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(view) = app.browsing.settings.window.get() {
        // SAFETY: The live settings window hides on a close request.
        unsafe { ffi::gtk_window_close(view.window()) };
        app.updates.present(data, &app.prefix);
    }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_window_is_fullscreen(window: Object) -> Bool;
}

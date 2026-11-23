//! Connects the software updates window to GTK's main loop.

use std::cell::{Cell, RefCell};
use std::ffi::c_uint;
use std::path::Path;
use std::ptr;

use crate::updates::Updates;
use crate::updates::dialog::{Actions, Dialog};

use super::ffi::{self, Bool, Object};
use super::{App, connect};

pub struct UpdateControls {
    model: RefCell<Updates>,
    dialog: RefCell<Option<Dialog>>,
    parent: Cell<Object>,
    poll_source: Cell<c_uint>,
    automatic_source: Cell<c_uint>,
    rendering: Cell<bool>,
}

impl UpdateControls {
    pub fn new() -> Self {
        Self {
            model: RefCell::new(Updates::new()),
            dialog: RefCell::new(None),
            parent: Cell::new(ptr::null_mut()),
            poll_source: Cell::new(0),
            automatic_source: Cell::new(0),
            rendering: Cell::new(false),
        }
    }

    pub fn attach(&self, parent: Object, data: Object, manual: bool) {
        self.parent.set(parent);
        connect(parent, c"close-request", browser_closing as *const _, data);
        if !manual {
            self.model.borrow_mut().check(true);
            self.watch(data);
        }
        if self.model.borrow().version().is_some() {
            // SAFETY: The close handler and run remove this source before the App is dropped.
            self.automatic_source
                .set(unsafe { ffi::g_timeout_add_seconds(60 * 60, check_automatic, data) });
        }
    }

    pub fn present(&self, data: Object, prefix: &Path) {
        let parent = self.parent.get();
        if parent.is_null() {
            return;
        }
        let version = option_env!("QUICKIE_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
        self.dialog.borrow_mut().get_or_insert_with(|| {
            Dialog::new(
                parent,
                prefix,
                version,
                data,
                Actions {
                    check: check_clicked,
                    toggle: toggle_automatic,
                    cancel: cancel_clicked,
                    action: action_clicked,
                },
            )
        });
        self.render();
        if let Some(dialog) = self.dialog.borrow().as_ref() {
            dialog.present();
        }
    }

    pub fn check(&self, data: Object) {
        self.model.borrow_mut().check(false);
        self.render();
        self.watch(data);
    }

    pub fn clear_sources(&self) {
        for source in [&self.poll_source, &self.automatic_source] {
            let id = source.replace(0);
            if id != 0 {
                // SAFETY: Each registered source ID is removed once on the GTK main thread.
                unsafe { ffi::g_source_remove(id) };
            }
        }
    }

    pub fn finish(self) -> Result<(), String> {
        self.model.into_inner().finish()
    }

    fn watch(&self, data: Object) {
        if self.model.borrow().is_busy() && self.poll_source.get() == 0 {
            // SAFETY: The close handler and run remove this source before the App is dropped.
            self.poll_source
                .set(unsafe { ffi::g_timeout_add(100, poll_updates, data) });
        }
    }

    fn render(&self) {
        if let Some(dialog) = self.dialog.borrow().as_ref() {
            self.rendering.set(true);
            dialog.render(&self.model.borrow());
            self.rendering.set(false);
        }
    }
}

extern "C" fn browser_closing(_: Object, data: Object) -> Bool {
    // SAFETY: The browser's close signal is synchronous and its App outlives the event loop.
    let app = unsafe { &*data.cast::<App>() };
    app.updates.clear_sources();
    app.updates.parent.set(ptr::null_mut());
    app.updates.dialog.take();
    ffi::FALSE
}

extern "C" fn check_automatic(data: Object) -> Bool {
    // SAFETY: This source is installed and removed within the App lifetime on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.updates.model.borrow_mut().check(true);
    app.updates.watch(data);
    ffi::TRUE
}

extern "C" fn poll_updates(data: Object) -> Bool {
    // SAFETY: This source is installed and removed within the App lifetime on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.updates.model.borrow_mut().poll();
    app.updates.render();
    if app.updates.model.borrow().is_busy() {
        ffi::TRUE
    } else {
        app.updates.poll_source.set(0);
        ffi::FALSE
    }
}

extern "C" fn check_clicked(_: Object, data: Object) {
    // SAFETY: This signal is emitted by the live dialog on its GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.updates.check(data);
}

extern "C" fn toggle_automatic(toggle: Object, _: Object, data: Object) {
    // SAFETY: This signal is emitted by the live dialog on its GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if app.updates.rendering.get() {
        return;
    }
    // SAFETY: The emitter of this signal is the dialog's live GtkSwitch.
    let enabled = unsafe { ffi::gtk_switch_get_active(toggle) == ffi::TRUE };
    app.updates.model.borrow_mut().set_automatic(enabled);
    app.updates.render();
    app.updates.watch(data);
}

extern "C" fn cancel_clicked(_: Object, data: Object) {
    // SAFETY: This signal is emitted by the live dialog on its GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.updates.model.borrow_mut().cancel();
    app.updates.render();
}

extern "C" fn action_clicked(_: Object, data: Object) {
    // SAFETY: This signal is emitted by the live dialog on its GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if app.updates.model.borrow_mut().request_restart() {
        let window = app.updates.parent.get();
        if !window.is_null() {
            // SAFETY: A live parent is retained until the browser's close-request handler.
            unsafe { ffi::gtk_window_close(window) };
        }
    } else {
        app.updates.model.borrow_mut().prepare();
        app.updates.render();
        app.updates.watch(data);
    }
}

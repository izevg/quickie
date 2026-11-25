//! The passive origin disclosure for the active GTK tab.

mod view;

use std::cell::{Cell, RefCell};
use std::ffi::c_uint;

use quickie_domain_browsing::{Origin, TabId, is_cross_origin};
use url::Url;

use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{App, connect};

const HOLD_MILLIS: c_uint = 2_400;

#[derive(Default)]
pub(super) struct Chip {
    view: Cell<Option<view::View>>,
    seen: RefCell<Option<(TabId, Url)>>,
    timer: Cell<c_uint>,
    data: Cell<Object>,
}

impl Chip {
    pub(super) fn attach(&self, overlay: Object, data: Object) {
        let chip = view::View::new(overlay);
        connect(
            chip.revealer(),
            c"notify::child-revealed",
            faded as *const _,
            data,
        );
        self.view.set(Some(chip));
        self.data.set(data);
    }

    pub(super) fn observe(&self, web_view: Object, tab: TabId, url: &Url) {
        let mut seen = self.seen.borrow_mut();
        let changed = seen
            .as_ref()
            .is_none_or(|(previous, before)| *previous != tab || is_cross_origin(before, url));
        if !changed {
            return;
        }
        *seen = Some((tab, url.clone()));
        drop(seen);
        let (Some(chip), Some(origin)) = (self.view.get(), Origin::of(url)) else {
            self.hide();
            return;
        };
        self.stop_timer();
        chip.show(web_view, &origin, url.path());
        // SAFETY: The application owns this source and removes it before teardown.
        self.timer
            .set(unsafe { ffi::g_timeout_add(HOLD_MILLIS, expire, self.data.get()) });
    }

    pub(in crate::gtk) fn hide(&self) {
        self.stop_timer();
        if let Some(chip) = self.view.get() {
            chip.hide();
        }
    }

    pub(super) fn forget(&self) {
        self.seen.replace(None);
        self.hide();
    }

    pub(super) fn shutdown(&self) {
        self.stop_timer();
        if let Some(chip) = self.view.take() {
            chip.release();
        }
    }

    fn stop_timer(&self) {
        let timer = self.timer.replace(0);
        if timer != 0 {
            // SAFETY: A nonzero ID belongs to this chip and has not fired yet.
            unsafe { ffi::g_source_remove(timer) };
        }
    }
}

extern "C" fn expire(data: Object) -> Bool {
    // SAFETY: This source was installed for the live application on GTK's main thread.
    let app = unsafe { &*data.cast::<App>() };
    let chip = &app.browsing.origin;
    chip.timer.set(0);
    if let Some(view) = chip.view.get() {
        view.fade();
    }
    ffi::FALSE
}

extern "C" fn faded(_: Object, _: Object, data: Object) {
    // SAFETY: The GTK signal was connected with the live application as its data.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(chip) = app.browsing.origin.view.get() {
        chip.clear_faded_backdrop();
    }
}

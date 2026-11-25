//! Per-site zoom in GTK, through the same use case as `AppKit`.

use std::cell::RefCell;
use std::ffi::c_double;

use quickie_application::{AppError, PageZoom, Zoom};
use quickie_domain_browsing::{Jar, JarId, Session, TabId};
use quickie_domain_settings::{ZoomLevel, ZoomStep};
use quickie_store::{PendingWrites, WriteBehindStore};
use url::Url;

use super::{Pages, report};
use crate::gtk::ffi::Object;

#[derive(Default)]
pub(super) struct SiteZoom {
    service: RefCell<Option<Zoom<WriteBehindStore>>>,
    writes: RefCell<Option<PendingWrites>>,
}

impl SiteZoom {
    pub(super) fn restore(&self, store: WriteBehindStore) -> Result<(), AppError> {
        let writes = store.pending();
        let service = Zoom::start(store)?;
        if let Some(error) = service.corrupt_on_start() {
            report(error);
        }
        self.service.replace(Some(service));
        self.writes.replace(Some(writes));
        Ok(())
    }

    pub(super) fn step(
        &self,
        url: &Url,
        jar: Jar,
        step: ZoomStep,
    ) -> Result<Option<ZoomLevel>, AppError> {
        self.service
            .borrow_mut()
            .as_mut()
            .map(|service| service.step(url, jar, step))
            .transpose()
    }

    pub(super) fn apply(&self, session: &Session, pages: &Pages) {
        if let Some(service) = self.service.borrow().as_ref() {
            service.show(session, None, &mut GtkPages(pages));
        }
    }

    pub(super) fn apply_view(&self, view: Object, url: &Url, jar: Jar) {
        if let Some(service) = self.service.borrow().as_ref() {
            set_zoom(view, service.level(url, jar));
        }
    }

    pub(super) fn forget(&self, jar: JarId) {
        if let Some(service) = self.service.borrow_mut().as_mut() {
            service.forget_jar(jar);
        }
    }

    pub(super) fn flush(&self) {
        if let Some(writes) = self.writes.take()
            && let Err(error) = writes.flush()
        {
            report(&error);
        }
    }
}

struct GtkPages<'a>(&'a Pages);

impl PageZoom for GtkPages<'_> {
    fn set_zoom(&mut self, tab: TabId, level: ZoomLevel) {
        if let Some(view) = self.0.view(tab) {
            set_zoom(view, level);
        }
    }
}

fn set_zoom(view: Object, level: ZoomLevel) {
    // SAFETY: Both callers supply a live WebKitGTK view on its owning GTK thread.
    unsafe { webkit_web_view_set_zoom_level(view, level.factor()) };
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_set_zoom_level(view: Object, level: c_double);
}

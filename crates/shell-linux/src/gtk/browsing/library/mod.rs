//! The browsing profile's durable visits and explicitly saved bookmarks.

#[cfg(test)]
mod tests;

use std::cell::RefCell;

use quickie_application::{AppError, Library};
use quickie_domain_browsing::{Jar, Profile, Tab, TabId};
use quickie_domain_library::{Downloads, Recorded, Saved};
use quickie_store::{PendingWrites, WriteBehindStore};
use url::Url;

use super::{SystemClock, report};

type Service = Library<WriteBehindStore, SystemClock>;

#[derive(Default)]
pub(super) struct ProfileLibrary {
    service: RefCell<Option<Service>>,
    writes: RefCell<Option<PendingWrites>>,
}

impl ProfileLibrary {
    pub(super) fn restore(&self, store: WriteBehindStore) -> Result<Downloads, AppError> {
        let writes = store.pending();
        let library = Library::start(store, SystemClock)?;
        for error in library.corrupt_on_start() {
            report(error);
        }
        let downloads = library.downloads().clone();
        self.service.replace(Some(library));
        self.writes.replace(Some(writes));
        Ok(downloads)
    }

    pub(super) fn with<T>(&self, read: impl FnOnce(&Service) -> T) -> Option<T> {
        self.service.borrow().as_ref().map(read)
    }

    pub(super) fn visit(
        &self,
        tab: TabId,
        jar: Jar,
        url: &Url,
        title: &str,
    ) -> Result<bool, AppError> {
        if tab.profile() != Profile::Main {
            return Ok(false);
        }
        self.change(|library| library.record_visit(url, title, jar))
            .map(|recorded| recorded.is_some_and(|visit| visit != Recorded::Ignored))
    }

    pub(super) fn retitle(
        &self,
        tab: TabId,
        jar: Jar,
        url: &Url,
        title: &str,
    ) -> Result<bool, AppError> {
        if tab.profile() != Profile::Main {
            return Ok(false);
        }
        self.change(|library| library.retitle_visit(url, title, jar))
            .map(Option::unwrap_or_default)
    }

    pub(super) fn bookmark(&self, tab: &Tab) -> Result<Option<Saved>, AppError> {
        if tab.id.profile() != Profile::Main {
            return Ok(None);
        }
        self.change(|library| library.save_bookmark(&tab.url, tab.title.clone()))
    }

    pub(super) fn flush(&self) {
        if let Some(writes) = self.writes.take()
            && let Err(error) = writes.flush()
        {
            report(&error);
        }
    }

    fn change<T>(
        &self,
        update: impl FnOnce(&mut Service) -> Result<T, AppError>,
    ) -> Result<Option<T>, AppError> {
        self.service.borrow_mut().as_mut().map(update).transpose()
    }
}

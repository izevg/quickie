//! Downloads initiated in browsing tabs. Throwaway records stay in memory.

mod policy;
mod transfer;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::ffi::c_uint;
use std::path::{Path, PathBuf};
use std::ptr;

use quickie_application::Clock;
use quickie_domain_browsing::{Jar, TabId};
use quickie_domain_library::{
    Download, DownloadId, DownloadState, Downloads as List, DownloadsSnapshot,
};
use quickie_store::PendingWrites;
use url::Url;

use super::SystemClock;
use crate::gtk::ffi::{self, Object};
use crate::gtk::{App, find, keys, palette, privacy};

pub(in crate::gtk::browsing) use policy::observe;

const KEY_UP: c_uint = 0xff52;
const KEY_DOWN: c_uint = 0xff54;
const KEY_RETURN: c_uint = 0xff0d;
const KEY_KP_ENTER: c_uint = 0xff8d;

#[derive(Default)]
pub(super) struct Downloads {
    policy: policy::Policy,
    transfers: transfer::Transfers,
    list: RefCell<List>,
    view: Cell<Option<view::View>>,
    opened: Cell<bool>,
    selected: Cell<Option<DownloadId>>,
    folder: OnceCell<PathBuf>,
    origins: RefCell<HashMap<DownloadId, Source>>,
    saved: RefCell<Option<DownloadsSnapshot>>,
    writes: OnceCell<PendingWrites>,
}

#[derive(Clone, Copy)]
struct Source {
    tab: TabId,
    jar: Jar,
    view: Object,
}

impl Source {
    fn from_view(app: &App, view: Object) -> Option<Self> {
        let tab = app.browsing.pages.tab(view)?;
        let jar = app.browsing.pages.jar(tab)?;
        Some(Self { tab, jar, view })
    }

    fn open(self, app: &App) -> bool {
        app.browsing.pages.view(self.tab) == Some(self.view)
    }

    fn private(self) -> bool {
        matches!(self.jar, Jar::Throwaway(_))
    }
}

impl Downloads {
    pub(super) fn attach(
        &self,
        application: Object,
        window: Object,
        overlay: Object,
        data: Object,
        isolated: bool,
    ) {
        if !isolated {
            if let Some(folder) = transfer::folder() {
                let _ = self.folder.set(folder);
            }
            self.policy.attach(application, data);
        }
        self.view.set(Some(view::View::new(window, overlay, data)));
    }

    pub(super) fn forget(&self, view: Object) {
        self.policy.forget(view);
        self.transfers.forget(view);
        for source in self
            .origins
            .borrow_mut()
            .values_mut()
            .filter(|source| source.view == view)
        {
            source.view = ptr::null_mut();
        }
    }

    pub(super) fn folder(&self) -> Option<&Path> {
        self.folder.get().map(PathBuf::as_path)
    }

    pub(super) fn detach(&self, application: Object) {
        self.view.set(None);
        self.policy.detach(application);
        self.transfers.detach(self);
    }

    pub(super) fn restore(&self, list: List, writes: PendingWrites) {
        self.saved.replace(Some(list.snapshot()));
        self.list.replace(list);
        let _ = self.writes.set(writes);
    }

    fn save(&self) {
        let Some(writes) = self.writes.get() else {
            return;
        };
        let mut snapshot = self.list.borrow().snapshot();
        let origins = self.origins.borrow();
        snapshot.downloads.retain(|download| {
            !origins
                .get(&download.id)
                .is_some_and(|source| source.private())
        });
        let mut saved = self.saved.borrow_mut();
        if saved
            .as_ref()
            .is_some_and(|previous| previous.downloads == snapshot.downloads)
        {
            return;
        }
        match writes.save_downloads(&snapshot) {
            Ok(()) => *saved = Some(snapshot),
            Err(error) => super::report(&error),
        }
    }

    fn start(&self, source: Source, url: Url, file_name: String) -> DownloadId {
        let id = self
            .list
            .borrow_mut()
            .start(url, file_name, SystemClock.now());
        self.origins.borrow_mut().insert(id, source);
        if !source.private() {
            self.save();
        }
        id
    }

    fn retry_source(&self, app: &App, id: DownloadId) -> Option<Source> {
        if let Some(source) = self.origins.borrow().get(&id).copied() {
            if source.open(app) {
                return Some(source);
            }
            if source.private() {
                return None;
            }
        }
        Source::from_view(app, app.widgets.get()?.view)
    }

    fn clear(&self) {
        let cleared = self.list.borrow_mut().clear_finished();
        let mut origins = self.origins.borrow_mut();
        let mut regular_changed = false;
        for download in cleared {
            regular_changed |= !origins.remove(&download.id).is_some_and(Source::private);
        }
        drop(origins);
        if regular_changed {
            self.save();
        }
    }

    fn rows(&self) -> Vec<Download> {
        let list = self.list.borrow();
        let mut rows: Vec<_> = list.all().collect();
        rows.reverse();
        rows.into_iter().take(view::ROWS).cloned().collect()
    }

    fn refresh(&self) {
        if !self.opened.get() {
            return;
        }
        let Some(view) = self.view.get() else { return };
        let rows = self.rows();
        let selected = self
            .selected
            .get()
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .or_else(|| (!rows.is_empty()).then_some(0));
        self.selected.set(selected.map(|index| rows[index].id));
        let clearable = self.list.borrow().all().any(|download| !running(download));
        view.render(&rows, selected, clearable);
    }

    fn step(&self, down: bool) {
        let rows = self.rows();
        let Some(selected) = self.selected.get() else {
            return;
        };
        let Some(index) = rows.iter().position(|row| row.id == selected) else {
            return;
        };
        let next = if down {
            (index + 1).min(rows.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.selected.set(Some(rows[next].id));
        self.refresh();
        if let Some(view) = self.view.get() {
            view.focus_row(next);
        }
    }

    fn action(&self, app: &App, id: DownloadId, cancel_running: bool) {
        let Some(download) = self.list.borrow().get(id).cloned() else {
            return;
        };
        match download.state {
            DownloadState::InProgress { .. } if cancel_running => {
                self.transfers.cancel(self, id);
            }
            DownloadState::InProgress { .. } => return,
            DownloadState::Finished => {
                let name = Path::new(&download.file_name);
                if name.file_name() == Some(name.as_os_str())
                    && let Some((view, path)) = self.view.get().zip(self.folder.get())
                {
                    view.reveal(&path.join(download.file_name));
                }
            }
            DownloadState::Cancelled | DownloadState::Failed(_) => {
                if let Some(source) = self.retry_source(app, id)
                    && self.transfers.retry(app, source, &download.url)
                {
                    let _ = self.list.borrow_mut().remove(id);
                    if !self
                        .origins
                        .borrow_mut()
                        .remove(&id)
                        .is_some_and(Source::private)
                    {
                        self.save();
                    }
                }
            }
        }
        self.refresh();
    }
}

fn running(download: &Download) -> bool {
    matches!(download.state, DownloadState::InProgress { .. })
}

pub(in crate::gtk) fn toggle(app: &App) {
    let downloads = &app.browsing.downloads;
    if downloads.opened.get() {
        hide(app, true);
        return;
    }
    let Some((panel, widgets)) = downloads.view.get().zip(app.widgets.get()) else {
        return;
    };
    app.browsing.panels.dismiss(app, false);
    privacy::close(app, &widgets, false);
    find::hide(app, &widgets, false);
    palette::hide(app, &widgets, false);
    downloads.opened.set(true);
    downloads.selected.set(None);
    downloads.refresh();
    panel.show(widgets.view, downloads.selected.get().is_some());
}

pub(in crate::gtk) fn hide(app: &App, focus: bool) {
    let downloads = &app.browsing.downloads;
    if downloads.opened.replace(false)
        && let Some(panel) = downloads.view.get()
    {
        panel.hide();
        if focus && let Some(widgets) = app.widgets.get() {
            // SAFETY: The active browser view is live on the GTK thread.
            unsafe { ffi::gtk_widget_grab_focus(widgets.view) };
        }
    }
}

pub(in crate::gtk) fn key_pressed(app: &App, key: c_uint, modifiers: c_uint) -> bool {
    let downloads = &app.browsing.downloads;
    let modifiers = modifiers & keys::SHORTCUT_MODIFIERS;
    if !downloads.opened.get() {
        return false;
    }
    match (key, modifiers) {
        (ffi::KEY_ESCAPE, 0) => hide(app, true),
        (KEY_UP | KEY_DOWN, 0) => downloads.step(key == KEY_DOWN),
        (KEY_RETURN | KEY_KP_ENTER, 0 | ffi::CONTROL) => {
            let Some(view) = downloads.view.get() else {
                return false;
            };
            if !view.row_focused() {
                return false;
            }
            if let Some(id) = downloads.selected.get() {
                downloads.action(app, id, false);
            }
        }
        _ => return false,
    }
    true
}

//! Browser tabs in one GTK window, with the shared transactional session.

pub(super) mod downloads;
mod engine;
pub(super) mod hints;
mod library;
pub(super) mod media;
mod origin;
mod panels;
pub(super) mod settings;

use std::cell::{Cell, RefCell};
use std::env;
use std::ffi::c_uint;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::ptr;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quickie_application::{AppError, Browsing as Session, Clock, Palette, PaletteIndex, Sources};
use quickie_domain_browsing::{
    Direction, Focus, JarChoice, PinnedSlot, Profile, TabId, Timestamp, WindowId,
};
use quickie_domain_commands::{Catalog, ContextFlag, FocusedView, Keymap, WhenContext};
use quickie_store::{PendingWrites, SqliteStore, WriteBehindStore};
use url::Url;

use super::ffi::{self, Object};
use super::{App, Options, active_view_changed, keys, palette, privacy};
use engine::{Engine, Pages};

type SessionState = Session<Engine, WriteBehindStore, SystemClock>;

#[derive(Default)]
pub(super) struct Browsing {
    state: RefCell<Option<SessionState>>,
    profile_lock: RefCell<Option<File>>,
    writes: RefCell<Option<PendingWrites>>,
    pages: Rc<Pages>,
    window: Cell<Option<WindowId>>,
    mounted: Cell<Object>,
    panels: panels::Panels,
    hints: hints::Hints,
    library: library::ProfileLibrary,
    origin: origin::Chip,
    media: media::Playback,
    downloads: downloads::Downloads,
    settings: settings::Settings,
}

#[derive(Debug)]
pub(super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_millis(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
    }
}

pub(super) fn attach(
    app: &App,
    window: Object,
    overlay: Object,
    data: Object,
) -> Result<Object, String> {
    let browser = &app.browsing;
    let isolated = matches!(app.options, Options::Smoke(_) | Options::Updates);
    let profile = lock_profile(isolated)?;
    let path = profile.as_ref().map(|(path, _)| path.as_path());
    let store = open_store(path)?;
    let library = open_store(path)?;
    let downloads = browser
        .library
        .restore(library)
        .map_err(|error| error.to_string())?;
    if !isolated {
        browser.downloads.restore(downloads, store.pending());
    }
    privacy::start(app, isolated);
    browser.media.attach(window, overlay, data);
    browser.hints.attach(overlay, data);
    browser.origin.attach(overlay, data);
    browser
        .downloads
        .attach(app.application, window, overlay, data, isolated);
    browser.settings.attach(window, overlay, data, isolated);
    let writes = store.pending();
    let engine = Engine::new(Rc::clone(&browser.pages), data, isolated);
    let home = home().map_err(|error| error.to_string())?;
    let state = Session::start(Profile::Main, engine, store, SystemClock, &home)
        .map_err(|error| error.to_string())?;
    let logical = state
        .session()
        .windows()
        .next()
        .ok_or("the session has no window")?;
    let initial = browser
        .pages
        .active(logical)
        .ok_or("the session has no active view")?;
    if let Some(error) = state.corrupt_on_start() {
        report(&error);
    }
    browser.window.set(Some(logical));
    browser.mounted.set(initial);
    browser.state.replace(Some(state));
    browser.profile_lock.replace(profile.map(|(_, lock)| lock));
    browser.writes.replace(Some(writes));
    browser.settings.mount(initial);
    browser.panels.attach(window, overlay, data);
    browser.run(SessionState::purge_trash);
    Ok(initial)
}

fn open_store(profile: Option<&Path>) -> Result<WriteBehindStore, String> {
    if let Some(profile) = profile {
        WriteBehindStore::open(&profile.join("shell.sqlite")).map_err(|error| error.to_string())
    } else {
        let store = SqliteStore::in_memory().map_err(|error| error.to_string())?;
        WriteBehindStore::spawn(store, Duration::ZERO).map_err(|error| error.to_string())
    }
}

fn lock_profile(isolated: bool) -> Result<Option<(PathBuf, File)>, String> {
    if isolated {
        return Ok(None);
    }
    let root = data_home()?;
    prepare_profile(&root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join("session.lock"))
        .map_err(|error| error.to_string())?;
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|error| format!("cannot lock the Quickie browsing session: {error}"))?;
    Ok(Some((root, lock)))
}

fn prepare_profile(root: &Path) -> Result<(), String> {
    use rustix::fs::{Mode, OFlags, fchmod, fstat, open};

    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(root)
        .map_err(|error| format!("cannot create the Quickie profile: {error}"))?;
    let profile = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open the Quickie profile: {error}"))?;
    let metadata = fstat(&profile).map_err(|error| error.to_string())?;
    if metadata.st_uid != rustix::process::geteuid().as_raw() {
        return Err("the Quickie profile is owned by another user".into());
    }
    if metadata.st_mode & 0o077 != 0 {
        fchmod(&profile, Mode::RWXU)
            .map_err(|error| format!("cannot protect the Quickie profile: {error}"))?;
    }
    Ok(())
}

fn data_home() -> Result<PathBuf, String> {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|path| path.join("quickie"))
        .ok_or_else(|| "cannot locate the Quickie profile directory".into())
}

fn home() -> Result<Url, url::ParseError> {
    Url::parse("about:blank")
}

fn report(error: &impl std::fmt::Display) {
    let _ = writeln!(io::stderr(), "Quickie browsing: {error}");
}

impl Browsing {
    fn run<T>(
        &self,
        operation: impl FnOnce(&mut SessionState) -> Result<T, AppError>,
    ) -> Option<T> {
        let mut state = self.state.try_borrow_mut().ok()?;
        match operation(state.as_mut()?) {
            Ok(value) => Some(value),
            Err(error) => {
                report(&error);
                None
            }
        }
    }

    fn settle(&self, app: &App) {
        palette::sources_changed(app);
        let Some(window) = self.window.get() else {
            return;
        };
        let Some(next) = self.pages.active(window) else {
            return;
        };
        let previous = self.mounted.replace(next);
        if previous != next {
            self.hints.hide();
            downloads::hide(app, false);
            self.pages.remember_picture(previous);
            active_view_changed(app, next);
            self.media.active_view_changed(app, previous, next);
            self.settings.set_page_fullscreen(false);
            self.settings.mount(next);
            self.media.mount_player(app);
            if !engine::observe_origin(next, app) {
                self.origin.forget();
            }
            // SAFETY: The new active view is a live widget in the browser overlay.
            unsafe { ffi::gtk_widget_grab_focus(next) };
        }
    }

    pub(in crate::gtk) fn active_tab(&self) -> Option<TabId> {
        self.state
            .try_borrow()
            .ok()?
            .as_ref()?
            .session()
            .active(self.window.get()?)
    }

    pub(super) fn palette_index(&self, keymap: &Keymap) -> Option<PaletteIndex> {
        let state = self.state.try_borrow().ok()?;
        let session = state.as_ref()?.session();
        self.library.with(|library| {
            Palette::new(Catalog::builtin(), SystemClock).open(&Sources {
                session,
                history: library.history(),
                bookmarks: library.bookmarks(),
                shortcuts: library.shortcuts(),
                keymap,
                extensions: &[],
            })
        })
    }

    fn close(&self, app: &App, tab: TabId) -> bool {
        let outcome = self.run(|state| state.close(tab));
        if let Some(closed) = &outcome
            && closed.next_active.is_none()
            && let Ok(home) = home()
        {
            self.run(|state| state.open(closed.window, home, JarChoice::Main, Focus::Foreground));
        }
        self.settle(app);
        outcome.is_some()
    }
}

pub(super) fn activate(app: &App, tab: TabId) {
    app.browsing.run(|state| state.activate(tab));
    app.browsing.settle(app);
}

pub(super) fn new_tab(app: &App, jar: JarChoice) {
    if let Some(window) = app.browsing.window.get()
        && let Ok(home) = home()
        && app
            .browsing
            .run(|state| state.open(window, home, jar, Focus::Foreground))
            .is_some()
    {
        app.browsing.settle(app);
        if let Some(widgets) = app.widgets.get() {
            palette::open_address(app, &widgets);
        }
    }
}

pub(super) fn close_active(app: &App) {
    if let Some(active) = app.browsing.active_tab() {
        app.browsing.close(app, active);
    }
}

pub(super) fn bookmark(app: &App) {
    let Some(id) = app.browsing.active_tab() else {
        return;
    };
    let state = app.browsing.state.borrow();
    let Some(tab) = state.as_ref().and_then(|state| state.session().tab(id)) else {
        return;
    };
    match app.browsing.library.bookmark(tab) {
        Ok(Some(saved)) if saved.created => palette::sources_changed(app),
        Ok(_) => {}
        Err(error) => report(&error),
    }
}

pub(super) fn reopen_closed(app: &App) {
    if let Some(window) = app.browsing.window.get() {
        app.browsing.run(|state| state.reopen_closed(window));
        app.browsing.settle(app);
    }
}

pub(super) fn show_overview(app: &App) {
    app.browsing.panels.toggle_overview(app);
}

pub(super) fn dismiss_panels(app: &App) {
    app.browsing.panels.dismiss(app, false);
}

pub(super) fn hide_page_overlays(app: &App) {
    app.browsing.hints.hide();
    app.browsing.origin.hide();
}

pub(super) fn cycle(app: &App, direction: Direction, held: bool) {
    if held {
        app.browsing.panels.step_cycle(app, direction);
        return;
    }
    if let Some(window) = app.browsing.window.get() {
        app.browsing.run(|state| {
            state
                .begin_cycle(window, direction)
                .map(|cycle| state.commit_cycle(&cycle))
                .transpose()
        });
        app.browsing.settle(app);
    }
}

pub(super) fn key_pressed(app: &App, key: c_uint, modifiers: c_uint) -> bool {
    let modifiers = modifiers & keys::SHORTCUT_MODIFIERS;
    app.browsing.panels.key_pressed(app, key, modifiers)
}

pub(super) fn use_pinned_slot(app: &App, slot: PinnedSlot, pin: bool) {
    if pin {
        if let Some(tab) = app.browsing.active_tab() {
            app.browsing.run(|state| state.pin(tab, slot));
        }
    } else {
        app.browsing.run(|state| state.go_to_slot(slot));
        app.browsing.settle(app);
    }
}

pub(super) fn key_context(app: &App) -> WhenContext {
    match app.browsing.panels.key_mode() {
        Some(panels::Mode::Overview) => WhenContext::new()
            .in_view(FocusedView::Overview)
            .with(ContextFlag::OverviewOpen),
        Some(panels::Mode::Switcher) => WhenContext::new()
            .in_view(FocusedView::Page)
            .with(ContextFlag::SwitcherOpen),
        None => WhenContext::new()
            .in_view(FocusedView::Page)
            .with(ContextFlag::PageFocused),
    }
}

pub(super) fn key_released(app: &App, key: c_uint, _modifiers: c_uint) {
    if matches!(key, 0xffe3 | 0xffe4) {
        app.browsing.panels.commit_cycle(app);
    }
}

pub(super) fn detach(app: &App) {
    app.browsing.hints.hide();
    app.browsing.origin.shutdown();
    app.browsing.downloads.detach(app.application);
    app.browsing.media.shutdown(app);
    app.browsing.panels.detach();
    app.browsing.settings.detach();
    app.browsing.library.flush();
    if let Some(writes) = app.browsing.writes.take()
        && let Err(error) = writes.flush()
    {
        report(&error);
    }
    app.browsing.mounted.set(ptr::null_mut());
}

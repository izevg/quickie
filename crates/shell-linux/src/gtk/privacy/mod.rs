//! Shields for the GTK browser, using the shared app use cases and `WebKit`'s content filters.

mod blocker;
mod cosmetic;
mod fetch;
mod permissions;
mod view;

use std::cell::{Cell, RefCell};
use std::env;
use std::ffi::{CStr, CString, c_int, c_uint};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use quickie_application::{
    Clock, ContentBlockerPort, FetchedList, ListFetch, RulesStatus, Shields,
};
use quickie_domain_shields::{FilterListId, Refresh, Site, SitePolicy, Timestamp};
use quickie_store::SqliteStore;
use url::Url;

use super::{App, Widgets, connect, ffi, find, keys, palette};
use blocker::NativeBlocker;
use fetch::Fetches;
use ffi::{Bool, Object};
pub(super) use view::View;
use view::{Panel, Reload, Summary};

const KEY_RETURN: c_uint = 0xff0d;
const KEY_KP_ENTER: c_uint = 0xff8d;
const LOAD_COMMITTED: c_int = 2;
const LOAD_STARTED: c_int = 0;
const LISTS: [FilterListId; 5] = FilterListId::ALL;

type Service = Shields<SqliteStore, WallClock, NativeBlocker>;

#[derive(Default)]
pub(super) struct Privacy {
    shields: RefCell<Option<Service>>,
    engine: NativeBlocker,
    panel: Cell<Option<Panel>>,
    rendering: Cell<bool>,
    fetches: RefCell<Fetches>,
    source: Cell<c_uint>,
    queued: RefCell<Vec<(Object, CString)>>,
    observed: RefCell<Vec<Object>>,
    reload: RefCell<Vec<Object>>,
    queued_reload: Cell<Option<Object>>,
    reloading: Cell<bool>,
    notice: RefCell<String>,
    permissions: permissions::Permissions,
}

impl Privacy {
    pub(super) fn stop(&self) {
        let source = self.source.replace(0);
        if source != 0 {
            // SAFETY: This is the timer installed by kick, using the application as its data.
            unsafe { ffi::g_source_remove(source) };
        }
        self.fetches.replace(Fetches::default());
        self.permissions.clear();
        self.engine.cosmetic.stop();
    }
}

#[derive(Clone, Copy)]
struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> Timestamp {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_millis(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
    }
}

pub(super) fn start(app: &App, smoke: bool) {
    let store = if smoke {
        SqliteStore::in_memory().map_err(|error| error.to_string())
    } else {
        open_store()
    };
    let service = store.and_then(|repo| {
        Shields::start(repo, WallClock, app.privacy.engine.clone())
            .map_err(|error| error.to_string())
    });
    match service {
        Ok(shields) => {
            let warnings = shields
                .corrupt_on_start()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            app.privacy.notice.replace(warnings.join("\n"));
            let due = shields.due_fetches();
            app.privacy.shields.replace(Some(shields));
            start_fetches(app, due);
        }
        Err(error) => report(app, format!("Privacy preferences unavailable: {error}")),
    }
}

pub(super) fn attach(overlay: Object, data: Object) -> View {
    let view = View::new(overlay, data);
    connect(overlay, c"get-child-position", layout as *const _, data);
    for toggle in view
        .site_switches
        .into_iter()
        .chain(view.list_switches)
        .chain([view.keep_fresh])
    {
        connect(toggle, c"notify::active", toggle_changed as *const _, data);
    }
    for button in [
        view.close,
        view.settings,
        view.enable,
        view.refresh,
        view.reload,
    ] {
        connect(button, c"clicked", clicked as *const _, data);
    }
    view
}

fn open_store() -> Result<SqliteStore, String> {
    let directory = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or("Cannot locate the browser profile")?
        .join("quickie");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    SqliteStore::open(&directory.join("shell.sqlite")).map_err(|error| error.to_string())
}

pub(super) fn observe(web: Object, data: Object) {
    // SAFETY: The composition root passes the live App and web view on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if app.privacy.engine.observe(web, data) {
        app.privacy.observed.borrow_mut().push(web);
        connect(web, c"load-changed", load_changed as *const _, data);
        permissions::observe(web, data);
    }
}

pub(super) fn forget(app: &App, web: Object) {
    permissions::dismiss_for(app, web);
    app.privacy.engine.forget(web);
    app.privacy
        .observed
        .borrow_mut()
        .retain(|view| *view != web);
    app.privacy.reload.borrow_mut().retain(|view| *view != web);
    if app.privacy.queued_reload.get() == Some(web) {
        app.privacy.queued_reload.set(None);
    }
    app.privacy
        .queued
        .borrow_mut()
        .retain(|(view, _)| *view != web);
}

pub(super) fn navigate(app: &App, web: Object, uri: &CStr) {
    if app.privacy.engine.pending() {
        let mut queued = app.privacy.queued.borrow_mut();
        queued.retain(|(view, _)| *view != web);
        queued.push((web, uri.to_owned()));
        return;
    }
    load(web, uri);
}

fn load(web: Object, uri: &CStr) {
    permissions::load_requested(web, uri);
    // SAFETY: The view is live and WebKit copies the NUL-terminated navigation URI.
    unsafe { ffi::webkit_web_view_load_uri(web, uri.as_ptr()) };
}

pub(super) fn key_pressed(app: &App, widgets: &Widgets, key: c_uint, modifiers: c_uint) -> bool {
    const SHIELDS_MODIFIERS: c_uint = ffi::ALT | ffi::CONTROL;
    if permissions::key(app, widgets, key, modifiers) {
        return true;
    }
    match (key, modifiers & keys::SHORTCUT_MODIFIERS) {
        (0x53 | 0x73, SHIELDS_MODIFIERS) => show_shields(app, widgets),
        (ffi::KEY_ESCAPE, 0) if app.privacy.panel.get().is_some() => close(app, widgets, true),
        (KEY_RETURN | KEY_KP_ENTER, 0)
            if app.privacy.panel.get() == Some(Panel::Shields)
                && app.privacy.reload.borrow().contains(&widgets.view) =>
        {
            reload(app, widgets);
        }
        _ => return false,
    }
    true
}

pub(super) fn show_shields(app: &App, widgets: &Widgets) {
    if app.privacy.panel.get() == Some(Panel::Shields) {
        close(app, widgets, true);
    } else {
        show(app, widgets, Panel::Shields);
    }
}

fn show(app: &App, widgets: &Widgets, panel: Panel) {
    find::hide(app, widgets, false);
    palette::hide(app, widgets, false);
    app.privacy.panel.set(Some(panel));
    refresh(app, widgets);
    widgets.privacy.show(widgets.view);
}

pub(super) fn close(app: &App, widgets: &Widgets, focus: bool) {
    app.privacy.queued_reload.set(None);
    if app.privacy.panel.replace(None).is_some() {
        widgets.privacy.hide(widgets.view, focus);
    }
}

pub(super) fn refresh(app: &App, widgets: &Widgets) {
    permissions::active_changed(app, widgets);
    let Some(panel) = app.privacy.panel.get() else {
        return;
    };
    let site =
        address(widgets.view).and_then(|url| Site::try_from(&url).ok().map(|site| (url, site)));
    let engine = app.privacy.engine.status();
    let fetching = app.privacy.fetches.borrow().active();
    let reload = reload_status(app, widgets.view);
    let shields = app.privacy.shields.borrow();
    let service = shields.as_ref();
    let site = site.map(|(url, site)| {
        let policy = service
            .and_then(|shields| shields.policy(&url))
            .unwrap_or_default();
        (
            site.to_string(),
            [policy.shields, policy.https_only, policy.javascript],
        )
    });
    let status = match engine {
        RulesStatus::Idle => String::new(),
        RulesStatus::Compiling => "Compiling rules…".to_owned(),
        RulesStatus::Active(rules) if rules.rules == 0 => "No network rules active".to_owned(),
        RulesStatus::Active(rules) => format!("Rules active: {} network rules", rules.rules),
        RulesStatus::Failed(reason) => format!("Failed: {reason}"),
    };
    let notice = app.privacy.notice.borrow();
    let cosmetic = if app.privacy.engine.cosmetic.failed() {
        "Cosmetic filters failed to load"
    } else if app.privacy.engine.cosmetics_ready() {
        ""
    } else {
        "Cosmetic filters loading…"
    };
    let activity = if fetching {
        "Refreshing filter lists…"
    } else {
        ""
    };
    let status = [status.as_str(), cosmetic, activity, &notice]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let summary = Summary {
        panel,
        site,
        lists: LISTS.map(|id| {
            service.map_or_else(
                || (false, String::new()),
                |shields| {
                    let list = shields.lists().get(id);
                    let detail = if !list.enabled {
                        "Off".to_owned()
                    } else if let Some(fetched) = list.fetched {
                        if WallClock.now().millis_since(fetched.at) < Timestamp::DAY_MS {
                            "Fetched today".to_owned()
                        } else {
                            format!(
                                "Fetched {} days ago",
                                WallClock.now().millis_since(fetched.at) / Timestamp::DAY_MS
                            )
                        }
                    } else {
                        "Not fetched yet".to_owned()
                    };
                    (list.enabled, detail)
                },
            )
        }),
        keep_fresh: service.is_some_and(|shields| shields.lists().refresh() == Refresh::KeepFresh),
        reload,
        status,
        available: service.is_some(),
        busy: fetching,
    };
    drop(notice);
    drop(shields);
    app.privacy.rendering.set(true);
    widgets.privacy.render(&summary);
    app.privacy.rendering.set(false);
}

fn address(web: Object) -> Option<Url> {
    // SAFETY: WebKit owns the synchronous URI pointer returned for this live view.
    let uri = unsafe { ffi::webkit_web_view_get_uri(web) };
    if uri.is_null() {
        return None;
    }
    // SAFETY: A non-null WebKit URI is NUL-terminated and borrowed until the next load.
    Url::parse(unsafe { CStr::from_ptr(uri) }.to_str().ok()?).ok()
}

fn report(app: &App, detail: String) {
    app.privacy.notice.replace(detail);
}

fn start_fetches(app: &App, fetches: Vec<ListFetch>) {
    let errors = app.privacy.fetches.borrow_mut().start(fetches);
    finish_fetches(app, errors);
    kick(app);
}

fn finish_fetches(app: &App, fetched: Vec<FetchedList>) {
    if fetched.is_empty() {
        return;
    }
    let mut shields = app.privacy.shields.borrow_mut();
    let Some(shields) = shields.as_mut() else {
        return;
    };
    match shields.lists_fetched(fetched) {
        Ok(finished) => {
            if !finished.updated.is_empty() {
                app.privacy.reloading.set(true);
            }
            for (list, error) in finished.failed {
                report(app, format!("{} refresh failed: {error}", list.title()));
            }
        }
        Err(error) => report(app, error.to_string()),
    }
}

fn kick(app: &App) {
    if app.privacy.source.get() == 0 && working(app) {
        // SAFETY: The App remains live until Privacy::stop removes the timer after the event loop.
        let source =
            unsafe { ffi::g_timeout_add(35, tick, std::ptr::from_ref(app).cast_mut().cast()) };
        app.privacy.source.set(source);
    }
}

fn working(app: &App) -> bool {
    app.privacy.engine.pending()
        || app.privacy.engine.cosmetic.working()
        || app.privacy.fetches.borrow().active()
}

extern "C" fn tick(data: Object) -> Bool {
    // SAFETY: The timer installed by kick is scoped to this live App.
    let app = unsafe { &*data.cast::<App>() };
    let fetched = app.privacy.fetches.borrow_mut().poll();
    let changed = !fetched.is_empty();
    finish_fetches(app, fetched);
    let cosmetic_changed = app.privacy.engine.cosmetic.poll();
    let settled = app.privacy.engine.poll();
    if settled {
        let reloading = app.privacy.reloading.replace(false);
        if reloading && matches!(app.privacy.engine.status(), RulesStatus::Active(_)) {
            app.privacy
                .reload
                .replace(app.privacy.observed.borrow().clone());
        }
    }
    if !app.privacy.engine.pending() {
        let queued = app.privacy.queued.take();
        for (view, uri) in queued {
            load(view, &uri);
        }
    }
    if (changed || settled || cosmetic_changed)
        && let Some(widgets) = app.widgets.get()
    {
        refresh(app, &widgets);
        if app.privacy.queued_reload.get() == Some(widgets.view)
            && reload_status(app, widgets.view) == Reload::Ready
        {
            reload(app, &widgets);
        }
    }
    let more = working(app);
    if !more {
        app.privacy.source.set(0);
    }
    Bool::from(more)
}

fn reload_status(app: &App, web: Object) -> Reload {
    if !app.privacy.reload.borrow().contains(&web) {
        return Reload::Hidden;
    }
    match app.privacy.engine.status() {
        RulesStatus::Failed(_) => Reload::Hidden,
        RulesStatus::Compiling => Reload::Pending,
        _ if app.privacy.fetches.borrow().active() => Reload::Pending,
        _ => Reload::Ready,
    }
}

fn reload(app: &App, widgets: &Widgets) {
    match reload_status(app, widgets.view) {
        Reload::Hidden => {}
        Reload::Pending => app.privacy.queued_reload.set(Some(widgets.view)),
        Reload::Ready => {
            close(app, widgets, true);
            // SAFETY: The active WebKit view is live and the latest settled filters are attached.
            unsafe { ffi::webkit_web_view_reload(widgets.view) };
        }
    }
}

extern "C" fn load_changed(web: Object, event: c_int, data: Object) {
    // SAFETY: The live view emits this signal on the GTK thread with the App as its data.
    let app = unsafe { &*data.cast::<App>() };
    if event == LOAD_STARTED {
        permissions::dismiss_for(app, web);
    }
    if event != LOAD_COMMITTED {
        return;
    }
    if !app.privacy.engine.pending() {
        app.privacy.reload.borrow_mut().retain(|view| *view != web);
    }
    if let Some(widgets) = app.widgets.get().filter(|widgets| widgets.view == web) {
        refresh(app, &widgets);
    }
}

extern "C" fn clicked(button: Object, data: Object) {
    // SAFETY: The live button signal runs on the GTK thread with the App as its data.
    let app = unsafe { &*data.cast::<App>() };
    let Some(widgets) = app.widgets.get() else {
        return;
    };
    let view = widgets.privacy;
    if view.close == button {
        close(app, &widgets, true);
    } else if view.settings == button {
        show(app, &widgets, Panel::Settings);
    } else if view.reload == button {
        reload(app, &widgets);
    } else if view.enable == button {
        set_lists(app, &LISTS, true);
    } else if view.refresh == button {
        report(app, String::new());
        let fetches = app
            .privacy
            .shields
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, Shields::requested_fetches);
        start_fetches(app, fetches);
    }
    refresh(app, &widgets);
}

extern "C" fn toggle_changed(toggle: Object, _: Object, data: Object) {
    // SAFETY: The live switch signal runs on the GTK thread with the App as its data.
    let app = unsafe { &*data.cast::<App>() };
    if app.privacy.rendering.get() {
        return;
    }
    let Some(widgets) = app.widgets.get() else {
        return;
    };
    let view = widgets.privacy;
    // SAFETY: This signal comes only from the live switches attached by this module.
    let active = unsafe { ffi::gtk_switch_get_active(toggle) } != ffi::FALSE;
    if let Some(index) = view
        .site_switches
        .iter()
        .position(|&widget| widget == toggle)
    {
        set_site(app, widgets.view, index, active);
    } else if let Some(index) = view
        .list_switches
        .iter()
        .position(|&widget| widget == toggle)
    {
        set_lists(app, &[LISTS[index]], active);
    } else if toggle == view.keep_fresh {
        set_refresh(app, active);
    }
    refresh(app, &widgets);
}

fn set_site(app: &App, web: Object, index: usize, active: bool) {
    let Some(url) = address(web) else { return };
    let mut store = app.privacy.shields.borrow_mut();
    let Some(shields) = store.as_mut() else {
        return;
    };
    let Some(policy) = shields.policy(&url) else {
        return;
    };
    let next = match index {
        0 => SitePolicy {
            shields: active,
            ..policy
        },
        1 => SitePolicy {
            https_only: active,
            ..policy
        },
        _ => SitePolicy {
            javascript: active,
            ..policy
        },
    };
    match shields.set_policy(&url, next) {
        Ok(true) if index == 0 => {
            let mut reload = app.privacy.reload.borrow_mut();
            if !reload.contains(&web) {
                reload.push(web);
            }
        }
        Ok(_) => {}
        Err(error) => report(app, error.to_string()),
    }
    drop(store);
    kick(app);
}

fn set_lists(app: &App, lists: &[FilterListId], enabled: bool) {
    report(app, String::new());
    let mut store = app.privacy.shields.borrow_mut();
    let Some(shields) = store.as_mut() else {
        return;
    };
    let changed = if enabled {
        shields.enable_lists(lists)
    } else {
        shields.disable_lists(lists)
    };
    match changed {
        Ok(changed) if !changed.is_empty() => app.privacy.reloading.set(true),
        Ok(_) => {}
        Err(error) => report(app, error.to_string()),
    }
    let due = if enabled {
        shields.due_fetches()
    } else {
        Vec::new()
    };
    drop(store);
    start_fetches(app, due);
}

fn set_refresh(app: &App, active: bool) {
    let mut store = app.privacy.shields.borrow_mut();
    let Some(shields) = store.as_mut() else {
        return;
    };
    let refresh = if active {
        Refresh::KeepFresh
    } else {
        Refresh::OnRequest
    };
    if let Err(error) = shields.set_refresh(refresh) {
        report(app, error.to_string());
    }
    let due = if active {
        shields.due_fetches()
    } else {
        Vec::new()
    };
    drop(store);
    start_fetches(app, due);
}

extern "C" fn layout(
    overlay: Object,
    child: Object,
    allocation: *mut palette::Allocation,
    data: Object,
) -> Bool {
    // SAFETY: GTK emits layout for this live overlay and passes the original App and allocation.
    let app = unsafe { &*data.cast::<App>() };
    let Some(widgets) = app
        .widgets
        .get()
        .filter(|widgets| widgets.privacy.panel == child)
    else {
        return ffi::FALSE;
    };
    let Some(panel) = app.privacy.panel.get() else {
        return ffi::FALSE;
    };
    // SAFETY: GtkOverlay supplied its writable allocation for the duration of the signal.
    widgets
        .privacy
        .layout(overlay, panel, unsafe { &mut *allocation });
    ffi::TRUE
}

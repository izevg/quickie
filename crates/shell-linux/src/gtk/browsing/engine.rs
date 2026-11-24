//! The application's page port over independent native `WebKit` views.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;
use std::rc::Rc;

use quickie_application::{EngineError, EnginePort, Navigation};
use quickie_domain_browsing::{Focus, Jar, JarId, TabId, WindowId};
use url::Url;

use super::super::ffi::{self, Bool, Object};
use super::super::{App, agent, connect, find, inspector, keys, privacy};
use super::{activate, downloads, home, media};

const LOAD_STARTED: c_int = 0;
const LOAD_COMMITTED: c_int = 2;
const LOAD_FINISHED: c_int = 3;
const THUMBNAIL_LIMIT: usize = 24;
static THUMBNAIL_SIZE: PaintSize = PaintSize {
    width: 160.0,
    height: 100.0,
};

#[repr(C)]
struct PaintSize {
    width: f32,
    height: f32,
}

#[derive(Debug)]
struct Native(Object);

impl Native {
    fn owned(object: Object) -> Option<Self> {
        (!object.is_null()).then(|| Self(object))
    }

    fn widget(object: Object) -> Option<Self> {
        if object.is_null() {
            return None;
        }
        // SAFETY: Newly constructed GTK widgets have a floating reference to sink.
        Self::owned(unsafe { g_object_ref_sink(object) })
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: Each wrapper owns one reference on the GTK thread.
        unsafe { ffi::g_object_unref(self.0) };
    }
}

#[derive(Debug)]
pub(super) struct Page {
    widget: Native,
    jar: Jar,
    committed: Cell<bool>,
}

impl Page {
    fn new(view: Object, data: Object, jar: Jar) -> Result<Self, EngineError> {
        let widget = Native::widget(view)
            .ok_or_else(|| EngineError("cannot create a WebKit view".into()))?;
        // SAFETY: The new view owns its settings on this thread.
        let settings = unsafe { ffi::webkit_web_view_get_settings(view) };
        // SAFETY: These settings live as long as the view.
        unsafe { ffi::webkit_settings_set_enable_developer_extras(settings, ffi::TRUE) };
        inspector::observe_detachment(view, data);
        keys::focus::observe(view);
        find::observe(view, data);
        downloads::observe(view, data);
        privacy::observe(view, data);
        media::bind(view, data);
        connect(view, c"load-changed", load_changed as *const _, data);
        connect(view, c"notify::title", metadata_changed as *const _, data);
        connect(view, c"notify::uri", metadata_changed as *const _, data);
        connect(view, c"create", create_requested as *const _, data);
        connect(view, c"close", close_requested as *const _, data);
        Ok(Self {
            widget,
            jar,
            committed: Cell::new(false),
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct Pages {
    entries: RefCell<HashMap<TabId, Page>>,
    shown: RefCell<HashMap<WindowId, TabId>>,
    pictures: RefCell<VecDeque<(TabId, Native)>>,
}

impl Pages {
    pub(super) fn active(&self, window: WindowId) -> Option<Object> {
        self.view(*self.shown.borrow().get(&window)?)
    }

    pub(super) fn view(&self, tab: TabId) -> Option<Object> {
        self.entries.borrow().get(&tab).map(|page| page.widget.0)
    }

    pub(super) fn tab(&self, view: Object) -> Option<TabId> {
        self.entries
            .borrow()
            .iter()
            .find_map(|(&tab, page)| (page.widget.0 == view).then_some(tab))
    }

    pub(super) fn jar(&self, tab: TabId) -> Option<Jar> {
        self.entries.borrow().get(&tab).map(|page| page.jar)
    }

    fn forget_picture(&self, tab: TabId) {
        self.pictures
            .borrow_mut()
            .retain(|(owner, _)| *owner != tab);
    }

    pub(super) fn remember_picture(&self, view: Object) {
        let Some(tab) = self.tab(view) else { return };
        // SAFETY: The live view is still owned by the engine on the GTK thread.
        if unsafe { gtk_widget_get_mapped(view) } == ffi::FALSE {
            return;
        }
        // SAFETY: The constructor returns a new paintable for the currently mounted view.
        let Some(source) = Native::owned(unsafe { gtk_widget_paintable_new(view) }) else {
            return;
        };
        // SAFETY: The immutable image is a new reference to the view's current frame.
        let Some(picture) = Native::owned(unsafe { gdk_paintable_get_current_image(source.0) })
        else {
            return;
        };
        // SAFETY: Both queries read the live immutable paintable.
        let width = unsafe { gdk_paintable_get_intrinsic_width(picture.0) };
        // SAFETY: The immutable paintable remains live.
        let height = unsafe { gdk_paintable_get_intrinsic_height(picture.0) };
        if width <= 0 || height <= 0 {
            return;
        }
        // SAFETY: GTK creates a new snapshot on the same thread as the source paintable.
        let snapshot = unsafe { gtk_snapshot_new() };
        // SAFETY: The texture and allocated snapshot are live. Dimensions are positive.
        unsafe {
            gdk_paintable_snapshot(
                picture.0,
                snapshot,
                f64::from(THUMBNAIL_SIZE.width),
                f64::from(THUMBNAIL_SIZE.height),
            );
        }
        // SAFETY: The new paintable retains the texture in card-sized bounds.
        let thumbnail = unsafe { gtk_snapshot_to_paintable(snapshot, &raw const THUMBNAIL_SIZE) };
        // SAFETY: This releases the caller's reference to the recorded snapshot.
        unsafe { ffi::g_object_unref(snapshot) };
        let Some(picture) = Native::owned(thumbnail) else {
            return;
        };
        self.forget_picture(tab);
        let mut pictures = self.pictures.borrow_mut();
        pictures.push_front((tab, picture));
        pictures.truncate(THUMBNAIL_LIMIT);
    }

    pub(super) fn picture(&self, tab: TabId) -> Object {
        self.pictures
            .borrow()
            .iter()
            .find_map(|(owner, picture)| (*owner == tab).then_some(picture.0))
            .unwrap_or(ptr::null_mut())
    }
}

#[derive(Debug)]
pub(super) struct Engine {
    pages: Rc<Pages>,
    jars: HashMap<Jar, Native>,
    data: Object,
    isolated: bool,
}

impl Engine {
    pub(super) fn new(pages: Rc<Pages>, data: Object, isolated: bool) -> Self {
        Self {
            pages,
            jars: HashMap::new(),
            data,
            isolated,
        }
    }

    fn new_view(&mut self, jar: Jar) -> Result<Object, EngineError> {
        if jar == Jar::Main && !self.isolated {
            // SAFETY: GTK and the default profile's WebKit context already exist.
            return Ok(unsafe { ffi::webkit_web_view_new() });
        }
        let session = match self.jars.entry(jar) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                // SAFETY: WebKit returns a wholly ephemeral network session with a new reference.
                let session = Native::owned(unsafe { webkit_network_session_new_ephemeral() })
                    .ok_or_else(|| EngineError("cannot isolate WebKit storage".into()))?;
                entry.insert(session)
            }
        };
        // SAFETY: This live session has the type required by the network-session construction
        // property. The final NULL terminates the GObject property list.
        let kind = unsafe { webkit_web_view_get_type() };
        // SAFETY: The valid view type and session remain live during construction.
        Ok(unsafe {
            g_object_new(
                kind,
                c"network-session".as_ptr(),
                session.0,
                ptr::null::<c_char>(),
            )
        })
    }
}

impl EnginePort for Engine {
    type Page = Page;

    fn adopt(&mut self, page: Page, tab: TabId) -> Result<(), EngineError> {
        self.pages.entries.borrow_mut().insert(tab, page);
        Ok(())
    }

    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        let view = self.new_view(jar)?;
        let page = Page::new(view, self.data, jar)?;
        self.adopt(page, tab)?;
        self.navigate(tab, &Navigation::Load(url.clone()))
    }

    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError> {
        if self.pages.view(tab).is_none() {
            return Err(EngineError("the tab has no WebKit view".into()));
        }
        self.pages.shown.borrow_mut().insert(window, tab);
        Ok(())
    }

    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError> {
        let view = self
            .pages
            .view(tab)
            .ok_or_else(|| EngineError("the tab has no WebKit view".into()))?;
        match navigation {
            Navigation::Load(url) => {
                let uri =
                    CString::new(url.as_str()).map_err(|error| EngineError(error.to_string()))?;
                // SAFETY: This is the App from the active GTK event loop.
                let app = unsafe { &*self.data.cast::<App>() };
                privacy::navigate(app, view, &uri);
            }
            // SAFETY: The engine owns the live view on the GTK thread.
            Navigation::Back => unsafe { ffi::webkit_web_view_go_back(view) },
            // SAFETY: The engine owns the live view on the GTK thread.
            Navigation::Forward => unsafe { ffi::webkit_web_view_go_forward(view) },
            // SAFETY: The engine owns the live view on the GTK thread.
            Navigation::Reload => unsafe { ffi::webkit_web_view_reload(view) },
            // SAFETY: The engine owns the live view on the GTK thread.
            Navigation::HardReload => unsafe { webkit_web_view_reload_bypass_cache(view) },
            // SAFETY: The engine owns the live view on the GTK thread.
            Navigation::Stop => unsafe { webkit_web_view_stop_loading(view) },
        }
        Ok(())
    }

    fn close(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.pages
            .shown
            .borrow_mut()
            .retain(|_, owner| *owner != tab);
        self.pages.forget_picture(tab);
        let page = self.pages.entries.borrow_mut().remove(&tab);
        if let Some(page) = page {
            // SAFETY: This is the App from the active GTK event loop.
            let app = unsafe { &*self.data.cast::<App>() };
            app.browsing.media.tab_closed(tab);
            agent::source_removed(app, page.widget.0);
            privacy::forget(app, page.widget.0);
            app.browsing.downloads.forget(page.widget.0);
            // SAFETY: The live page owns its inspector until the end of this block.
            let inspector = unsafe { ffi::webkit_web_view_get_inspector(page.widget.0) };
            // SAFETY: Closing a live inspector is idempotent.
            unsafe { ffi::webkit_web_inspector_close(inspector) };
        }
        Ok(())
    }

    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError> {
        self.jars.remove(&Jar::Throwaway(jar));
        Ok(())
    }
}

extern "C" fn load_changed(view: Object, event: c_int, data: Object) {
    // SAFETY: The handlers run on the GTK thread with the registered App reference.
    let app = unsafe { &*data.cast::<App>() };
    if event == LOAD_STARTED {
        keys::focus::reset(view);
    }
    if matches!(event, LOAD_STARTED | LOAD_COMMITTED)
        && let Some(tab) = app.browsing.pages.tab(view)
        && let Some(page) = app.browsing.pages.entries.borrow().get(&tab)
    {
        page.committed.set(event == LOAD_COMMITTED);
    }
    if matches!(event, LOAD_COMMITTED | LOAD_FINISHED) {
        record_metadata(view, app);
    }
}

extern "C" fn metadata_changed(view: Object, _: Object, data: Object) {
    // SAFETY: The view is live while it emits a property notification.
    if unsafe { webkit_web_view_is_loading(view) } != ffi::FALSE {
        return;
    }
    // SAFETY: The handlers run on the GTK thread with the registered App reference.
    let app = unsafe { &*data.cast::<App>() };
    record_metadata(view, app);
}

fn record_metadata(view: Object, app: &App) {
    let Some(tab) = app.browsing.pages.tab(view) else {
        return;
    };
    if !app
        .browsing
        .pages
        .entries
        .borrow()
        .get(&tab)
        .is_some_and(|page| page.committed.get())
    {
        return;
    }
    // SAFETY: WebKit owns the nullable URI while this callback executes.
    let uri = unsafe { ffi::webkit_web_view_get_uri(view) };
    let Some(url) = utf8(uri).and_then(|uri| Url::parse(&uri).ok()) else {
        return;
    };
    // SAFETY: WebKit owns the nullable title while this callback executes.
    let title = unsafe { ffi::webkit_web_view_get_title(view) };
    app.browsing
        .run(|state| state.navigated(tab, url, utf8(title).unwrap_or_default()));
}

fn utf8(text: *const c_char) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: Callers pass a live WebKit string and copy it immediately.
    Some(
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned(),
    )
}

extern "C" fn create_requested(opener: Object, action: Object, data: Object) -> Object {
    // SAFETY: The callback has the App reference and a live opener on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let Some(tab) = app.browsing.pages.tab(opener) else {
        return ptr::null_mut();
    };
    let Some(jar) = app.browsing.pages.jar(tab) else {
        return ptr::null_mut();
    };
    // SAFETY: The signal supplies a live navigation action and its borrowed request.
    let request = unsafe { webkit_navigation_action_get_request(action) };
    // SAFETY: The live request owns its URI during the signal.
    let uri = unsafe { webkit_uri_request_get_uri(request) };
    let Some(url) = utf8(uri)
        .and_then(|uri| Url::parse(&uri).ok())
        .or_else(|| home().ok())
    else {
        return ptr::null_mut();
    };
    // SAFETY: WebKit's creation signal requires related-view to be the live opener.
    let kind = unsafe { webkit_web_view_get_type() };
    // SAFETY: The live opener supplies the process and network session; the property list ends in NULL.
    let view = unsafe {
        g_object_new(
            kind,
            c"related-view".as_ptr(),
            opener,
            ptr::null::<c_char>(),
        )
    };
    let Ok(page) = Page::new(view, data, jar) else {
        return ptr::null_mut();
    };
    // SAFETY: Keep the native view valid through error cleanup even when adoption drops its page.
    let guard = Native::owned(unsafe { g_object_ref(view) });
    if app
        .browsing
        .run(|state| state.adopt(tab, page, url, Focus::Background))
        .is_none()
    {
        privacy::forget(app, view);
        return ptr::null_mut();
    }
    drop(guard);
    // SAFETY: Both values come from the live navigation action supplied by this signal.
    let modifiers = unsafe { webkit_navigation_action_get_modifiers(action) };
    // SAFETY: The live action stores the numeric mouse button.
    let button = unsafe { webkit_navigation_action_get_mouse_button(action) };
    if button != 2 && modifiers & ffi::CONTROL == 0 {
        connect(view, c"ready-to-show", ready_to_show as *const _, data);
    }
    // SAFETY: The engine has retained the view, and WebKit takes a separate reference.
    unsafe { g_object_ref(view) }
}

extern "C" fn ready_to_show(view: Object, data: Object) {
    // SAFETY: GTK calls the registered handler with the live App reference.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(tab) = app.browsing.pages.tab(view) {
        activate(app, tab);
    }
}

extern "C" fn close_requested(view: Object, data: Object) {
    // SAFETY: GTK calls the registered handler with the live App reference.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(tab) = app.browsing.pages.tab(view) {
        app.browsing.close(app, tab);
    }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_get_mapped(widget: Object) -> Bool;
    fn gtk_widget_paintable_new(widget: Object) -> Object;
    fn gdk_paintable_get_current_image(paintable: Object) -> Object;
    fn gdk_paintable_get_intrinsic_width(paintable: Object) -> c_int;
    fn gdk_paintable_get_intrinsic_height(paintable: Object) -> c_int;
    fn gdk_paintable_snapshot(paintable: Object, snapshot: Object, width: f64, height: f64);
    fn gtk_snapshot_new() -> Object;
    fn gtk_snapshot_to_paintable(snapshot: Object, size: *const PaintSize) -> Object;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_new(kind: usize, property: *const c_char, ...) -> Object;
    fn g_object_ref_sink(object: Object) -> Object;
    fn g_object_ref(object: Object) -> Object;
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_get_type() -> usize;
    fn webkit_network_session_new_ephemeral() -> Object;
    fn webkit_web_view_stop_loading(view: Object);
    fn webkit_web_view_is_loading(view: Object) -> Bool;
    fn webkit_web_view_reload_bypass_cache(view: Object);
    fn webkit_navigation_action_get_request(action: Object) -> Object;
    fn webkit_navigation_action_get_modifiers(action: Object) -> c_uint;
    fn webkit_navigation_action_get_mouse_button(action: Object) -> c_uint;
    fn webkit_uri_request_get_uri(request: Object) -> *const c_char;
}

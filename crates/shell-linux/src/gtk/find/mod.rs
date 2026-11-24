//! Routes the GTK find panel through the page's native find controller.

mod state;
mod view;

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_uint};

use super::{App, Widgets, connect, ffi, keys, palette};
use ffi::{Bool, Object};
use state::{Direction, MATCH_LIMIT, State};

pub(super) use view::View;

const CASE_INSENSITIVE_AND_WRAP: c_uint = 1 | (1 << 4);
const BACKWARDS: c_uint = 1 << 3;
const KEY_RETURN: c_uint = 0xff0d;
const KEY_KP_ENTER: c_uint = 0xff8d;
const LOAD_STARTED: c_int = 0;

#[derive(Default)]
pub(super) struct Find {
    open: Cell<bool>,
    state: RefCell<State>,
    geometry: Cell<Option<(palette::Allocation, c_int, c_int)>>,
    composing: Cell<bool>,
}

pub(super) fn attach(overlay: Object, web_view: Object, data: Object) -> View {
    let bar = View::new(overlay, controller_for(web_view));
    connect(overlay, c"get-child-position", layout as *const _, data);
    connect(bar.entry, c"changed", edited as *const _, data);
    connect(bar.entry, c"activate", activated as *const _, data);
    // SAFETY: GtkEntry owns its GtkText editable delegate for the lifetime of the entry.
    let text = unsafe { gtk_editable_get_delegate(bar.entry) };
    connect(text, c"preedit-changed", preedit_changed as *const _, data);
    connect(bar.previous, c"clicked", previous_clicked as *const _, data);
    connect(bar.next, c"clicked", next_clicked as *const _, data);
    connect(bar.close, c"clicked", close_clicked as *const _, data);
    let controller = keys::capture_controller();
    connect(controller, c"key-pressed", panel_key as *const _, data);
    // SAFETY: The live find panel adopts the new capture controller.
    unsafe { ffi::gtk_widget_add_controller(bar.panel, controller) };
    bar.set_count(None);
    bar
}

fn controller_for(web_view: Object) -> Object {
    // SAFETY: The live WebKit view owns its find controller.
    unsafe { webkit_web_view_get_find_controller(web_view) }
}

pub(super) fn observe(web_view: Object, data: Object) {
    let controller = controller_for(web_view);
    connect(web_view, c"load-changed", load_changed as *const _, data);
    connect(controller, c"found-text", found as *const _, data);
    connect(controller, c"failed-to-find-text", failed as *const _, data);
}

pub(super) fn rebind(app: &App, widgets: &mut Widgets, view: Object) {
    hide(app, widgets, false);
    widgets.find.controller = controller_for(view);
}

pub(super) fn key_pressed(app: &App, widgets: &Widgets, key: c_uint, modifiers: c_uint) -> bool {
    const SHIFT_CONTROL: c_uint = ffi::CONTROL | ffi::SHIFT;
    match (key, modifiers & keys::SHORTCUT_MODIFIERS) {
        (0x46 | 0x66, ffi::CONTROL) => show(app, widgets),
        (0x47 | 0x67, ffi::CONTROL | SHIFT_CONTROL) => {
            show_if_closed(app, widgets);
            let direction = if modifiers & ffi::SHIFT == 0 {
                Direction::Next
            } else {
                Direction::Previous
            };
            step(app, widgets.find, direction);
        }
        _ => return false,
    }
    true
}

fn show(app: &App, widgets: &Widgets) {
    let already_open = app.find.open.replace(true);
    widgets.find.show();
    if !already_open {
        search_from_entry(app, widgets.find);
    }
}

pub(super) fn show_query(app: &App, widgets: &Widgets, query: &str) {
    let Ok(query) = std::ffi::CString::new(query) else {
        return;
    };
    app.find.open.set(true);
    widgets.find.show();
    // SAFETY: This is the live find entry, which copies the verified citation text.
    unsafe { ffi::gtk_editable_set_text(widgets.find.entry, query.as_ptr()) };
    search_from_entry(app, widgets.find);
}

fn show_if_closed(app: &App, widgets: &Widgets) {
    if !app.find.open.get() {
        show(app, widgets);
    }
}

pub(super) fn hide(app: &App, widgets: &Widgets, restore_focus: bool) {
    if !app.find.open.replace(false) {
        return;
    }
    app.find.state.borrow_mut().set_query(None);
    app.find.geometry.set(None);
    app.find.composing.set(false);
    widgets.find.hide();
    // SAFETY: The live controller still belongs to this page.
    unsafe { webkit_find_controller_search_finish(widgets.find.controller) };
    if restore_focus {
        // SAFETY: The hidden find bar and live WebKit view share a window.
        unsafe { ffi::gtk_widget_grab_focus(widgets.view) };
    }
}

fn search_from_entry(app: &App, bar: View) {
    if !app.find.open.get() {
        return;
    }
    // SAFETY: This live GtkEntry implements GtkEditable.
    let text = unsafe { ffi::gtk_editable_get_text(bar.entry) };
    // SAFETY: GtkEditable returns a non-null NUL-terminated string, valid until edited.
    let text = unsafe { CStr::from_ptr(text) };
    let query = (!text.is_empty()).then(|| text.to_owned());
    app.find.state.borrow_mut().set_query(query);
    if text.is_empty() {
        // SAFETY: The live controller can drop all marks as soon as the query is cleared.
        unsafe { webkit_find_controller_search_finish(bar.controller) };
    }
    render_count(app, bar);
    dispatch(app, bar);
}

fn step(app: &App, bar: View, direction: Direction) {
    app.find.state.borrow_mut().queue_step(direction);
    dispatch(app, bar);
}

fn render_count(app: &App, bar: View) {
    let count = app.find.state.borrow().count();
    bar.set_count(count);
}

fn dispatch(app: &App, bar: View) {
    let Some(request) = app.find.state.borrow_mut().take_request() else {
        return;
    };
    if request.direction.is_none() {
        // SAFETY: The live controller can discard the highlights from the previous query.
        unsafe { webkit_find_controller_search_finish(bar.controller) };
    }
    let options = if request.direction == Some(Direction::Previous) {
        CASE_INSENSITIVE_AND_WRAP | BACKWARDS
    } else {
        CASE_INSENSITIVE_AND_WRAP
    };
    // Search with a direction finds relative to the selection and returns the marked total.
    // The step-only methods can report just the selected hit on packaged WebKitGTK versions.
    // SAFETY: The live controller copies the query before returning.
    unsafe {
        webkit_find_controller_search(bar.controller, request.query.as_ptr(), options, MATCH_LIMIT);
    }
}

fn answered(data: Object, controller: Object, matches: c_uint) {
    with_widgets(data, |app, widgets| {
        let bar = widgets.find;
        if bar.controller != controller {
            return;
        }
        app.find.state.borrow_mut().finish(matches);
        if app.find.open.get() {
            render_count(app, bar);
            dispatch(app, bar);
        }
    });
}

fn with_widgets(data: Object, action: impl FnOnce(&App, &Widgets)) {
    // SAFETY: Every caller is a GTK signal with the App pointer supplied by attach.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get() {
        action(app, &widgets);
    }
}

extern "C" fn layout(
    overlay: Object,
    child: Object,
    allocation: *mut palette::Allocation,
    data: Object,
) -> Bool {
    // SAFETY: This is the live App pointer registered for the GTK overlay's layout signal.
    let app = unsafe { &*data.cast::<App>() };
    let Some(widgets) = app.widgets.get() else {
        return ffi::FALSE;
    };
    let bar = widgets.find;
    if child != bar.panel || !app.find.open.get() {
        return ffi::FALSE;
    }
    // SAFETY: GtkOverlay provided a writable allocation for this child.
    let allocation = unsafe { &mut *allocation };
    let (width, height) = View::layout(overlay, allocation);
    let geometry = Some((*allocation, width, height));
    if app.find.geometry.replace(geometry) != geometry {
        bar.refresh_backdrop(widgets.view);
    }
    ffi::TRUE
}

extern "C" fn found(controller: Object, matches: c_uint, data: Object) {
    answered(data, controller, matches);
}

extern "C" fn failed(controller: Object, data: Object) {
    answered(data, controller, 0);
}

extern "C" fn load_changed(view: Object, event: c_int, data: Object) {
    if event == LOAD_STARTED {
        with_widgets(data, |app, widgets| {
            if widgets.view == view {
                hide(app, widgets, false);
            }
        });
    }
}

extern "C" fn edited(_: Object, data: Object) {
    with_widgets(data, |app, widgets| search_from_entry(app, widgets.find));
}

extern "C" fn activated(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        step(app, widgets.find, Direction::Next);
    });
}

extern "C" fn preedit_changed(_: Object, text: *const c_char, data: Object) {
    // SAFETY: GtkText supplies the preedit string for this signal invocation.
    let composing = !text.is_null() && !unsafe { CStr::from_ptr(text) }.is_empty();
    with_widgets(data, |app, _| app.find.composing.set(composing));
}

extern "C" fn previous_clicked(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        step(app, widgets.find, Direction::Previous);
    });
}

extern "C" fn next_clicked(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        step(app, widgets.find, Direction::Next);
    });
}

extern "C" fn close_clicked(_: Object, data: Object) {
    with_widgets(data, |app, widgets| hide(app, widgets, true));
}

extern "C" fn panel_key(
    _: Object,
    key: c_uint,
    _: c_uint,
    modifiers: c_uint,
    data: Object,
) -> Bool {
    // SAFETY: This is the live App pointer registered on the panel key controller.
    let app = unsafe { &*data.cast::<App>() };
    // GTK handles the key itself while the input method is composing.
    if app.find.composing.get() {
        return ffi::FALSE;
    }
    let Some(widgets) = app.widgets.get() else {
        return ffi::FALSE;
    };
    match (key, modifiers & keys::SHORTCUT_MODIFIERS) {
        (ffi::KEY_ESCAPE, 0) => hide(app, &widgets, true),
        (KEY_RETURN | KEY_KP_ENTER, ffi::SHIFT) => step(app, widgets.find, Direction::Previous),
        _ => return ffi::FALSE,
    }
    ffi::TRUE
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_editable_get_delegate(editable: Object) -> Object;
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_get_find_controller(view: Object) -> Object;
    fn webkit_find_controller_search(
        controller: Object,
        text: *const c_char,
        options: c_uint,
        max_matches: c_uint,
    );
    fn webkit_find_controller_search_finish(controller: Object);
}

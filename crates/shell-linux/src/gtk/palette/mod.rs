//! Opens the address overlay and palette and dispatches their native actions.

mod model;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_uint};

use quickie_domain_browsing::{Direction, JarChoice, PinnedSlot};
use quickie_domain_commands::{
    CommandId, ContextFlag, FocusedView, Key, KeyStroke, Modifiers, WhenContext,
};

use super::ffi::{self, Bool, Object};
use super::{App, Widgets, agent, browsing, connect, inspector, keys, privacy};
use model::{Command, Item, Mode, Model, Target, Visit};
pub(super) use view::{Allocation, View, panel_widgets};

#[derive(Default)]
pub(super) struct Palette {
    mode: Cell<Option<Mode>>,
    model: OnceCell<RefCell<Model>>,
    items: RefCell<Vec<Item>>,
    history: RefCell<Vec<Visit>>,
    geometry: Cell<Option<(view::Allocation, c_int, c_int)>>,
}

pub(super) fn attach(overlay: Object, data: Object) -> View {
    view::install_theme(overlay);
    let view = View::new(overlay);
    connect(overlay, c"get-child-position", layout as *const _, data);
    connect(view.entry, c"changed", input_changed as *const _, data);
    connect(view.entry, c"activate", entry_activated as *const _, data);
    connect(view.list, c"row-activated", row_activated as *const _, data);
    view.observe_focus(focus_left as *const _, data);
    view
}

pub(super) fn key_pressed(app: &App, widgets: &Widgets, key: c_uint, mods: c_uint) -> bool {
    match (key, mods & keys::SHORTCUT_MODIFIERS) {
        (ffi::KEY_ESCAPE, 0) if app.palette.mode.get().is_some() => hide(app, widgets, true),
        (0xff09, 0) if app.palette.mode.get() == Some(Mode::Address) => {
            return complete_address(app, widgets.palette);
        }
        (0xff52 | 0xff54, 0) if app.palette.mode.get().is_some() => {
            widgets
                .palette
                .step(key == 0xff54, app.palette.items.borrow().len());
        }
        _ => return false,
    }
    true
}

pub(super) fn key_context(app: &App) -> Option<WhenContext> {
    let mode = app.palette.mode.get()?;
    let (view, flag) = if mode == Mode::Address {
        (FocusedView::Address, ContextFlag::AddressOpen)
    } else {
        (FocusedView::Palette, ContextFlag::PaletteOpen)
    };
    Some(
        WhenContext::new()
            .in_view(view)
            .with(flag)
            .with(ContextFlag::InputFocused),
    )
}

fn show(app: &App, widgets: &Widgets, mode: Mode) {
    super::find::hide(app, widgets, false);
    browsing::dismiss_panels(app);
    privacy::close(app, widgets, false);
    app.palette.mode.set(Some(mode));
    if mode != Mode::Commands {
        app.palette.history.replace(visits(widgets.view));
    }
    // SAFETY: WebKit's URI belongs to the live view on its GTK thread.
    let uri = unsafe { ffi::webkit_web_view_get_uri(widgets.view) };
    // SAFETY: WebKit returns a nullable NUL-terminated URI for this live view.
    let location = unsafe { borrowed(uri) };
    let text = if mode == Mode::Address && location != c"about:blank" {
        location
    } else {
        c""
    };
    widgets.palette.show(mode, text);
    requery(app, widgets);
}

pub(super) fn open_address(app: &App, widgets: &Widgets) {
    show(app, widgets, Mode::Address);
}

pub(super) fn hide(app: &App, widgets: &Widgets, restore_page_focus: bool) {
    if app.palette.mode.replace(None).is_some() {
        app.palette.geometry.set(None);
        widgets.palette.hide();
        if restore_page_focus {
            // SAFETY: The browser remains a live focusable child of the same window.
            unsafe { ffi::gtk_widget_grab_focus(widgets.view) };
        }
    }
}

fn requery(app: &App, widgets: &Widgets) {
    let Some(mode) = app.palette.mode.get() else {
        return;
    };
    // SAFETY: The live entry implements GtkEditable.
    let input = unsafe { ffi::gtk_editable_get_text(widgets.palette.entry) };
    // SAFETY: The editable's text is borrowed until the next edit.
    let input = unsafe { borrowed(input) }.to_str().unwrap_or_default();
    // SAFETY: The live WebKit view is owned by the browser window.
    let current = unsafe { ffi::webkit_web_view_get_uri(widgets.view) };
    // SAFETY: The nullable URI is read synchronously before any navigation can occur.
    let current = unsafe { borrowed(current) }.to_str().unwrap_or_default();
    let tabs = if mode == Mode::Commands {
        Vec::new()
    } else {
        app.browsing.tabs()
    };
    let mut items = app
        .palette
        .model
        .get_or_init(RefCell::default)
        .borrow_mut()
        .items(mode, input, current, &app.palette.history.borrow(), &tabs);
    for item in &mut items {
        if let Target::Command(command) = item.target
            && command != Command::Updates
            && let Ok(id) = CommandId::try_from(command.id())
        {
            item.shortcut = app.keys.shortcut(&id);
        }
    }
    let can_complete = mode == Mode::Address && model::completion(&items, input).is_some();
    widgets
        .palette
        .render(&items, mode, input.trim().is_empty(), can_complete);
    app.palette.items.replace(items);
}

fn complete_address(app: &App, view: View) -> bool {
    // SAFETY: The entry is a live GtkEditable and its text is borrowed only for this lookup.
    let input = unsafe { ffi::gtk_editable_get_text(view.entry) };
    // SAFETY: GTK owns this NUL-terminated text until a subsequent edit.
    let input = unsafe { borrowed(input) };
    let address = model::completion(
        &app.palette.items.borrow(),
        input.to_str().unwrap_or_default(),
    )
    .and_then(|address| CString::new(address).ok());
    if let Some(address) = address {
        view.fill(&address);
        true
    } else {
        false
    }
}

fn activate(app: &App, widgets: &Widgets, index: usize, data: Object) {
    let target = app
        .palette
        .items
        .borrow()
        .get(index)
        .map(|item| item.target.clone());
    match target {
        Some(Target::Command(command)) => run_command(app, widgets, command, None, data),
        Some(Target::Tab(tab)) => {
            hide(app, widgets, true);
            browsing::activate(app, tab);
        }
        Some(Target::Ask(question)) => {
            hide(app, widgets, false);
            agent::ask(app, widgets, &question);
        }
        Some(Target::Navigate(uri) | Target::Visit(uri)) => {
            if let Ok(uri) = CString::new(uri) {
                hide(app, widgets, true);
                privacy::navigate(app, widgets.view, &uri);
            }
        }
        None => {}
    }
}

pub(in crate::gtk) fn has_command(id: &CommandId) -> bool {
    Command::of(id.as_str()).is_some() || pinned_command(id).is_some()
}

pub(super) fn dispatch(
    app: &App,
    widgets: &Widgets,
    id: &CommandId,
    stroke: KeyStroke,
    data: Object,
) -> bool {
    if let Some(command) = Command::of(id.as_str()) {
        run_command(app, widgets, command, Some(stroke), data);
    } else if let Some((slot, pin)) = pinned_command(id) {
        browsing::use_pinned_slot(app, slot, pin);
    } else {
        return false;
    }
    true
}

fn pinned_command(id: &CommandId) -> Option<(PinnedSlot, bool)> {
    let (number, pin) = if let Some(number) = id.as_str().strip_prefix("quickie.tab.pinToSlot") {
        (number, true)
    } else {
        (id.as_str().strip_prefix("quickie.tab.goToSlot")?, false)
    };
    let index = number.parse::<usize>().ok()?.checked_sub(1)?;
    Some((PinnedSlot::from_index(index)?, pin))
}

fn run_command(
    app: &App,
    widgets: &Widgets,
    command: Command,
    stroke: Option<KeyStroke>,
    data: Object,
) {
    match command {
        Command::Address => return show(app, widgets, Mode::Address),
        Command::GoTo => return show(app, widgets, Mode::GoTo),
        Command::OpenCommands => return show(app, widgets, Mode::Commands),
        _ => {}
    }
    hide(app, widgets, true);
    match command {
        Command::Address | Command::GoTo | Command::OpenCommands => {}
        // SAFETY: Navigation targets the live browser view on the GTK thread.
        Command::Back => unsafe { ffi::webkit_web_view_go_back(widgets.view) },
        // SAFETY: Navigation targets the live browser view on the GTK thread.
        Command::Forward => unsafe { ffi::webkit_web_view_go_forward(widgets.view) },
        // SAFETY: The current load belongs to the live browser view.
        Command::Reload => unsafe { ffi::webkit_web_view_reload(widgets.view) },
        // SAFETY: The current load belongs to the live browser view.
        Command::HardReload => unsafe { webkit_web_view_reload_bypass_cache(widgets.view) },
        Command::Inspector => inspector::toggle(widgets.view),
        Command::Find => super::find::show(app, widgets),
        Command::FindNext => super::find::find_match(app, widgets, false),
        Command::FindPrevious => super::find::find_match(app, widgets, true),
        Command::CopyUrl => {
            // SAFETY: This WebKit URI is borrowed from the live browser view.
            let uri = unsafe { ffi::webkit_web_view_get_uri(widgets.view) };
            if !uri.is_null() {
                // SAFETY: The live mapped view accompanies the display clipboard.
                let clipboard = unsafe { gtk_widget_get_clipboard(widgets.view) };
                // SAFETY: GDK copies the valid URI text synchronously.
                unsafe { gdk_clipboard_set_text(clipboard, uri) };
            }
        }
        Command::Updates => app.updates.present(data, &app.prefix),
        Command::Settings => browsing::settings::show(app, data),
        Command::NewTab => browsing::new_tab(app, JarChoice::Main),
        Command::NewThrowaway => browsing::new_tab(app, JarChoice::NewThrowaway),
        Command::CloseTab => browsing::close_active(app),
        Command::ReopenTab => browsing::reopen_closed(app),
        Command::Overview => browsing::show_overview(app),
        Command::NextTab | Command::PreviousTab => {
            let direction = if command == Command::NextTab {
                Direction::Forward
            } else {
                Direction::Backward
            };
            let held = stroke.is_some_and(|stroke| {
                stroke.key == Key::Tab && stroke.modifiers.contains(Modifiers::CONTROL)
            });
            browsing::cycle(app, direction, held);
        }
        Command::Agent => agent::open(app, widgets),
        Command::AgentSettings => agent::settings(app, widgets),
        Command::Shields => privacy::show_shields(app, widgets),
        Command::Downloads => browsing::downloads::toggle(app),
    }
}

extern "C" fn layout(
    overlay: Object,
    child: Object,
    allocation: *mut view::Allocation,
    data: Object,
) -> Bool {
    // SAFETY: GTK emits get-child-position with the live App reference from attach.
    let app = unsafe { &*data.cast::<App>() };
    let Some(widgets) = app.widgets.get() else {
        return ffi::FALSE;
    };
    if let Some(mode) = app.palette.mode.get()
        && child == widgets.palette.panel
    {
        // SAFETY: GTK supplies a writable allocation for this panel during the signal.
        let allocation = unsafe { &mut *allocation };
        let (width, height) = widgets.palette.layout(overlay, mode, allocation);
        let geometry = (*allocation, width, height);
        if app.palette.geometry.replace(Some(geometry)) != Some(geometry) {
            widgets.palette.refresh_backdrop(widgets.view);
        }
        return ffi::TRUE;
    }
    ffi::FALSE
}

extern "C" fn focus_left(_: Object, data: Object) {
    // SAFETY: The panel's live focus controller passes the App reference registered in attach.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get() {
        hide(app, &widgets, false);
    }
}

extern "C" fn input_changed(_: Object, data: Object) {
    // SAFETY: The live application owns the entry and passes back the App reference.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get() {
        requery(app, &widgets);
    }
}

extern "C" fn entry_activated(_: Object, data: Object) {
    // SAFETY: The live application owns the entry and passes back the App reference.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get()
        && let Some(index) = widgets.palette.selected()
    {
        activate(app, &widgets, index, data);
    }
}

extern "C" fn row_activated(_: Object, row: Object, data: Object) {
    // SAFETY: The live application owns the list and passes back the App reference.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get()
        && let Some(index) = view::index(row)
    {
        activate(app, &widgets, index, data);
    }
}

fn visits(view: Object) -> Vec<Visit> {
    const HISTORY_LIMIT: c_int = 12;
    // SAFETY: This WebKit view owns its history, and its items remain valid while being read.
    let history = unsafe { webkit_web_view_get_back_forward_list(view) };
    (-HISTORY_LIMIT..=HISTORY_LIMIT)
        .filter_map(|offset| {
            // SAFETY: The borrowed list contributes items without changing history.
            let item = unsafe { webkit_back_forward_list_get_nth_item(history, offset) };
            if item.is_null() {
                return None;
            }
            // SAFETY: The list owns the item and its URI for the duration of the lookup.
            let address = unsafe { webkit_back_forward_list_item_get_uri(item) };
            // SAFETY: The nullable URI is copied before the item can be removed.
            let address = unsafe { borrowed(address) }.to_str().ok()?;
            // SAFETY: The list owns the item and its title for the duration of the lookup.
            let title = unsafe { webkit_back_forward_list_item_get_title(item) };
            // SAFETY: The nullable title is copied before the item can be removed.
            let title = unsafe { borrowed(title) }.to_str().ok()?;
            let title = if title.is_empty() { address } else { title };
            Some(Visit {
                title: title.into(),
                address: address.into(),
            })
        })
        .collect()
}

unsafe fn borrowed<'a>(text: *const c_char) -> &'a CStr {
    if text.is_null() {
        c""
    } else {
        // SAFETY: Callers receive a NUL-terminated string from a live object and only borrow it
        // for the duration of their operation with that object.
        unsafe { CStr::from_ptr(text) }
    }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_get_clipboard(widget: Object) -> Object;
    fn gdk_clipboard_set_text(clipboard: Object, text: *const c_char);
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_reload_bypass_cache(view: Object);
    fn webkit_web_view_get_back_forward_list(view: Object) -> Object;
    fn webkit_back_forward_list_get_nth_item(list: Object, index: c_int) -> Object;
    fn webkit_back_forward_list_item_get_uri(item: Object) -> *const c_char;
    fn webkit_back_forward_list_item_get_title(item: Object) -> *const c_char;
}

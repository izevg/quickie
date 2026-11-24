//! The held Control-Tab switcher and the tab overview share native cards and filtering.

mod view;

use std::cell::{Cell, RefCell};
use std::ffi::c_uint;
use std::ptr;

use quickie_domain_browsing::{Direction, Tab, TabCycle, TabId};

use super::super::ffi::{self, Bool, Object};
use super::super::{App, connect, find, palette, privacy};
use super::activate;
use view::{Allocation, Card, View};

const REVEAL_MILLIS: c_uint = 151;
const TAB: c_uint = 0xff09;
const SHIFT_TAB: c_uint = 0xfe20;
const KEY_BACKSPACE: c_uint = 0xff08;
const KEY_UP: c_uint = 0xff52;
const KEY_DOWN: c_uint = 0xff54;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Switcher,
    Overview,
}

#[derive(Default)]
pub(super) struct Panels {
    view: Cell<Option<View>>,
    mode: Cell<Option<Mode>>,
    cycle: RefCell<Option<TabCycle>>,
    reveal: Cell<Option<c_uint>>,
    query: RefCell<String>,
    kept: RefCell<Vec<TabId>>,
    selected: Cell<Option<TabId>>,
    chosen: Cell<Option<TabId>>,
    geometry: Cell<Option<Allocation>>,
}

impl Panels {
    pub(super) fn attach(&self, window: Object, overlay: Object, data: Object) {
        let view = View::new(overlay, data);
        self.view.set(Some(view));
        connect(overlay, c"get-child-position", layout as *const _, data);
        connect(
            window,
            c"notify::is-active",
            focus_changed as *const _,
            data,
        );
        connect(view.entry, c"changed", query_changed as *const _, data);
        connect(view.entry, c"activate", confirmed as *const _, data);
        connect(
            view.cards,
            c"child-activated",
            card_clicked as *const _,
            data,
        );
    }

    pub(super) fn detach(&self) {
        if let Some(source) = self.reveal.take() {
            // SAFETY: The ID is cleared when the single main-thread callback runs.
            unsafe { ffi::g_source_remove(source) };
        }
    }

    pub(super) fn toggle_overview(&self, app: &App) {
        if self.mode.get() == Some(Mode::Overview) {
            self.dismiss(app, true);
            return;
        }
        self.dismiss(app, false);
        self.chosen.set(app.browsing.active_tab());
        self.show(app, Mode::Overview);
    }

    fn show(&self, app: &App, mode: Mode) {
        let Some(view) = self.view.get() else { return };
        if let Some(widgets) = app.widgets.get() {
            privacy::close(app, &widgets, false);
            find::hide(app, &widgets, false);
            palette::hide(app, &widgets, false);
        }
        let active = app.browsing.mounted.get();
        app.browsing.pages.remember_picture(active);
        self.mode.set(Some(mode));
        view.show(mode);
        self.update(app);
        view.refresh_backdrop(active);
    }

    pub(super) fn dismiss(&self, app: &App, focus: bool) {
        self.detach();
        self.cycle.take();
        self.query.take();
        self.kept.take();
        self.selected.set(None);
        self.chosen.set(None);
        if self.mode.replace(None).is_some()
            && let Some(view) = self.view.get()
        {
            self.geometry.set(None);
            view.hide();
            let active = app.browsing.mounted.get();
            if focus && !active.is_null() {
                // SAFETY: The active native view is owned by the current browser window.
                unsafe { ffi::gtk_widget_grab_focus(active) };
            }
        }
    }

    pub(super) fn commit_cycle(&self, app: &App) {
        let Some(cycle) = self.cycle.take() else {
            return;
        };
        self.dismiss(app, true);
        app.browsing.run(|state| state.commit_cycle(&cycle));
        app.browsing.settle(app);
    }

    fn step_cycle(&self, app: &App, direction: Direction) {
        let started = self
            .cycle
            .borrow_mut()
            .as_mut()
            .map(|cycle| cycle.step(direction))
            .is_some();
        if !started {
            self.begin_cycle(app, direction);
            return;
        }
        self.update(app);
    }

    fn begin_cycle(&self, app: &App, direction: Direction) {
        let Some(window) = app.browsing.window.get() else {
            return;
        };
        let cycle = app
            .browsing
            .state
            .borrow()
            .as_ref()
            .and_then(|state| state.begin_cycle(window, direction));
        if let Some(cycle) = cycle {
            self.dismiss(app, false);
            if let Some(widgets) = app.widgets.get() {
                privacy::close(app, &widgets, false);
                find::hide(app, &widgets, true);
                palette::hide(app, &widgets, true);
            }
            self.cycle.replace(Some(cycle));
            let data = ptr::from_ref(app).cast_mut().cast();
            // SAFETY: The registered timeout runs only on the GTK thread while the App is live.
            self.reveal.set(Some(unsafe {
                ffi::g_timeout_add(REVEAL_MILLIS, reveal, data)
            }));
        }
    }

    pub(super) fn key_pressed(&self, app: &App, key: c_uint, modifiers: c_uint) -> bool {
        const CONTROL_SHIFT: c_uint = ffi::CONTROL | ffi::SHIFT;
        if matches!(
            (key, modifiers),
            (TAB | SHIFT_TAB, ffi::CONTROL | CONTROL_SHIFT)
        ) {
            let direction = if modifiers & ffi::SHIFT == 0 {
                Direction::Forward
            } else {
                Direction::Backward
            };
            self.step_cycle(app, direction);
            return true;
        }
        if self.cycle.borrow().is_some() {
            if key == ffi::KEY_ESCAPE {
                self.dismiss(app, true);
                return true;
            }
            if modifiers & ffi::CONTROL == 0 {
                self.commit_cycle(app);
            }
        }
        match self.mode.get() {
            Some(Mode::Switcher) => self.switcher_key(app, key, modifiers),
            Some(Mode::Overview) => self.overview_key(app, key, modifiers),
            None => false,
        }
    }

    fn switcher_key(&self, app: &App, key: c_uint, modifiers: c_uint) -> bool {
        if modifiers & !(ffi::CONTROL | ffi::SHIFT) != 0 {
            return false;
        }
        match key {
            0x57 | 0x77 => {
                self.close_selection(app);
                return true;
            }
            KEY_BACKSPACE => {
                self.query.borrow_mut().pop();
            }
            _ => {
                // SAFETY: GDK accepts any scalar keyval and returns zero when it has no text.
                let Some(typed) = char::from_u32(unsafe { gdk_keyval_to_unicode(key) })
                    .filter(|typed| typed.is_alphanumeric() || *typed == ' ')
                else {
                    return false;
                };
                self.query.borrow_mut().push(typed);
            }
        }
        self.update(app);
        true
    }

    fn overview_key(&self, app: &App, key: c_uint, modifiers: c_uint) -> bool {
        if (key, modifiers) == (0x77, ffi::CONTROL) {
            self.close_selection(app);
            return true;
        }
        if modifiers != 0 {
            return false;
        }
        let columns = self
            .view
            .get()
            .map_or(1, View::columns)
            .min(self.kept.borrow().len().max(1));
        if key == ffi::KEY_ESCAPE {
            self.dismiss(app, true);
            return true;
        }
        if !matches!(key, ffi::KEY_LEFT | ffi::KEY_RIGHT | KEY_UP | KEY_DOWN) {
            return false;
        }
        let kept = self.kept.borrow();
        if let Some(last) = kept.len().checked_sub(1) {
            let index = kept
                .iter()
                .position(|&tab| Some(tab) == self.selected.get())
                .unwrap_or_default();
            let index = match key {
                ffi::KEY_LEFT => index.saturating_sub(1),
                ffi::KEY_RIGHT => index.saturating_add(1).min(last),
                KEY_UP => index.checked_sub(columns).unwrap_or(index),
                KEY_DOWN if index / columns < last / columns => {
                    index.saturating_add(columns).min(last)
                }
                _ => index,
            };
            let selected = kept.get(index).copied();
            self.selected.set(selected);
            self.chosen.set(selected);
            if let Some(view) = self.view.get() {
                view.select(index);
            }
        }
        true
    }

    fn close_selection(&self, app: &App) {
        if let Some(tab) = self.selected.get() {
            self.close_tab(app, tab);
        }
    }

    fn close_tab(&self, app: &App, tab: TabId) {
        if !app.browsing.close(app, tab) {
            return;
        }
        if let Some(cycle) = self.cycle.borrow_mut().as_mut() {
            cycle.remove(tab);
        }
        self.update(app);
        if let Some(view) = self.view.get() {
            view.focus(self.mode.get());
        }
    }

    fn update(&self, app: &App) {
        let (Some(mode), Some(view)) = (self.mode.get(), self.view.get()) else {
            return;
        };
        let state = app.browsing.state.borrow();
        let (Some(state), Some(window)) = (state.as_ref(), app.browsing.window.get()) else {
            return;
        };
        let session = state.session();
        let query = self.query.borrow();
        let mut cycle = self.cycle.borrow_mut();
        let tabs: Vec<&Tab> = if let Some(cycle) = cycle.as_ref() {
            cycle
                .order()
                .iter()
                .filter_map(|&tab| session.tab(tab))
                .collect()
        } else if let Ok(tabs) = session.tabs(window) {
            tabs.collect()
        } else {
            return;
        };
        let kept = filtered(&tabs, &query);
        let selected = if let Some(cycle) = cycle.as_mut() {
            cycle.narrow(&kept);
            cycle.selected()
        } else {
            self.chosen
                .get()
                .filter(|&chosen| kept.iter().any(|&index| tabs[index].id == chosen))
                .or_else(|| kept.first().map(|&index| tabs[index].id))
        };
        let shown = if mode == Mode::Switcher {
            visible_strip(&kept, &tabs, selected, view.capacity())
        } else {
            &kept
        };
        let cards = shown.iter().map(|&index| {
            let tab = tabs[index];
            Card {
                tab,
                slot: session.slot_of(tab.id),
                picture: app.browsing.pages.picture(tab.id),
            }
        });
        self.selected.set(selected);
        self.kept
            .replace(shown.iter().map(|&index| tabs[index].id).collect());
        view.render(mode, &query, tabs.len(), kept.len(), cards, selected);
    }
}

fn filtered(tabs: &[&Tab], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    tabs.iter()
        .enumerate()
        .filter_map(|(index, tab)| {
            let title = tab.title.to_lowercase();
            let host = tab.url.host_str().unwrap_or_default();
            words
                .iter()
                .all(|word| title.contains(word) || host.contains(word))
                .then_some(index)
        })
        .collect()
}

fn visible_strip<'a>(
    kept: &'a [usize],
    tabs: &[&Tab],
    selected: Option<TabId>,
    capacity: usize,
) -> &'a [usize] {
    let at = kept
        .iter()
        .position(|&index| Some(tabs[index].id) == selected)
        .unwrap_or_default();
    let first = at
        .saturating_sub(capacity / 2)
        .min(kept.len().saturating_sub(capacity));
    &kept[first..kept.len().min(first + capacity)]
}

extern "C" fn reveal(data: Object) -> Bool {
    // SAFETY: The timeout runs while the App that supplied data is live.
    let app = unsafe { &*data.cast::<App>() };
    let panels = &app.browsing.panels;
    panels.reveal.set(None);
    if panels.cycle.borrow().is_some() {
        panels.show(app, Mode::Switcher);
    }
    ffi::FALSE
}

extern "C" fn layout(
    overlay: Object,
    child: Object,
    allocation: *mut Allocation,
    data: Object,
) -> Bool {
    // SAFETY: The attached overlay supplies the same App reference throughout its lifetime.
    let app = unsafe { &*data.cast::<App>() };
    let panels = &app.browsing.panels;
    if let (Some(mode), Some(view)) = (panels.mode.get(), panels.view.get())
        && child == view.panel
    {
        // SAFETY: GTK supplies a writable allocation for this overlay child.
        let allocation = unsafe { &mut *allocation };
        view.layout(overlay, mode, allocation);
        if panels.geometry.replace(Some(*allocation)) != Some(*allocation) {
            view.refresh_backdrop(app.browsing.mounted.get());
        }
        return ffi::TRUE;
    }
    ffi::FALSE
}

extern "C" fn query_changed(entry: Object, data: Object) {
    // SAFETY: The attached entry supplies the same App reference throughout its lifetime.
    let app = unsafe { &*data.cast::<App>() };
    let panels = &app.browsing.panels;
    if panels.mode.get() == Some(Mode::Overview) {
        panels.query.replace(view::input(entry));
        panels.update(app);
    }
}

extern "C" fn confirmed(_: Object, data: Object) {
    // SAFETY: The attached entry supplies the same App reference throughout its lifetime.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(tab) = app.browsing.panels.selected.get() {
        open(app, tab);
    }
}

extern "C" fn card_clicked(_: Object, card: Object, data: Object) {
    // SAFETY: The attached list supplies the same App reference throughout its lifetime.
    let app = unsafe { &*data.cast::<App>() };
    let tab = card_tab(app, card);
    if let Some(tab) = tab {
        open(app, tab);
    }
}

extern "C" fn close_clicked(button: Object, data: Object) {
    // SAFETY: The attached button supplies the same App reference throughout its lifetime.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(tab) = card_tab(app, view::parent_card(button)) {
        app.browsing.panels.close_tab(app, tab);
    }
}

fn card_tab(app: &App, card: Object) -> Option<TabId> {
    let index = view::index(card)?;
    app.browsing.panels.kept.borrow().get(index).copied()
}

fn open(app: &App, tab: TabId) {
    app.browsing.panels.dismiss(app, true);
    activate(app, tab);
}

extern "C" fn outside_clicked(_: Object, _: i32, _: f64, _: f64, data: Object) {
    // SAFETY: The browser's gesture controller only runs with the registered App reference.
    let app = unsafe { &*data.cast::<App>() };
    app.browsing.panels.dismiss(app, true);
}

extern "C" fn focus_changed(window: Object, _: Object, data: Object) {
    // SAFETY: GTK emits notify for the live browser window on the main thread.
    if unsafe { gtk_window_is_active(window) } == ffi::FALSE {
        // SAFETY: The attached window supplies the same App reference during its lifetime.
        let app = unsafe { &*data.cast::<App>() };
        app.browsing.panels.dismiss(app, false);
    }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_window_is_active(window: Object) -> Bool;
    fn gdk_keyval_to_unicode(key: c_uint) -> u32;
}

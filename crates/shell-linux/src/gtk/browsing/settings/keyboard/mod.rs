//! The native Keyboard pane. The shell provides the same strokes and keymap as dispatch.

pub(in crate::gtk) mod file;
mod view;

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_uint, c_void};
use std::rc::Rc;

use quickie_application::{BindingLine, BindingSource};
use quickie_domain_commands::{Chord, CommandId, KeyStroke, Platform};

use crate::gtk::ffi::{self, Object};
use crate::gtk::keys::SHORTCUT_MODIFIERS;

const DOWN: c_uint = 0xff54;
const RETURN: c_uint = 0xff0d;
const KP_ENTER: c_uint = 0xff8d;
const BACKSPACE: c_uint = 0xff08;
const DELETE: c_uint = 0xffff;
const HINT: &str = "Enter or double-click records · Backspace resets · Ctrl+F searches";

pub(in crate::gtk) enum Action {
    Open,
    Reload,
    Bind(CommandId, Chord),
    Reset(CommandId),
}

pub(in crate::gtk) struct Hooks {
    pub dispatch: fn(Object, Action),
    pub read: fn(Object, c_uint, c_uint, c_uint) -> Option<KeyStroke>,
    pub check: fn(Object, &CommandId, Chord) -> Result<(), String>,
    pub available: fn(&CommandId) -> bool,
}

pub(in crate::gtk) struct Keyboard(Rc<Pane>);

struct Pane {
    view: view::View,
    hooks: Hooks,
    data: Object,
    model: RefCell<Model>,
    live: Cell<bool>,
    working: Cell<bool>,
}

#[derive(Default)]
struct Model {
    lines: Vec<BindingLine>,
    visible: Vec<usize>,
    recording: Option<Recording>,
}

struct Recording {
    position: usize,
    command: CommandId,
    title: &'static str,
    accepted: Option<Chord>,
}

impl Keyboard {
    pub(in crate::gtk) fn new(data: Object, hooks: Hooks) -> Self {
        let pane = Rc::new(Pane {
            view: view::View::new(),
            data,
            hooks,
            model: RefCell::default(),
            live: Cell::new(true),
            working: Cell::new(false),
        });
        for (widget, signal, callback) in [
            (pane.view.search, c"changed", search_changed as *const _),
            (pane.view.list, c"row-selected", selected as *const _),
            (pane.view.list, c"row-activated", activated as *const _),
            (pane.view.open, c"clicked", open as *const _),
            (pane.view.reload, c"clicked", reload as *const _),
        ] {
            bind(&pane, widget, signal, callback);
        }
        let focus = pane.view.list_focus();
        bind(&pane, focus, c"leave", focus_left as *const _);
        pane.view.hint(HINT, false);
        Self(pane)
    }

    pub(in crate::gtk) fn widget(&self) -> Object {
        self.0.view.root
    }

    pub(in crate::gtk) fn update(&self, lines: Vec<BindingLine>, file: &str, problems: &[String]) {
        let pane = &self.0;
        pane.view.problems(file, problems);
        let has_focus = pane.view.has_focus(pane.view.list);
        let selected = pane.selected_command();
        let mut model = pane.model.borrow_mut();
        if model.lines == lines {
            return;
        }
        model.lines = lines;
        model.recording = None;
        pane.render(&mut model);
        if has_focus {
            let position = model
                .visible
                .iter()
                .position(|&index| Some(&model.lines[index].command) == selected.as_ref());
            pane.view.focus_row(position.unwrap_or(0));
        }
    }

    pub(in crate::gtk) fn working(&self, active: bool) {
        self.0.working.set(active);
        self.0.view.working(active);
    }

    pub(in crate::gtk) fn status(&self, message: &str) {
        self.0
            .view
            .hint(if message.is_empty() { HINT } else { message }, false);
    }

    pub(in crate::gtk) fn status_label(&self) -> Object {
        self.0.view.hint
    }

    pub(in crate::gtk) fn error(&self, message: &str) {
        self.0.view.hint(message, true);
    }

    pub(in crate::gtk) fn stop_recording(&self) {
        self.0.stop_recording();
    }

    pub(in crate::gtk) fn key_pressed(
        &self,
        controller: Object,
        key: c_uint,
        hardware: c_uint,
        modifiers: c_uint,
    ) -> bool {
        let pane = &self.0;
        if !pane.view.mapped() {
            return false;
        }
        let shortcut_modifiers = modifiers & SHORTCUT_MODIFIERS;
        let list_focused = pane.view.has_focus(pane.view.list);
        if pane.model.borrow().recording.is_some() {
            if list_focused {
                pane.record(controller, key, hardware, modifiers);
                return true;
            }
            pane.stop_recording();
        }
        if matches!((key, shortcut_modifiers), (0x46 | 0x66, ffi::CONTROL)) {
            pane.view.focus_search();
            return true;
        }
        if shortcut_modifiers != 0 {
            return false;
        }
        if pane.view.has_focus(pane.view.search) && matches!(key, DOWN | RETURN | KP_ENTER) {
            pane.view.focus_row(0);
            return true;
        }
        if !list_focused || pane.working.get() {
            return false;
        }
        match (key, pane.view.selected()) {
            (RETURN | KP_ENTER, Some(position)) => pane.begin_recording(position),
            (BACKSPACE | DELETE, Some(position)) => pane.reset(position),
            _ => return false,
        }
        true
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        self.0.live.set(false);
    }
}

impl Model {
    fn line(&self, position: usize) -> Option<&BindingLine> {
        self.visible
            .get(position)
            .and_then(|&index| self.lines.get(index))
    }
}

impl Pane {
    fn render(&self, model: &mut Model) {
        let query = self.view.query();
        model.visible.clear();
        self.view.clear_rows();
        for (index, line) in model.lines.iter().enumerate() {
            if line.matches(&query) {
                self.view
                    .append_row(line, (self.hooks.available)(&line.command));
                model.visible.push(index);
            }
        }
        self.view.hint(HINT, false);
    }

    fn selected_command(&self) -> Option<CommandId> {
        let position = self.view.selected()?;
        self.model
            .borrow()
            .line(position)
            .map(|line| line.command.clone())
    }

    fn begin_recording(&self, position: usize) {
        if self.working.get() {
            return;
        }
        let mut model = self.model.borrow_mut();
        let Some(line) = model.line(position) else {
            return;
        };
        if !(self.hooks.available)(&line.command) {
            self.view
                .hint("This command is unavailable on Linux.", true);
            return;
        }
        let recording = Recording {
            position,
            title: line.title,
            command: line.command.clone(),
            accepted: None,
        };
        self.view.row_keys(position, "Type keys…", false);
        self.view.hint(
            &format!("Type the new keys for {} · Escape cancels", recording.title),
            false,
        );
        model.recording = Some(recording);
    }

    fn stop_recording(&self) {
        let mut model = self.model.borrow_mut();
        if self.restore_row(&mut model) {
            self.view.hint(HINT, false);
        }
    }

    fn restore_row(&self, model: &mut Model) -> bool {
        let Some(recording) = model.recording.take() else {
            return false;
        };
        if let Some(line) = model.line(recording.position) {
            self.view.row_keys(
                recording.position,
                line.keys.as_deref().unwrap_or("—"),
                false,
            );
        }
        true
    }

    fn reset(&self, position: usize) {
        let command = {
            let model = self.model.borrow();
            model.line(position).and_then(|selected| {
                let command = &selected.command;
                let changed = model
                    .lines
                    .iter()
                    .any(|line| line.command == *command && line.source == BindingSource::User);
                (changed && (self.hooks.available)(command)).then(|| command.clone())
            })
        };
        if let Some(command) = command {
            (self.hooks.dispatch)(self.data, Action::Reset(command));
        }
    }

    fn record(&self, controller: Object, key: c_uint, hardware: c_uint, modifiers: c_uint) {
        match (key, modifiers & SHORTCUT_MODIFIERS) {
            (ffi::KEY_ESCAPE, 0) => self.stop_recording(),
            (RETURN | KP_ENTER, 0) => {
                let action = {
                    let model = self.model.borrow();
                    model.recording.as_ref().and_then(|recording| {
                        Some(Action::Bind(recording.command.clone(), recording.accepted?))
                    })
                };
                if let Some(action) = action {
                    self.stop_recording();
                    (self.hooks.dispatch)(self.data, action);
                }
            }
            (0xffe1..=0xffee | 0xfe03 | 0xfe11, _) => {}
            _ => self.read_stroke(controller, key, hardware, modifiers),
        }
    }

    fn read_stroke(&self, controller: Object, key: c_uint, hardware: c_uint, modifiers: c_uint) {
        let mut model = self.model.borrow_mut();
        let Some(recording) = model.recording.as_mut() else {
            return;
        };
        recording.accepted = None;
        let Some(stroke) = (self.hooks.read)(controller, key, hardware, modifiers) else {
            self.view.row_keys(recording.position, "Unavailable", true);
            self.view.hint("Quickie cannot bind that key.", true);
            return;
        };
        let chord = Chord::from(stroke);
        let keys = chord.glyphs(Platform::Linux);
        match (self.hooks.check)(self.data, &recording.command, chord) {
            Ok(()) => {
                recording.accepted = Some(chord);
                self.view.row_keys(recording.position, &keys, false);
                self.view
                    .hint(&format!("{keys} · Enter saves · Escape cancels"), false);
            }
            Err(error) => {
                self.view.row_keys(recording.position, &keys, true);
                self.view.hint(&error, true);
            }
        }
    }
}

fn bind(pane: &Rc<Pane>, widget: Object, signal: &CStr, callback: *const c_void) {
    let data = Rc::into_raw(Rc::clone(pane)).cast_mut().cast();
    // SAFETY: This closure owns its Pane reference until GTK calls drop_signal.
    unsafe {
        ffi::g_signal_connect_data(
            widget,
            signal.as_ptr(),
            callback,
            data,
            Some(drop_signal),
            0,
        );
    };
}

unsafe extern "C" fn drop_signal(data: Object, _: Object) {
    // SAFETY: The matching signal handed GTK one owned Rc as its data.
    drop(unsafe { Rc::from_raw(data.cast::<Pane>()) });
}

extern "C" fn search_changed(_: Object, data: Object) {
    // SAFETY: The signal owns the Pane reference while this callback runs.
    let pane = unsafe { &*data.cast::<Pane>() };
    if pane.live.get() {
        let mut model = pane.model.borrow_mut();
        model.recording = None;
        pane.render(&mut model);
    }
}

extern "C" fn selected(_: Object, row: Object, data: Object) {
    // SAFETY: The signal owns the Pane reference while this callback runs.
    let pane = unsafe { &*data.cast::<Pane>() };
    if !pane.live.get() {
        return;
    }
    let Ok(mut model) = pane.model.try_borrow_mut() else {
        return;
    };
    let position = view::index(row);
    if model
        .recording
        .as_ref()
        .is_some_and(|recording| Some(recording.position) != position)
    {
        pane.restore_row(&mut model);
    }
    if model.recording.is_none() {
        let conflict = position
            .and_then(|position| model.line(position))
            .and_then(|line| line.conflict.as_ref());
        if let Some(command) = conflict {
            pane.view.hint(&format!("Already runs {command}"), true);
        } else {
            pane.view.hint(HINT, false);
        }
    }
}

extern "C" fn activated(_: Object, row: Object, data: Object) {
    // SAFETY: The signal owns the Pane reference while this callback runs.
    let pane = unsafe { &*data.cast::<Pane>() };
    if pane.live.get()
        && let Some(position) = view::index(row)
    {
        pane.begin_recording(position);
    }
}

extern "C" fn focus_left(_: Object, data: Object) {
    // SAFETY: The focus controller owns the Pane reference while this callback runs.
    let pane = unsafe { &*data.cast::<Pane>() };
    if pane.live.get()
        && let Ok(mut model) = pane.model.try_borrow_mut()
        && pane.restore_row(&mut model)
    {
        pane.view.hint(HINT, false);
    }
}

extern "C" fn open(_: Object, data: Object) {
    // SAFETY: The signal owns the Pane reference while this callback runs.
    let pane = unsafe { &*data.cast::<Pane>() };
    if pane.live.get() && !pane.working.get() {
        pane.stop_recording();
        (pane.hooks.dispatch)(pane.data, Action::Open);
    }
}

extern "C" fn reload(_: Object, data: Object) {
    // SAFETY: The signal owns the Pane reference while this callback runs.
    let pane = unsafe { &*data.cast::<Pane>() };
    if pane.live.get() && !pane.working.get() {
        pane.stop_recording();
        (pane.hooks.dispatch)(pane.data, Action::Reload);
    }
}

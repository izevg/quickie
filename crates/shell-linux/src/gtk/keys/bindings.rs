//! One GTK keymap for dispatch, the Settings editor, and command keycaps.

use std::cell::{Cell, RefCell};
use std::ffi::c_uint;
use std::time::{Duration, Instant};

use quickie_application::{Rejection, check_binding as verify_binding, keymap_lines};
use quickie_domain_commands::{
    BindingError, Catalog, Chord, CommandId, KeyStroke, Keymap, KeymapError, Platform, Resolution,
    WhenContext,
};

use crate::gtk::App;
use crate::gtk::browsing::settings::keyboard::file::{self, Outcome, State, Worker};
use crate::gtk::browsing::settings::keyboard::{Action, Keyboard};
use crate::gtk::browsing::settings::{keyboard_status, open_keybindings, refresh_keyboard};
use crate::gtk::ffi::{self, Bool, Object};

const CHORD_WAIT: Duration = Duration::from_secs(2);
const FILE_POLL_MILLIS: c_uint = 30;

pub(in crate::gtk) struct Keybindings {
    catalog: Catalog,
    state: RefCell<State>,
    worker: Option<Worker>,
    pending: Cell<Option<(KeyStroke, Instant)>>,
    file_poll: Cell<c_uint>,
}

impl Keybindings {
    pub(in crate::gtk) fn new(isolated: bool) -> Self {
        let catalog = Catalog::builtin();
        let (worker, state) = match file::profile_path().filter(|_| !isolated) {
            Some(path) => match Worker::start(path, catalog.clone()) {
                Ok((worker, state)) => (Some(worker), state),
                Err(error) => (None, State::defaults(Some(error))),
            },
            None => (None, State::defaults(None)),
        };
        Self {
            catalog,
            state: RefCell::new(state),
            worker,
            pending: Cell::new(None),
            file_poll: Cell::new(0),
        }
    }

    pub(in crate::gtk) fn render(&self, pane: &Keyboard) {
        let state = self.state.borrow();
        let path = self
            .worker
            .as_ref()
            .map_or_else(Default::default, |worker| worker.path().to_string_lossy());
        let problems = state.problems();
        pane.update(
            keymap_lines(&state.keymap, &self.catalog, &state.rejected),
            &path,
            &problems,
        );
    }

    pub(in crate::gtk) fn shortcut(&self, id: &CommandId) -> String {
        let state = self.state.borrow();
        state
            .keymap
            .bindings_for(id)
            .next()
            .map_or_else(String::new, |binding| binding.chord.glyphs(Platform::Linux))
    }

    pub(in crate::gtk) fn with_keymap<T>(&self, read: impl FnOnce(&Keymap) -> T) -> T {
        read(&self.state.borrow().keymap)
    }

    pub(in crate::gtk) fn resolve(
        &self,
        stroke: KeyStroke,
        context: &WhenContext,
    ) -> Option<Resolution> {
        self.resolve_matching(stroke, context, None)
    }

    pub(in crate::gtk) fn resolve_only(
        &self,
        stroke: KeyStroke,
        context: &WhenContext,
        command: &str,
    ) -> Option<Resolution> {
        self.resolve_matching(stroke, context, Some(command))
    }

    fn resolve_matching(
        &self,
        stroke: KeyStroke,
        context: &WhenContext,
        only: Option<&str>,
    ) -> Option<Resolution> {
        let now = Instant::now();
        let pending = self
            .pending
            .take()
            .filter(|&(_, since)| now.saturating_duration_since(since) <= CHORD_WAIT)
            .map(|(first, _)| first);
        if pending.is_none() && stroke.types_text() {
            return None;
        }
        let state = self.state.borrow();
        let resolution = state.keymap.resolve(stroke, pending, context);
        if let Resolution::Command(command) = &resolution
            && only.is_some_and(|allowed| command.as_str() != allowed)
        {
            return None;
        }
        if let Resolution::Pending(first) = resolution {
            if let Some(command) = only
                && !state.keymap.bindings().any(|binding| {
                    binding.command.as_str() == command
                        && binding.chord.first == first
                        && binding.chord.second.is_some()
                        && binding
                            .when
                            .as_ref()
                            .is_none_or(|when| when.evaluate(context))
                })
            {
                return None;
            }
            self.pending.set(Some((first, now)));
        }
        // A failed chord's second stroke is consumed as on AppKit.
        (resolution != Resolution::Unbound || pending.is_some()).then_some(resolution)
    }

    pub(in crate::gtk) fn cancel_chord(&self) {
        self.pending.set(None);
    }

    pub(in crate::gtk) fn detach(&self) {
        let source = self.file_poll.replace(0);
        if source != 0 {
            // SAFETY: The keybindings poll belongs to the live App and is removed once.
            unsafe { ffi::g_source_remove(source) };
        }
    }
}

pub(in crate::gtk) fn edit(data: Object, action: Action) {
    // SAFETY: The Settings pane passes its live App on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let keys = &app.keys;
    if keys.file_poll.get() != 0 {
        return;
    }
    let Some(worker) = keys.worker.as_ref() else {
        keyboard_status(
            app,
            false,
            "The keyboard shortcut file is unavailable.",
            true,
        );
        return;
    };
    if let Err(error) = worker.submit(action) {
        keyboard_status(app, false, &error, true);
        return;
    }
    keyboard_status(app, true, "Applying key bindings…", false);
    // SAFETY: The source is detached before the App leaves the GTK thread.
    keys.file_poll
        .set(unsafe { ffi::g_timeout_add(FILE_POLL_MILLIS, file_finished, data) });
}

pub(in crate::gtk) fn check_binding(
    data: Object,
    id: &CommandId,
    chord: Chord,
) -> Result<(), String> {
    // SAFETY: The Settings pane passes its live App on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let keys = &app.keys;
    let state = keys.state.borrow();
    verify_binding(&state.keymap, &keys.catalog, id, chord)
        .map_err(|error| refusal(&keys.catalog, chord, &error))
}

fn refusal(catalog: &Catalog, chord: Chord, error: &Rejection) -> String {
    let keys = chord.glyphs(Platform::Linux);
    match error {
        Rejection::Invalid(KeymapError::Conflict(conflict)) => {
            let other = catalog.get(&conflict.first).map_or_else(
                || conflict.first.to_string(),
                |command| command.title.to_owned(),
            );
            format!("{keys} already runs {other}")
        }
        Rejection::Malformed(BindingError::Typing(_)) => {
            format!("{keys} types text. Add Ctrl, Alt, or Super.")
        }
        other => other.to_string(),
    }
}

extern "C" fn file_finished(data: Object) -> Bool {
    // SAFETY: This source receives the live App and runs only on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let keys = &app.keys;
    let Some(outcome) = keys.worker.as_ref().and_then(Worker::outcome) else {
        return ffi::TRUE;
    };
    keys.file_poll.set(0);
    match outcome {
        Ok(Outcome::Loaded(state)) => {
            let unread = state.unread.clone();
            keys.state.replace(state);
            keys.cancel_chord();
            refresh_keyboard(app);
            if let Some(error) = unread {
                keyboard_status(
                    app,
                    false,
                    &format!("Could not read keybindings.json. {error}"),
                    true,
                );
            } else {
                keyboard_status(app, false, "Key bindings applied.", false);
            }
        }
        Ok(Outcome::Open(path)) => {
            keyboard_status(app, false, "", false);
            open_keybindings(app, &path);
        }
        Err(error) => keyboard_status(app, false, &error, true),
    }
    ffi::FALSE
}

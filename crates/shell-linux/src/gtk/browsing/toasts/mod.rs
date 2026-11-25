//! Native notification pills, with the most recent untagged toast closest to the page bottom.

mod view;

use std::cell::RefCell;
use std::ffi::{c_int, c_uint};
use std::rc::{Rc, Weak};

use crate::gtk::ffi::{self, Bool, Object};

const MAX_TOASTS: usize = 3;
const HOLD_MS: c_uint = 3_000;
const FADE_MS: c_uint = 120;
const PRIORITY_DEFAULT: c_int = 0;

#[derive(Clone, Copy)]
enum Kind {
    Info,
    Done,
    Success,
    Problem,
}

#[derive(Clone, Copy)]
pub(in crate::gtk) struct Toast<'a> {
    kind: Kind,
    text: &'a str,
    detail: Option<&'a str>,
    keys: Option<&'a str>,
}

impl<'a> Toast<'a> {
    const fn new(kind: Kind, text: &'a str) -> Self {
        Self {
            kind,
            text,
            detail: None,
            keys: None,
        }
    }

    pub(in crate::gtk) const fn info(text: &'a str) -> Self {
        Self::new(Kind::Info, text)
    }

    pub(in crate::gtk) const fn done(text: &'a str) -> Self {
        Self::new(Kind::Done, text)
    }

    pub(in crate::gtk) const fn success(text: &'a str) -> Self {
        Self::new(Kind::Success, text)
    }

    pub(in crate::gtk) const fn problem(text: &'a str) -> Self {
        Self::new(Kind::Problem, text)
    }

    pub(in crate::gtk) const fn with_detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub(in crate::gtk) const fn with_keys(mut self, keys: &'a str) -> Self {
        if !keys.is_empty() {
            self.keys = Some(keys);
        }
        self
    }
}

#[derive(Default)]
pub(super) struct Stack {
    state: Rc<RefCell<State>>,
}

#[derive(Default)]
struct State {
    view: Option<view::View>,
    entries: Vec<Entry>,
    next: u64,
}

struct Entry {
    id: u64,
    widget: Object,
    tag: Option<&'static str>,
    timer: Option<c_uint>,
}

#[derive(Clone, Copy)]
enum Phase {
    Holding,
    Fading,
}

struct Expiry {
    state: Weak<RefCell<State>>,
    id: u64,
    phase: Phase,
}

impl Stack {
    pub(super) fn attach(&self, overlay: Object) {
        let mut state = self.state.borrow_mut();
        state.clear();
        state.view = Some(view::View::new(overlay));
    }

    pub(in crate::gtk) fn show(&self, toast: Toast<'_>) {
        self.push(&toast, None);
    }

    pub(in crate::gtk) fn show_tagged(&self, toast: Toast<'_>, tag: &'static str) {
        self.push(&toast, Some(tag));
    }

    pub(in crate::gtk) fn clear(&self) {
        self.state.borrow_mut().clear();
    }

    fn push(&self, toast: &Toast<'_>, tag: Option<&'static str>) {
        let mut state = self.state.borrow_mut();
        let replaced = tag.and_then(|tag| {
            state
                .entries
                .iter()
                .position(|entry| entry.tag == Some(tag))
        });
        if let Some(index) = replaced {
            state.remove(index);
        } else if state.entries.len() == MAX_TOASTS {
            state.remove(0);
        }
        let index = replaced.unwrap_or(state.entries.len());
        let previous = index
            .checked_sub(1)
            .map(|previous| state.entries[previous].widget);
        let Some(view) = &state.view else {
            return;
        };
        let widget = view.insert(toast, previous);
        let id = state.next;
        state.next = id.wrapping_add(1);
        state.entries.insert(
            index,
            Entry {
                id,
                widget,
                tag,
                timer: Some(schedule(Rc::downgrade(&self.state), id, Phase::Holding)),
            },
        );
    }
}

impl State {
    fn remove(&mut self, index: usize) {
        let entry = self.entries.remove(index);
        if let Some(timer) = entry.timer {
            // SAFETY: The entry still owns a registered source on the GTK thread.
            unsafe { ffi::g_source_remove(timer) };
        }
        if let Some(view) = &self.view {
            view.remove(entry.widget);
        }
    }

    fn clear(&mut self) {
        while !self.entries.is_empty() {
            self.remove(self.entries.len() - 1);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.clear();
    }
}

fn schedule(state: Weak<RefCell<State>>, id: u64, phase: Phase) -> c_uint {
    let delay = match phase {
        Phase::Holding => HOLD_MS,
        Phase::Fading => FADE_MS,
    };
    let expiry = Box::into_raw(Box::new(Expiry { state, id, phase }));
    // SAFETY: The main context runs both callbacks using the boxed data until the source ends.
    unsafe {
        g_timeout_add_full(
            PRIORITY_DEFAULT,
            delay,
            timer_elapsed,
            expiry.cast(),
            timer_destroyed,
        )
    }
}

extern "C" fn timer_elapsed(data: Object) -> Bool {
    // SAFETY: The source retains its boxed data for the duration of the callback.
    let expiry = unsafe { &*data.cast::<Expiry>() };
    let Some(state) = expiry.state.upgrade() else {
        return ffi::FALSE;
    };
    let mut state = state.borrow_mut();
    let Some(index) = state.entries.iter().position(|entry| entry.id == expiry.id) else {
        return ffi::FALSE;
    };
    let entry = &mut state.entries[index];
    entry.timer = None;
    match expiry.phase {
        Phase::Holding => {
            view::reveal(entry.widget, false);
            entry.timer = Some(schedule(expiry.state.clone(), entry.id, Phase::Fading));
        }
        Phase::Fading => state.remove(index),
    }
    ffi::FALSE
}

extern "C" fn timer_destroyed(data: Object) {
    // SAFETY: GLib calls this once to return the Box transferred to the source.
    drop(unsafe { Box::from_raw(data.cast::<Expiry>()) });
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_timeout_add_full(
        priority: c_int,
        interval: c_uint,
        callback: extern "C" fn(Object) -> Bool,
        data: Object,
        destroy: extern "C" fn(Object),
    ) -> c_uint;
}

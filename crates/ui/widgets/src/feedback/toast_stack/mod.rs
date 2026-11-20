//! The toast stack: E1 pills centered at the bottom of the window, newest lowest. Each fades in
//! with `fade-fast`, stays `toast-hold` (3 s), and fades out; at most [`MAX_TOASTS`] show, the
//! oldest giving way. Toasts are E1, so they never dim the page and may show under an E2.
//! What a toast says is a [`Toast`]: a status symbol and text, a count, a detail, keys.

mod toast;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
use objc2_foundation::{NSPoint, NSTimer};
use quickie_ui_shared::tokens::space;
use quickie_ui_shared::{Pill, Trigger, motion};

pub use toast::{Toast, ToastKind};

/// How many toasts show at once.
pub const MAX_TOASTS: usize = 3;

struct Shown {
    id: u64,
    /// What a newer toast of the same tag replaces, such as the page zoom.
    tag: Option<&'static str>,
    text: String,
    pill: Pill,
    host: Retained<NSView>,
    leaving: bool,
}

pub struct ToastStack {
    toasts: RefCell<Vec<Shown>>,
    next_id: Cell<u64>,
    this: Weak<Self>,
    mtm: MainThreadMarker,
}

impl ToastStack {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            toasts: RefCell::new(Vec::new()),
            next_id: Cell::new(0),
            this: this.clone(),
            mtm,
        })
    }

    /// Shows `toast` at the bottom of `host` for `toast-hold`; plain text is an info toast.
    pub fn show(&self, host: &NSView, toast: impl Into<Toast>) {
        self.push(host, &toast.into(), None);
    }

    /// Shows `toast` in place of the toast of the same `tag`, which leaves at once: a value
    /// that changes key by key, such as the page zoom, shows once, never stacked.
    pub fn show_tagged(&self, host: &NSView, toast: impl Into<Toast>, tag: &'static str) {
        let replaced: Vec<u64> = self
            .toasts
            .borrow()
            .iter()
            .filter(|toast| toast.tag == Some(tag))
            .map(|toast| toast.id)
            .collect();
        for id in replaced {
            self.remove(id);
        }
        self.push(host, &toast.into(), Some(tag));
    }

    fn push(&self, host: &NSView, toast: &Toast, tag: Option<&'static str>) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let pill = Pill::with_parts(&toast.parts(), toast.tone(), self.mtm);
        pill.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxXMargin,
        );
        host.addSubview(pill.view());
        motion::show(pill.view(), motion::FADE_FAST, Trigger::App);
        let overflow = {
            let mut toasts = self.toasts.borrow_mut();
            toasts.push(Shown {
                id,
                tag,
                text: toast.text.clone(),
                pill,
                host: host.retain(),
                leaving: false,
            });
            let staying = toasts.iter().filter(|toast| !toast.leaving).count();
            let oldest: Vec<u64> = toasts
                .iter()
                .filter(|toast| !toast.leaving)
                .take(staying.saturating_sub(MAX_TOASTS))
                .map(|toast| toast.id)
                .collect();
            oldest
        };
        for old in overflow {
            self.dismiss(old);
        }
        self.layout();
        self.after(motion::TOAST_HOLD, move |stack| stack.dismiss(id));
    }

    /// Takes every toast away at once, as when a page goes full screen.
    pub fn clear(&self) {
        let shown: Vec<u64> = self.toasts.borrow().iter().map(|toast| toast.id).collect();
        for id in shown {
            self.remove(id);
        }
    }

    /// The texts on screen, oldest first, leaving ones excluded.
    #[must_use]
    pub fn texts(&self) -> Vec<String> {
        self.toasts
            .borrow()
            .iter()
            .filter(|toast| !toast.leaving)
            .map(|toast| toast.text.clone())
            .collect()
    }

    /// Fades a toast out, then removes it.
    fn dismiss(&self, id: u64) {
        {
            let mut toasts = self.toasts.borrow_mut();
            let Some(toast) = toasts
                .iter_mut()
                .find(|toast| toast.id == id && !toast.leaving)
            else {
                return;
            };
            toast.leaving = true;
            motion::hide(toast.pill.view(), motion::FADE_FAST, Trigger::App);
        }
        self.after(motion::FADE_FAST.duration, move |stack| stack.remove(id));
    }

    fn remove(&self, id: u64) {
        let removed = {
            let mut toasts = self.toasts.borrow_mut();
            toasts
                .iter()
                .position(|toast| toast.id == id)
                .map(|index| toasts.remove(index))
        };
        if let Some(toast) = removed {
            toast.pill.view().removeFromSuperview();
            self.layout();
        }
    }

    /// Stacks the toasts that stay upward from `space-xl` above the bottom, newest lowest.
    fn layout(&self) {
        let toasts = self.toasts.borrow();
        let mut y = space::XL;
        for toast in toasts.iter().rev().filter(|toast| !toast.leaving) {
            let size = toast.pill.size();
            let width = toast.host.bounds().size.width;
            toast
                .pill
                .place(NSPoint::new(((width - size.width) / 2.0).round(), y));
            y += size.height + space::SM;
        }
    }

    /// Runs `then` on the main run loop `seconds` from now, if the stack still exists.
    fn after(&self, seconds: f64, then: impl Fn(&Self) + 'static) {
        let this = self.this.clone();
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(stack) = this.upgrade() {
                then(&stack);
            }
        });
        // SAFETY: a one-shot timer on the main run loop, which retains it until it fires; the
        // block runs on the main thread that owns what it captures.
        let _timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(seconds, false, &block)
        };
    }
}

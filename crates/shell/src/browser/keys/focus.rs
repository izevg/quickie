//! What a key leaves behind, and what resets it when the user's attention moves.
//!
//! A chord's first stroke and a held `⌃⇥` cycle are state between key presses. When a window
//! loses the keyboard, the app goes to the background, or a window starts or ends a
//! full-screen transition, the keys that would finish them may never arrive here: the chord is
//! dropped and the cycle ends on its selection. When a browser window gets the keyboard back
//! and nothing of its own holds it (`AppKit` leaves the window itself first responder after
//! some transitions), its page takes it again, so typing and `inputFocused` follow the page.
//!
//! Also the key debug overlay: while it shows, each press is logged with what became of it.

use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplicationDidResignActiveNotification, NSView, NSWindow, NSWindowDidBecomeKeyNotification,
    NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification,
    NSWindowDidResignKeyNotification, NSWindowWillEnterFullScreenNotification,
    NSWindowWillExitFullScreenNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};

use super::super::Browser;
use crate::input::keystroke::log::{KeyLog, Outcome, Press};

/// Where the user's attention went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Moved {
    /// Away from the keys in flight: a window lost the keyboard, the app the foreground, or a
    /// full-screen transition began.
    Away,
    /// Away, as a window starts leaving full screen: a page that filled it leaves too.
    OutOfFullScreen,
    /// Back to a window: it got the keyboard, or a full-screen transition ended.
    Back,
}

/// Observes the focus changes that reset key state, for as long as the result lives.
pub(super) fn observe(
    browser: &Rc<Browser>,
) -> Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>> {
    // SAFETY: immutable string constants AppKit defines and never changes.
    let names: [(&NSNotificationName, Moved); 7] = unsafe {
        [
            (NSWindowDidResignKeyNotification, Moved::Away),
            (NSApplicationDidResignActiveNotification, Moved::Away),
            (NSWindowWillEnterFullScreenNotification, Moved::Away),
            (
                NSWindowWillExitFullScreenNotification,
                Moved::OutOfFullScreen,
            ),
            (NSWindowDidBecomeKeyNotification, Moved::Back),
            (NSWindowDidEnterFullScreenNotification, Moved::Back),
            (NSWindowDidExitFullScreenNotification, Moved::Back),
        ]
    };
    let center = NSNotificationCenter::defaultCenter();
    names
        .into_iter()
        .map(|(name, moved)| {
            let this = Rc::downgrade(browser);
            let block = RcBlock::new(move |note: NonNull<NSNotification>| {
                // SAFETY: the center passes a valid notification that outlives this call.
                let note = unsafe { note.as_ref() };
                let window = note
                    .object()
                    .and_then(|object| object.downcast::<NSWindow>().ok());
                if let Some(browser) = this.upgrade() {
                    browser.attention_moved(moved, window.as_deref());
                }
            });
            // SAFETY: no object filter and no queue, so the block runs on the main thread that
            // posts these notifications, where the browser it captures lives.
            unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            }
        })
        .collect()
}

impl Browser {
    fn attention_moved(&self, moved: Moved, window: Option<&NSWindow>) {
        self.pending.take();
        self.commit_cycle();
        let Some(window) = window else {
            return;
        };
        match moved {
            Moved::Away => return,
            Moved::OutOfFullScreen => {
                self.window_leaves_fullscreen(window);
                self.drain();
                return;
            }
            Moved::Back => {}
        }
        let id = self
            .windows
            .borrow()
            .iter()
            .find(|(_, own)| ***own == *window)
            .map(|(&id, _)| id);
        let held_by_a_view = window
            .firstResponder()
            .is_some_and(|responder| responder.downcast::<NSView>().is_ok());
        if let Some(id) = id
            && !held_by_a_view
        {
            self.settle(id);
        }
    }

    /// "Show Key Debug Overlay": shows the last key presses over the current window, or takes
    /// them away. Presses are kept only while it shows.
    pub(in crate::browser) fn toggle_key_log(&self) {
        if self.key_log.borrow_mut().take().is_some() {
            self.key_log_panel.hide();
            return;
        }
        let Some(host) = self.current().and_then(|(_, window)| window.contentView()) else {
            return;
        };
        *self.key_log.borrow_mut() = Some(KeyLog::default());
        self.key_log_panel.show(&host, &[]);
    }

    /// Logs a press while the overlay shows; `press` describes it only then.
    pub(super) fn log_key(&self, press: impl FnOnce() -> Press, outcome: &Outcome) {
        let lines = {
            let mut log = self.key_log.borrow_mut();
            let Some(log) = log.as_mut() else {
                return;
            };
            log.push(press(), outcome.clone());
            log.lines(self.keymap.borrow().platform())
        };
        self.key_log_panel.update(&lines);
    }
}

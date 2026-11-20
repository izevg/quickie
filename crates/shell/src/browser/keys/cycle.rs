//! The `⌃⇥` hold: the switcher appears only after the hold passes the threshold, and releasing
//! the modifier (`flagsChanged`) commits the cycle. While it shows, typing filters it and `W`
//! closes the selected tab. A release the monitor never hears (it
//! happened in another app, or during a space or full-screen switch) is caught by the next key
//! press without the modifier, or by the window losing the keyboard ([`super::focus`]).

use std::ptr::NonNull;
use std::time::Instant;

use block2::RcBlock;
use objc2_app_kit::NSEvent;
use objc2_foundation::NSTimer;
use quickie_domain_browsing::{Direction, WindowId};
use quickie_domain_commands::{Key, KeyStroke, Modifiers};

use super::super::Browser;
use super::modifiers;
use crate::input::cycle::CycleHold;

impl Browser {
    /// Releasing a modifier that held a cycle open ends it on its selection.
    pub(super) fn flags_changed(&self, event: &NSEvent) {
        if !modifiers(event.modifierFlags()).contains(self.cycle_modifiers.get()) {
            self.commit_cycle();
        }
    }

    /// `⌃⇥`: begins a cycle held open by `held`, or steps the one in progress. Nothing shows
    /// until the hold passes the threshold, so a quick tap switches with no UI at all. With
    /// nothing held (the palette, or a key bound without modifiers) the cycle commits at once.
    pub(in crate::browser) fn cycle(
        &self,
        window: WindowId,
        direction: Direction,
        held: Modifiers,
    ) {
        if let Some(hold) = self.cycle.borrow_mut().as_mut() {
            hold.step(direction);
            if hold.revealed() {
                self.switcher.select(hold.selected_index());
            }
            return;
        }
        let Some(cycle) = self.browsing.borrow().begin_cycle(window, direction) else {
            return;
        };
        self.address.hide();
        self.command_palette.hide();
        self.hide_popovers();
        self.hide_find();
        *self.cycle.borrow_mut() = Some(CycleHold::new(cycle, window, Instant::now()));
        if held == Modifiers::NONE {
            self.commit_cycle();
            return;
        }
        self.cycle_modifiers.set(held);
        self.schedule_reveal();
    }

    fn schedule_reveal(&self) {
        let this = self.this.clone();
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(browser) = this.upgrade() {
                browser.reveal();
            }
        });
        // SAFETY: a one-shot timer on the main run loop, which retains it until it fires; the
        // block runs on the main thread that owns what it captures.
        let _timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                CycleHold::reveal_delay().as_secs_f64(),
                false,
                &block,
            )
        };
    }

    /// The hold passed the threshold: the switcher appears, with the pictures the engine
    /// keeps; new ones of the page on screen and of pages never drawn follow.
    fn reveal(&self) {
        let (window, order, selected) = {
            let mut cycle = self.cycle.borrow_mut();
            let Some(hold) = cycle.as_mut() else {
                return;
            };
            if !hold.reveal(Instant::now()) {
                return;
            }
            let order = hold.cycle().order().to_vec();
            (hold.window(), order, hold.selected_index())
        };
        let Some(host) = self.window(window).and_then(|w| w.contentView()) else {
            return;
        };
        let cards = self.tab_cards(&order);
        self.switcher.show(&host, cards, selected);
        self.refresh_thumbnails(window, &order);
    }

    /// A key while `⌃` holds a revealed switcher open: `W` closes the selected tab, `⌫` takes
    /// a typed character back, and other letters, digits and the space filter the cards.
    /// Letters are read by their key, so any keyboard layout filters in Latin letters.
    pub(super) fn switcher_key(&self, stroke: KeyStroke) -> bool {
        if !self
            .cycle
            .borrow()
            .as_ref()
            .is_some_and(CycleHold::revealed)
        {
            return false;
        }
        let kept = match stroke.key {
            Key::Letter(letter) if letter.as_char() == 'w' => {
                self.close_from_switcher();
                return true;
            }
            Key::Letter(letter) => self.switcher.type_text(&letter.as_char().to_string()),
            Key::Digit(digit) => self.switcher.type_text(&digit.as_char().to_string()),
            Key::Space => self.switcher.type_text(" "),
            Key::Backspace => self.switcher.erase(),
            _ => return false,
        };
        let selected = self.cycle.borrow_mut().as_mut().map(|hold| {
            hold.narrow(&kept);
            hold.selected_index()
        });
        self.switcher.select(selected.flatten());
        true
    }

    /// `W` in the switcher: the selected tab goes to the trash and leaves the cycle; the
    /// switcher stays, on the next tab. A cycle left without tabs ends.
    fn close_from_switcher(&self) {
        let Some((window, tab)) = self
            .cycle
            .borrow()
            .as_ref()
            .and_then(|hold| Some((hold.window(), hold.cycle().selected()?)))
        else {
            return;
        };
        self.close_in(window, tab);
        let order = self.cycle.borrow_mut().as_mut().map(|hold| {
            hold.remove(tab);
            hold.cycle().order().to_vec()
        });
        let Some(order) = order.filter(|order| !order.is_empty()) else {
            self.cancel_cycle();
            self.settle(window);
            return;
        };
        let cards = self.tab_cards(&order);
        let kept = self.switcher.set_tabs(cards, None);
        let selected = self.cycle.borrow_mut().as_mut().map(|hold| {
            hold.narrow(&kept);
            hold.selected_index()
        });
        self.switcher.select(selected.flatten());
    }

    /// Ends a cycle on its selection; nothing when none runs, or when the filter kept no tab.
    pub(in crate::browser) fn commit_cycle(&self) {
        let Some(hold) = self.cycle.borrow_mut().take() else {
            return;
        };
        self.switcher.hide();
        let window = self
            .run(|browsing| browsing.commit_cycle(hold.cycle()))
            .flatten();
        self.settle(window.unwrap_or(hold.window()));
    }

    pub(in crate::browser) fn cancel_cycle(&self) {
        if self.cycle.borrow_mut().take().is_some() {
            self.switcher.hide();
        }
    }
}

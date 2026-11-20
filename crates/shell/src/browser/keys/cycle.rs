//! The `⌃⇥` hold: the switcher appears only after the hold passes the threshold, and releasing
//! the modifier (`flagsChanged`) commits the cycle. A release the monitor never hears (it
//! happened in another app, or during a space or full-screen switch) is caught by the next key
//! press without the modifier, or by the window losing the keyboard ([`super::focus`]).

use std::ptr::NonNull;
use std::time::Instant;

use block2::RcBlock;
use objc2_app_kit::NSEvent;
use objc2_foundation::NSTimer;
use quickie_domain_browsing::{Direction, WindowId};
use quickie_domain_commands::Modifiers;

use super::super::{Browser, label};
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

    /// The hold passed the threshold: the switcher appears.
    fn reveal(&self) {
        let mut cycle = self.cycle.borrow_mut();
        let Some(hold) = cycle.as_mut() else {
            return;
        };
        if !hold.reveal(Instant::now()) {
            return;
        }
        let titles = {
            let browsing = self.browsing.borrow();
            let order = hold.cycle().order();
            order
                .iter()
                .map(|&tab| label(browsing.session(), tab))
                .collect()
        };
        if let Some(host) = self.window(hold.window()).and_then(|w| w.contentView()) {
            self.switcher.show(&host, titles, hold.selected_index());
        }
    }

    /// Ends a cycle on its selection; nothing when none runs.
    pub(in crate::browser) fn commit_cycle(&self) {
        let Some(hold) = self.cycle.borrow_mut().take() else {
            return;
        };
        self.switcher.hide();
        if let Some(window) = self.run(|browsing| browsing.commit_cycle(hold.cycle())) {
            self.settle(window);
        } else {
            self.settle(hold.window());
        }
    }

    pub(in crate::browser) fn cancel_cycle(&self) {
        if self.cycle.borrow_mut().take().is_some() {
            self.switcher.hide();
        }
    }
}

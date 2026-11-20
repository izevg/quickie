//! Keys before the page sees them. A key press becomes a [`KeyStroke`] by its key code, so it
//! means the same on every layout; an open overlay takes its fixed keys first; then the keymap
//! resolves the stroke in the current [`WhenContext`] to a command, or to the first half of a
//! chord, or to nothing, and only then does the page or the focused field get it.
//!
//! Also the `⌃⇥` hold: the switcher appears only after the hold passes the threshold, and
//! releasing the modifier (`flagsChanged`) commits the cycle.

use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
use objc2_foundation::NSTimer;
use quickie_application::Navigation;
use quickie_domain_browsing::{Direction, WindowId};
use quickie_domain_commands::{Key, KeyStroke, Modifiers, Resolution, WhenContext};
use url::Url;

use super::commands::Action;
use super::page::SheetRoute;
use super::{Browser, label};
use crate::input::cycle::CycleHold;
use crate::input::destination::destination;
use crate::input::keystroke::{OverlayKey, is_quit, overlay_key, stroke};

impl Browser {
    /// Handles a key press if it is the browser's; the page never sees it then.
    fn key_down(&self, event: &NSEvent) -> bool {
        // A system panel over a window, such as the print panel, takes its own keys and types.
        let elsewhere = event
            .window(self.mtm)
            .is_some_and(|window| !self.windows.borrow().values().any(|own| **own == *window));
        if elsewhere {
            return false;
        }
        let handled =
            if let Some(stroke) = stroke(event.keyCode(), modifiers(event.modifierFlags())) {
                self.route(stroke)
            } else {
                self.pending.set(None);
                // A key the keymap cannot name (the keypad, a media key) is the sheet's field's,
                // or nobody's while a sheet waits: never the page's under it.
                self.sheet_takes_unnamed()
            };
        self.drain();
        handled
    }

    pub(super) fn route(&self, stroke: KeyStroke) -> bool {
        if self.cycle.borrow().is_some() && stroke.key == Key::Escape {
            self.cancel_cycle();
            return true;
        }
        if let Some(route) = self.sheet_route(stroke) {
            return self.past_sheet(stroke, route);
        }
        if self.navigation_key(stroke) {
            return true;
        }
        if let Some(key) = overlay_key(stroke)
            && self.overlay_key(key)
        {
            return true;
        }
        let pending = self.pending.take();
        match self.keymap.resolve(stroke, pending, &self.when()) {
            Resolution::Command(id) => {
                self.dispatch(&id, Some(stroke));
                true
            }
            Resolution::Pending(first) => {
                self.pending.set(Some(first));
                true
            }
            Resolution::Unbound => {
                if let Some(first) = pending {
                    let platform = self.keymap.platform();
                    self.toast(&format!(
                        "{} {} is not bound",
                        first.glyphs(platform),
                        stroke.glyphs(platform)
                    ));
                    true
                } else if is_quit(stroke) {
                    NSApplication::sharedApplication(self.mtm).terminate(None);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// A key while a sheet waits: the sheet's own keys are taken, its field types, and of the
    /// shortcuts only those that leave the page (closing or switching tabs and windows) and
    /// `⌘Q` run; the page under the sheet never gets a key.
    fn past_sheet(&self, stroke: KeyStroke, route: SheetRoute) -> bool {
        match route {
            SheetRoute::Taken => true,
            SheetRoute::Field => false,
            SheetRoute::Keymap => {
                let pending = self.pending.take();
                match self.keymap.resolve(stroke, pending, &self.when()) {
                    Resolution::Command(id)
                        if Action::of(&id).is_some_and(Action::passes_sheet) =>
                    {
                        self.dispatch(&id, Some(stroke));
                    }
                    Resolution::Pending(first) => self.pending.set(Some(first)),
                    _ if is_quit(stroke) => {
                        NSApplication::sharedApplication(self.mtm).terminate(None);
                    }
                    _ => {}
                }
                true
            }
        }
    }

    /// What is true right now, for `when` clauses.
    ///
    /// `inputFocused` is the page's word: its focus script reports whether the focused element
    /// takes text, so `F` types into a form and opens link hints everywhere else. An overlay's
    /// own field, the find bar's included, is focused input too.
    pub(super) fn when(&self) -> WhenContext {
        let mut context = WhenContext::new();
        let view = if self.command_palette.is_open() {
            context = context.with_flag("paletteOpen").with_flag("inputFocused");
            "palette"
        } else if self.address.is_open() {
            context = context.with_flag("addressOpen").with_flag("inputFocused");
            "address"
        } else if self.navigation.overview_open() {
            context = context.with_flag("overviewOpen").with_flag("inputFocused");
            "overview"
        } else if self.agent_ui.settings_open() {
            context = context
                .with_flag("agentSettingsOpen")
                .with_flag("inputFocused");
            "agentSettings"
        } else {
            context = context.with_flag("pageFocused");
            if self.find_focused() || self.page_input_focused() {
                context = context.with_flag("inputFocused");
            }
            "page"
        };
        if self.navigation.hints_open() {
            context = context.with_flag("hintsOpen");
        }
        if self.cycle.borrow().is_some() {
            context = context.with_flag("switcherOpen");
        }
        if self.popovers.is_open() {
            context = context.with_flag("popoverOpen");
        }
        if self.agent_ui.is_open() {
            context = context.with_flag("agentOpen");
        }
        context.with_value("focusedView", view)
    }

    /// True when the current window's page says a text field of its own has focus.
    fn page_input_focused(&self) -> bool {
        let Some(tab) = self.current().and_then(|(window, _)| self.active(window)) else {
            return false;
        };
        self.browsing
            .try_borrow_mut()
            .is_ok_and(|mut browsing| browsing.engine_mut().input_focused(tab))
    }

    /// An overlay's fixed key. While an input method composes, every key is the field's.
    fn overlay_key(&self, key: OverlayKey) -> bool {
        if self.agent_ui.is_open() {
            return self.agent_key(key);
        }
        if self.popovers.is_open() {
            return self.popover_key(key);
        }
        if self.command_palette.is_open() {
            if self.command_palette.is_composing() {
                return false;
            }
            self.palette_key(key);
            return true;
        }
        if !self.address.is_open() {
            return self.find_key(key);
        }
        if self.address.is_composing() {
            return false;
        }
        match key {
            OverlayKey::Move(_) | OverlayKey::Previous => return false,
            OverlayKey::Accept { new_tab } => self.submit_address(new_tab),
            OverlayKey::Dismiss => self.close_address(),
        }
        true
    }

    /// Releasing a modifier that held a cycle open ends it on its selection.
    fn flags_changed(&self, event: &NSEvent) {
        if !modifiers(event.modifierFlags()).contains(self.cycle_modifiers.get()) {
            self.commit_cycle();
        }
    }

    pub(super) fn open_address(&self) {
        let Some((window, ns_window)) = self.current() else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        self.command_palette.hide();
        self.hide_popovers();
        self.cancel_cycle();
        self.hide_find();
        let url = self.active(window).and_then(|tab| {
            let browsing = self.browsing.borrow();
            browsing.session().tab(tab).map(|tab| tab.url.clone())
        });
        let url = url.filter(|url| *url != self.home);
        self.address
            .show(&host, &ns_window, url.as_ref().map_or("", Url::as_str));
    }

    /// `⏎` in the address overlay: go there in this tab; `⌘⏎`: in a new one.
    fn submit_address(&self, new_tab: bool) {
        let target = destination(&self.address.text()).and_then(|url| Url::parse(&url).ok());
        self.close_address();
        if let Some(url) = target {
            if new_tab {
                self.open_in_new_tab(url);
            } else if let Some((window, _)) = self.current() {
                self.navigate(window, &Navigation::Load(url));
            }
        }
    }

    fn close_address(&self) {
        self.address.hide();
        if let Some((window, _)) = self.current() {
            self.settle(window);
        }
    }

    /// `⌃⇥`: begins a cycle held open by `held`, or steps the one in progress. Nothing shows
    /// until the hold passes the threshold, so a quick tap switches with no UI at all. With
    /// nothing held (the palette, or a key bound without modifiers) the cycle commits at once.
    pub(super) fn cycle(&self, window: WindowId, direction: Direction, held: Modifiers) {
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

    fn commit_cycle(&self) {
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

    pub(super) fn cancel_cycle(&self) {
        if self.cycle.borrow_mut().take().is_some() {
            self.switcher.hide();
        }
    }
}

/// Routes key presses and modifier changes to the browser before the window dispatches them to
/// the page or a field, and pinches to the tab overview; a click or a scroll takes link hints
/// away. The returned monitor must stay alive for as long as shortcuts should work.
pub fn install_shortcuts(browser: &Rc<Browser>) -> Option<Retained<AnyObject>> {
    let browser = Rc::clone(browser);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid event that outlives this call.
        let key = unsafe { event.as_ref() };
        match key.r#type() {
            NSEventType::KeyDown if browser.key_down(key) => std::ptr::null_mut(),
            NSEventType::Magnify if browser.magnify(key) => std::ptr::null_mut(),
            NSEventType::FlagsChanged => {
                browser.flags_changed(key);
                event.as_ptr()
            }
            NSEventType::LeftMouseDown | NSEventType::ScrollWheel => {
                browser.pointer();
                event.as_ptr()
            }
            _ => event.as_ptr(),
        }
    });
    let mask = NSEventMask::KeyDown
        | NSEventMask::FlagsChanged
        | NSEventMask::Magnify
        | NSEventMask::LeftMouseDown
        | NSEventMask::ScrollWheel;
    // SAFETY: the handler returns either the event it was given or null, as the API requires,
    // and AppKit calls it on the main thread, where the captured browser lives.
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }
}

/// The modifiers the keymap reads; Caps Lock and Fn do not count.
fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    [
        (NSEventModifierFlags::Command, Modifiers::COMMAND),
        (NSEventModifierFlags::Shift, Modifiers::SHIFT),
        (NSEventModifierFlags::Option, Modifiers::OPTION),
        (NSEventModifierFlags::Control, Modifiers::CONTROL),
    ]
    .into_iter()
    .filter(|&(flag, _)| flags.contains(flag))
    .fold(Modifiers::NONE, |held, (_, modifier)| held | modifier)
}

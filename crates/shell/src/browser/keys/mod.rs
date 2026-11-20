//! Keys before anything else sees them. The app's local event monitor gets every key press
//! first, before the menu bar's key equivalents, the first responder (the page, a field, an
//! overlay), and `AppKit`'s own handling; what it returns unhandled goes on in that order.
//!
//! A press is read by [`read`]: held with `fn`/Globe it is the system's and passes untouched;
//! otherwise it becomes a [`KeyStroke`] (a Latin letter as the layout names it, any other key by
//! its US position). Then, in order: an open `⌃⇥` cycle takes `⎋`; a sheet takes its keys;
//! the overview and link hints take theirs; an open overlay takes its fixed keys; and the keymap
//! resolves the stroke in the current [`WhenContext`] to a command, the first half of a chord,
//! or nothing. Nothing passes on to the page, the field, or the menu bar, so `⌘C`, `⌘H`, `⌘M`,
//! `⌃⌘F`, and the system's strokes keep working.
//!
//! A press the page does not take comes back: `WebKit` hands it to `AppKit` again, which runs
//! the monitor a second time. The monitor knows the press by its timestamp and key code and
//! lets it go on untouched, so nothing runs twice.
//!
//! - [`cycle`]: the `⌃⇥` hold.
//! - [`focus`]: what resets when the user's attention moves, and the key debug overlay.

mod cycle;
mod focus;

use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSView, NSWindow,
};
use quickie_domain_commands::{
    CommandId, ContextFlag, FocusedView, Key, KeyStroke, Modifiers, Resolution, WhenContext,
};

use super::Browser;
use super::commands::Action;
use super::page::SheetRoute;
use crate::input::keystroke::chord::PendingChord;
use crate::input::keystroke::log::{Outcome, Press};
use crate::input::keystroke::{OverlayKey, Read, is_close, is_quit, overlay_key, read};

/// What keeps the keys working: the event monitor and the observers of focus changes. Keyboard
/// shortcuts work for as long as it lives.
pub struct Shortcuts {
    _monitor: Option<Retained<AnyObject>>,
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl Browser {
    /// Handles a key press if it is the browser's; nobody else sees it then.
    fn key_down(&self, event: &NSEvent) -> bool {
        let flags = event.modifierFlags();
        let held = modifiers(flags);
        let function = flags.contains(NSEventModifierFlags::Function);
        let typed = event
            .charactersIgnoringModifiers()
            .map_or_else(String::new, |chars| chars.to_string());
        let read = read(event.keyCode(), &typed, held, function);
        let press = (event.timestamp().to_bits(), event.keyCode());
        let mut passed = self.passed.get();
        let outcome = if passed.contains(press) {
            Outcome::Returned
        } else {
            self.handle(event.window(self.mtm).as_deref(), read, held)
        };
        if !outcome.handled() {
            passed.remember(press);
            self.passed.set(passed);
        }
        self.log_key(
            Press {
                code: event.keyCode(),
                chars: typed,
                modifiers: held,
                function,
                stroke: match read {
                    Read::Stroke(stroke) => Some(stroke),
                    Read::Unnamed | Read::System => None,
                },
            },
            &outcome,
        );
        self.drain();
        outcome.handled()
    }

    fn handle(&self, window: Option<&NSWindow>, read: Read, held: Modifiers) -> Outcome {
        if let Some(window) = window
            && !self.windows.borrow().values().any(|own| **own == *window)
        {
            return self.other_window_key(window, read);
        }
        // A cycle whose modifier was let go where the monitor could not hear it (another app,
        // a space or full-screen switch) ends here instead of holding on forever.
        if !held.contains(self.cycle_modifiers.get()) {
            self.commit_cycle();
        }
        match read {
            Read::Stroke(stroke) => self.route(stroke),
            Read::System => {
                self.pending.take();
                Outcome::System
            }
            Read::Unnamed => {
                self.pending.take();
                // A key the keymap cannot name (the keypad, a media key) is the sheet's
                // field's, or nobody's while a sheet waits: never the page's under it.
                if self.sheet_takes_unnamed() {
                    Outcome::Overlay
                } else {
                    Outcome::Unnamed
                }
            }
        }
    }

    /// A key in a window that is not a browser window. The Settings window closes on `⌘W` and
    /// stays on `⌘,`; `⌘Q` quits from anywhere; the rest is the window's (its fields type,
    /// the Edit menu copies and pastes). A system panel, such as the print panel, keeps all.
    fn other_window_key(&self, window: &NSWindow, read: Read) -> Outcome {
        let Read::Stroke(stroke) = read else {
            return Outcome::Elsewhere;
        };
        if is_quit(stroke) {
            NSApplication::sharedApplication(self.mtm).terminate(None);
            return Outcome::Quit;
        }
        if !self.is_settings_window(window) {
            return Outcome::Elsewhere;
        }
        if is_close(stroke) {
            self.close_settings();
            return Outcome::Overlay;
        }
        let context = WhenContext::new().in_view(FocusedView::Settings);
        match self.keymap.borrow().resolve(stroke, None, &context) {
            Resolution::Command(id) if id.as_str() == "quickie.app.settings" => Outcome::Command {
                id,
                reserved: false,
            },
            _ => Outcome::Elsewhere,
        }
    }

    pub(super) fn route(&self, stroke: KeyStroke) -> Outcome {
        if self.cycle.borrow().is_some() && stroke.key == Key::Escape {
            self.cancel_cycle();
            return Outcome::Overlay;
        }
        if self.extension_sheet_key(stroke).is_some() {
            return Outcome::Overlay;
        }
        if let Some(route) = self.sheet_route(stroke) {
            return self.past_sheet(stroke, route);
        }
        if self.navigation_key(stroke) {
            return Outcome::Overlay;
        }
        let fixed = overlay_key(stroke);
        if let Some(key) = fixed
            && self.overlay_key(key)
        {
            return Outcome::Overlay;
        }
        if fixed.is_none() && self.address.is_open() {
            // Any other key edits: the page's URL turns into the field before it lands.
            self.address.wake();
        }
        let pending = self.take_pending();
        let resolution = self.keymap.borrow().resolve(stroke, pending, &self.when());
        match resolution {
            Resolution::Command(id) => self.run_stroke(id, stroke),
            Resolution::Pending(first) => {
                self.pending.set(PendingChord::begin(first, Instant::now()));
                Outcome::Pending
            }
            Resolution::Unbound => {
                if let Some(first) = pending {
                    let platform = self.keymap.borrow().platform();
                    self.toast(&format!(
                        "{} {} is not bound",
                        first.glyphs(platform),
                        stroke.glyphs(platform)
                    ));
                    Outcome::ChordUnbound
                } else if self.extension_key(stroke) {
                    Outcome::Extension
                } else if is_quit(stroke) {
                    NSApplication::sharedApplication(self.mtm).terminate(None);
                    Outcome::Quit
                } else {
                    Outcome::Passed
                }
            }
        }
    }

    /// The first stroke of a chord, if it still waits.
    fn take_pending(&self) -> Option<KeyStroke> {
        self.pending.take().first(Instant::now())
    }

    fn run_stroke(&self, id: CommandId, stroke: KeyStroke) -> Outcome {
        let reserved = self.keymap.borrow().is_reserved(stroke);
        self.dispatch(&id, Some(stroke));
        Outcome::Command { id, reserved }
    }

    /// A key while a sheet waits: the sheet's own keys are taken, its field types, and of the
    /// shortcuts only those that leave the page (closing or switching tabs and windows) and
    /// `⌘Q` run; the page under the sheet never gets a key.
    fn past_sheet(&self, stroke: KeyStroke, route: SheetRoute) -> Outcome {
        match route {
            SheetRoute::Taken => Outcome::Overlay,
            SheetRoute::Field => Outcome::Passed,
            SheetRoute::Keymap => {
                let pending = self.take_pending();
                let resolution = self.keymap.borrow().resolve(stroke, pending, &self.when());
                match resolution {
                    Resolution::Command(id)
                        if Action::of(&id).is_some_and(Action::passes_sheet) =>
                    {
                        self.run_stroke(id, stroke)
                    }
                    Resolution::Pending(first) => {
                        self.pending.set(PendingChord::begin(first, Instant::now()));
                        Outcome::Pending
                    }
                    _ if is_quit(stroke) => {
                        NSApplication::sharedApplication(self.mtm).terminate(None);
                        Outcome::Quit
                    }
                    _ => Outcome::Overlay,
                }
            }
        }
    }

    /// What is true right now, for `when` clauses.
    ///
    /// `inputFocused` is the page's word: its focus script reports whether the focused element
    /// takes text, so `F` types into a form and opens link hints everywhere else. It counts
    /// only while the page has the keyboard. An overlay's own field, the find bar's included,
    /// is focused input too.
    pub(super) fn when(&self) -> WhenContext {
        let mut context = WhenContext::new();
        let view = if self.command_palette.is_open() {
            context = context
                .with(ContextFlag::PaletteOpen)
                .with(ContextFlag::InputFocused);
            FocusedView::Palette
        } else if self.address.is_open() {
            context = context
                .with(ContextFlag::AddressOpen)
                .with(ContextFlag::InputFocused);
            FocusedView::Address
        } else if self.navigation.overview_open() {
            context = context
                .with(ContextFlag::OverviewOpen)
                .with(ContextFlag::InputFocused);
            FocusedView::Overview
        } else {
            context = context.with(ContextFlag::PageFocused);
            if self.find_focused() || self.page_input_focused() {
                context = context.with(ContextFlag::InputFocused);
            }
            FocusedView::Page
        };
        if self.navigation.hints_open() {
            context = context.with(ContextFlag::HintsOpen);
        }
        if self.cycle.borrow().is_some() {
            context = context.with(ContextFlag::SwitcherOpen);
        }
        if self.popovers.is_open() {
            context = context.with(ContextFlag::PopoverOpen);
        }
        if self.agent_ui.is_open() {
            context = context.with(ContextFlag::AgentOpen);
        }
        context.in_view(view)
    }

    /// True when the current window's page has the keyboard and says a text field of its own
    /// has focus. The page's report alone can be stale: after a full-screen or space switch
    /// the window may hold the keyboard itself while the page still remembers its field.
    fn page_input_focused(&self) -> bool {
        let Some((window, ns_window)) = self.current() else {
            return false;
        };
        let Some(tab) = self.active(window) else {
            return false;
        };
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return false;
        };
        let engine = browsing.engine_mut();
        let has_keyboard = engine
            .view(tab)
            .is_some_and(|page| first_responder_within(&ns_window, page));
        has_keyboard && engine.input_focused(tab)
    }

    /// An overlay's fixed key. While an input method composes, every key is the field's.
    fn overlay_key(&self, key: OverlayKey) -> bool {
        if self.agent_ui.is_open() {
            return self.agent_key(key);
        }
        if self.popovers.is_open() || self.extension_ui.popup_open() {
            return self.popover_key(key);
        }
        if self.command_palette.is_open() {
            if self.command_palette.is_composing() {
                return false;
            }
            self.palette_key(key);
            return true;
        }
        if self.address.is_open() {
            return self.address_key(key);
        }
        self.find_key(key)
    }
}

/// Whether `window`'s first responder is `view` or inside it.
fn first_responder_within(window: &NSWindow, view: &NSView) -> bool {
    window
        .firstResponder()
        .and_then(|responder| responder.downcast::<NSView>().ok())
        .is_some_and(|responder| responder.isDescendantOf(view))
}

/// Routes key presses and modifier changes to the browser before the window dispatches them to
/// the page or a field, and pinches to the tab overview; a click or a scroll takes link hints
/// and a waiting chord away. Focus changes of the windows and the app reset what a key left
/// behind ([`focus`]). Installed once, by `main`; shortcuts work while the result lives.
pub fn install_shortcuts(browser: &Rc<Browser>) -> Shortcuts {
    let monitor_browser = Rc::clone(browser);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid event that outlives this call.
        let key = unsafe { event.as_ref() };
        let browser = &monitor_browser;
        match key.r#type() {
            NSEventType::KeyDown if browser.key_down(key) => std::ptr::null_mut(),
            NSEventType::Magnify if browser.magnify(key) => std::ptr::null_mut(),
            NSEventType::FlagsChanged => {
                browser.flags_changed(key);
                event.as_ptr()
            }
            NSEventType::LeftMouseDown => {
                // A click moves the user's attention: a chord's first stroke stops waiting,
                // and the address overlay's URL turns into the field to click into.
                browser.pending.take();
                if browser.address.is_open() {
                    browser.address.wake();
                }
                browser.pointer();
                event.as_ptr()
            }
            NSEventType::ScrollWheel => {
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
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };
    Shortcuts {
        _monitor: monitor,
        _observers: focus::observe(browser),
    }
}

/// The modifiers the keymap reads; Caps Lock and `fn` do not count (`fn` is read apart).
pub(in crate::browser) fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
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

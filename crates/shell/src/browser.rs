//! The browser: the browsing use cases, one `NSWindow` per session window, the overlays, and
//! the keys that drive them. Every change goes through `Browsing`; this file only maps keys to
//! use cases and puts the outcome on screen.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSWindow,
};
use objc2_foundation::NSTimer;
use quickie_application::{AppError, Navigation};
use quickie_domain_browsing::{
    CorruptSession, Direction, Focus, JarChoice, PinnedSlot, Session, TabId, WindowId,
};
use url::Url;

use crate::App;
use crate::input::cycle::CycleHold;
use crate::input::destination::destination;
use crate::input::shortcut::{self, Action, Context, Modifiers};
use crate::overlay::address::AddressOverlay;
use crate::overlay::switcher::TabSwitcher;
use crate::window::{browser_window, set_title};

pub struct Browser {
    browsing: RefCell<App>,
    windows: RefCell<BTreeMap<WindowId, Retained<NSWindow>>>,
    address: AddressOverlay,
    switcher: TabSwitcher,
    cycle: RefCell<Option<CycleHold>>,
    /// The start page: new tabs open on it, and the address overlay shows it as empty.
    home: Url,
    /// Why the saved session was discarded at launch.
    // TODO(toast): tell the user through the toast widget once `ui/widgets/feedback` exists.
    #[expect(
        dead_code,
        reason = "read by the toast widget, which does not exist yet"
    )]
    corrupt_on_start: Option<CorruptSession>,
    /// The last command that failed.
    // TODO(toast): report it through the toast widget as well.
    failure: RefCell<Option<AppError>>,
    this: Weak<Self>,
    mtm: MainThreadMarker,
}

impl Browser {
    /// Puts a started session on screen: one window per session window, each showing its
    /// active tab. Engine reports reach the session from here on.
    pub fn new(browsing: App, home: Url, mtm: MainThreadMarker) -> Rc<Self> {
        let corrupt_on_start = browsing.corrupt_on_start();
        let browser = Rc::new_cyclic(|this| Self {
            browsing: RefCell::new(browsing),
            windows: RefCell::new(BTreeMap::new()),
            address: AddressOverlay::new(mtm),
            switcher: TabSwitcher::new(mtm),
            cycle: RefCell::new(None),
            home,
            corrupt_on_start,
            failure: RefCell::new(None),
            this: this.clone(),
            mtm,
        });
        browser.open_windows();
        let weak = Rc::downgrade(&browser);
        browser
            .browsing
            .borrow_mut()
            .engine_mut()
            .on_navigated(move || {
                if let Some(browser) = weak.upgrade() {
                    browser.drain();
                }
            });
        browser
    }

    pub fn show(&self) {
        let windows: Vec<(WindowId, Retained<NSWindow>)> = self
            .windows
            .borrow()
            .iter()
            .map(|(&id, window)| (id, window.clone()))
            .collect();
        for (id, window) in windows {
            window.center();
            window.makeKeyAndOrderFront(None);
            self.settle(id);
        }
    }

    /// Records what pages committed since the last time. Runs from the engine's waker on the
    /// main run loop, and after every command.
    pub fn drain(&self) {
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let navigations = browsing.engine_mut().take_navigations();
        if navigations.is_empty() {
            return;
        }
        for navigated in navigations {
            if let Err(error) = browsing.navigated(navigated.tab, navigated.url, navigated.title) {
                *self.failure.borrow_mut() = Some(error);
            }
        }
        drop(browsing);
        self.refresh_titles();
    }

    /// Handles a key press if it is a browser shortcut; the page never sees it then.
    fn key_down(&self, event: &NSEvent) -> bool {
        let context = Context {
            address_open: self.address.is_open(),
            cycling: self.cycle.borrow().is_some(),
        };
        let action = shortcut::resolve(event.keyCode(), modifiers(event.modifierFlags()), context);
        let handled = action.is_some_and(|action| self.perform(action));
        self.drain();
        handled
    }

    /// Releasing `⌃` ends a cycle on its selection.
    fn flags_changed(&self, event: &NSEvent) {
        if !event
            .modifierFlags()
            .contains(NSEventModifierFlags::Control)
        {
            self.commit_cycle();
        }
    }

    fn perform(&self, action: Action) -> bool {
        let Some((window, ns_window)) = self.current() else {
            return false;
        };
        match action {
            Action::OpenAddress => self.open_address(window, &ns_window),
            Action::Back => self.navigate(window, &Navigation::Back),
            Action::Forward => self.navigate(window, &Navigation::Forward),
            Action::Reload => self.navigate(window, &Navigation::Reload),
            Action::Quit => NSApplication::sharedApplication(self.mtm).terminate(None),
            Action::NewTab => self.new_tab(window, &ns_window),
            Action::CloseTab => self.close_tab(window),
            Action::ReopenClosed => self.reopen_closed(window),
            Action::GoToSlot(slot) => self.go_to_slot(window, slot),
            Action::PinToSlot(slot) => self.pin(window, slot),
            Action::Cycle(direction) => self.cycle(window, direction),
            Action::CancelCycle => self.cancel_cycle(),
            Action::Submit => return self.submit(window),
            Action::Dismiss => self.close_address(window),
        }
        true
    }

    fn navigate(&self, window: WindowId, navigation: &Navigation) {
        if let Some(tab) = self.active(window) {
            self.run(|browsing| browsing.navigate(tab, navigation));
        }
    }

    /// ⌘T: a tab on the start page, with the address overlay open to say where to go.
    fn new_tab(&self, window: WindowId, ns_window: &NSWindow) {
        self.address.hide();
        let home = self.home.clone();
        let opened =
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
        self.settle(window);
        if opened.is_some() {
            self.open_address(window, ns_window);
        }
    }

    /// ⌘W: the tab goes to the trash. A window left empty gets a fresh start page for now,
    /// rather than closing the last window.
    fn close_tab(&self, window: WindowId) {
        self.address.hide();
        if let Some(tab) = self.active(window) {
            self.run(|browsing| browsing.close(tab));
        }
        if self.active(window).is_none() {
            let home = self.home.clone();
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
        }
        self.settle(window);
    }

    fn reopen_closed(&self, window: WindowId) {
        self.address.hide();
        self.run(|browsing| browsing.reopen_closed(window));
        self.settle(window);
    }

    /// ⌘1 … ⌘9. The pinned tab may live in another window, which comes forward.
    fn go_to_slot(&self, window: WindowId, slot: PinnedSlot) {
        self.address.hide();
        let Some(Some(target)) = self.run(|browsing| browsing.go_to_slot(slot)) else {
            self.settle(window);
            return;
        };
        if target != window
            && let Some(ns_window) = self.window(target)
        {
            ns_window.makeKeyAndOrderFront(None);
        }
        self.settle(target);
    }

    fn pin(&self, window: WindowId, slot: PinnedSlot) {
        if let Some(tab) = self.active(window) {
            self.run(|browsing| browsing.pin(tab, slot));
        }
    }

    /// ⌃⇥: begins a cycle, or steps the one in progress. Nothing shows until the hold passes
    /// the threshold, so a quick tap switches with no UI at all.
    fn cycle(&self, window: WindowId, direction: Direction) {
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
        *self.cycle.borrow_mut() = Some(CycleHold::new(cycle, window, Instant::now()));
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

    fn cancel_cycle(&self) {
        if self.cycle.borrow_mut().take().is_some() {
            self.switcher.hide();
        }
    }

    fn open_address(&self, window: WindowId, ns_window: &NSWindow) {
        let Some(host) = ns_window.contentView() else {
            return;
        };
        let url = self.active(window).and_then(|tab| {
            let browsing = self.browsing.borrow();
            browsing.session().tab(tab).map(|tab| tab.url.clone())
        });
        let url = url.filter(|url| *url != self.home);
        self.address
            .show(&host, ns_window, url.as_ref().map_or("", Url::as_str));
    }

    /// ⏎ in the overlay. While an input method composes, ⏎ is the field's.
    fn submit(&self, window: WindowId) -> bool {
        if self.address.is_composing() {
            return false;
        }
        let target = destination(&self.address.text()).and_then(|url| Url::parse(&url).ok());
        self.close_address(window);
        if let Some(url) = target {
            self.navigate(window, &Navigation::Load(url));
        }
        true
    }

    fn close_address(&self, window: WindowId) {
        self.address.hide();
        self.settle(window);
    }

    /// After a command: the window's active page takes the keyboard (unless the address
    /// overlay has it), and window titles follow the session.
    fn settle(&self, window: WindowId) {
        let view = {
            let mut browsing = self.browsing.borrow_mut();
            let active = browsing.session().active(window);
            active.and_then(|tab| browsing.engine_mut().view(tab).map(Message::retain))
        };
        if let (Some(ns_window), Some(view)) = (self.window(window), view)
            && !self.address.is_open()
        {
            ns_window.makeFirstResponder(Some(&view));
        }
        self.refresh_titles();
    }

    fn refresh_titles(&self) {
        let browsing = self.browsing.borrow();
        let session = browsing.session();
        for (&id, window) in self.windows.borrow().iter() {
            let title = session
                .active(id)
                .and_then(|tab| session.tab(tab))
                .map_or("", |tab| tab.title.as_str());
            set_title(window, title);
        }
    }

    /// One `NSWindow` per session window, its content view registered with the engine.
    fn open_windows(&self) {
        let ids: Vec<WindowId> = self.browsing.borrow().session().windows().collect();
        for id in ids {
            if self.windows.borrow().contains_key(&id) {
                continue;
            }
            let window = browser_window(self.mtm);
            if let Some(host) = window.contentView() {
                self.browsing
                    .borrow_mut()
                    .engine_mut()
                    .attach_host(id, &host);
            }
            self.windows.borrow_mut().insert(id, window);
        }
    }

    /// The window shortcuts act on: the key window, else the first one.
    fn current(&self) -> Option<(WindowId, Retained<NSWindow>)> {
        let key = NSApplication::sharedApplication(self.mtm).keyWindow();
        let windows = self.windows.borrow();
        key.and_then(|key| windows.iter().find(|(_, window)| ***window == *key))
            .or_else(|| windows.iter().next())
            .map(|(&id, window)| (id, window.clone()))
    }

    fn window(&self, id: WindowId) -> Option<Retained<NSWindow>> {
        self.windows.borrow().get(&id).cloned()
    }

    fn active(&self, window: WindowId) -> Option<TabId> {
        self.browsing.borrow().session().active(window)
    }

    /// Runs a use case; a failure is kept for the toast and yields `None`.
    fn run<T>(&self, command: impl FnOnce(&mut App) -> Result<T, AppError>) -> Option<T> {
        let outcome = command(&mut self.browsing.borrow_mut());
        match outcome {
            Ok(value) => Some(value),
            Err(error) => {
                *self.failure.borrow_mut() = Some(error);
                None
            }
        }
    }
}

/// What the switcher calls a tab: its title, else its host, else its URL.
fn label(session: &Session, tab: TabId) -> String {
    let Some(tab) = session.tab(tab) else {
        return String::new();
    };
    if tab.title.is_empty() {
        tab.url.host_str().unwrap_or(tab.url.as_str()).to_owned()
    } else {
        tab.title.clone()
    }
}

/// Routes key presses and modifier changes to the browser before the window dispatches them to
/// the page or a field. The returned monitor must stay alive for as long as shortcuts should
/// work.
pub fn install_shortcuts(browser: &Rc<Browser>) -> Option<Retained<AnyObject>> {
    let browser = Rc::clone(browser);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid event that outlives this call.
        let key = unsafe { event.as_ref() };
        match key.r#type() {
            NSEventType::KeyDown if browser.key_down(key) => std::ptr::null_mut(),
            NSEventType::FlagsChanged => {
                browser.flags_changed(key);
                event.as_ptr()
            }
            _ => event.as_ptr(),
        }
    });
    // SAFETY: the handler returns either the event it was given or null, as the API requires,
    // and AppKit calls it on the main thread, where the captured browser lives.
    unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown | NSEventMask::FlagsChanged,
            &handler,
        )
    }
}

fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers::NONE
        .with(
            Modifiers::COMMAND,
            flags.contains(NSEventModifierFlags::Command),
        )
        .with(
            Modifiers::SHIFT,
            flags.contains(NSEventModifierFlags::Shift),
        )
        .with(
            Modifiers::OPTION,
            flags.contains(NSEventModifierFlags::Option),
        )
        .with(
            Modifiers::CONTROL,
            flags.contains(NSEventModifierFlags::Control),
        )
}

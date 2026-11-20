//! Windows: `⌘N` opens one with the same chrome (none), `⌘⇧W` closes one into the trash, and a
//! window whose last tab closes goes with it unless it is the last one. Keys act on the key
//! window ([`Browser::current`]). Pages that open or close pages: [`opened`].

mod opened;

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2_foundation::NSPoint;
use quickie_domain_browsing::{Focus, JarChoice, TabId, WindowId};

use super::Browser;
use crate::window::browser_window;

impl Browser {
    /// `⌘N`: a new window on the start page, cascaded from the current one, with the address
    /// overlay open to say where to go.
    pub(super) fn new_window(&self) {
        let Some(window) = self.run(crate::App::open_window) else {
            return;
        };
        let ns_window = self.add_window(window);
        let home = self.home.clone();
        let opened =
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
        ns_window.makeKeyAndOrderFront(None);
        self.settle(window);
        if opened.is_some() {
            self.open_address();
        }
    }

    /// `⌘⇧W`: the window and its tabs go to the trash. The last window is replaced by a fresh
    /// one, so the app always has a page to show.
    pub(super) fn close_window(&self, window: WindowId) {
        self.close_overlays();
        self.hide_find();
        self.hide_navigation();
        if self.run(|browsing| browsing.close_window(window)).is_none() {
            return;
        }
        self.remove_window(window);
        let next = self.current();
        match next {
            Some((next, ns_window)) => {
                ns_window.makeKeyAndOrderFront(None);
                self.settle(next);
            }
            None => self.new_window(),
        }
    }

    /// Closes `tab` of `window` into the trash. The last tab of a window takes the window with
    /// it, unless it is the last window, which gets a fresh start page instead.
    pub(super) fn close_tab_in(&self, window: WindowId, tab: TabId) {
        let (tabs, windows) = {
            let browsing = self.browsing.borrow();
            let session = browsing.session();
            (
                session.mru(window).map_or(0, <[TabId]>::len),
                session.windows().count(),
            )
        };
        if tabs <= 1 && windows > 1 {
            self.close_window(window);
            return;
        }
        self.run(|browsing| browsing.close(tab));
        if self.active(window).is_none() {
            let home = self.home.clone();
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
        }
        self.settle(window);
    }

    /// The window a tab is in.
    pub(super) fn window_of(&self, tab: TabId) -> Option<WindowId> {
        let browsing = self.browsing.borrow();
        let session = browsing.session();
        session
            .windows()
            .find(|&window| session.mru(window).is_some_and(|mru| mru.contains(&tab)))
    }

    /// An `NSWindow` for session window `window`, its content view registered with the
    /// engine, placed a step down and right of the current window.
    pub(super) fn add_window(&self, window: WindowId) -> Retained<NSWindow> {
        let ns_window = browser_window(self.mtm);
        match self.current() {
            Some((_, current)) => {
                let top_left = current.frame().origin;
                let top = NSPoint::new(top_left.x, top_left.y + current.frame().size.height);
                ns_window.cascadeTopLeftFromPoint(ns_window.cascadeTopLeftFromPoint(top));
            }
            None => ns_window.center(),
        }
        if let Some(host) = ns_window.contentView() {
            self.browsing
                .borrow_mut()
                .engine_mut()
                .attach_host(window, &host);
        }
        self.windows.borrow_mut().insert(window, ns_window.clone());
        ns_window
    }

    /// Takes a closed window off the screen and forgets it.
    fn remove_window(&self, window: WindowId) {
        let Some(ns_window) = self.windows.borrow_mut().remove(&window) else {
            return;
        };
        self.browsing.borrow_mut().engine_mut().detach_host(window);
        self.forget_window_page(window);
        ns_window.orderOut(None);
        ns_window.close();
        self.present_asks();
    }
}

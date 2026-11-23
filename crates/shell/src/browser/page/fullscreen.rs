//! A page that goes full screen (the Fullscreen API: a video's full-screen button,
//! `requestFullscreen`) always gets the full-screen notice: whose page it is and how to leave.
//!
//! When the engine fills the page's own view ([`Fullscreen::InWindow`]), the page fills its
//! existing window in the current Space. The page frame drops its inset. Toasts, the origin
//! chip, the find bar, and the load glow are hidden, and a browser command first takes the
//! page out of full screen. Escape, switching tabs, closing the tab, and leaving native
//! window fullscreen also exit page fullscreen.

use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow};
use quickie_application::{Fullscreen, PageAsks, PageSurface};
use quickie_domain_browsing::{TabId, WindowId};
use url::Url;

use super::super::Browser;

impl Browser {
    /// A page entered or left full screen.
    pub(in crate::browser) fn fullscreen_changed(&self, tab: TabId, state: Fullscreen) {
        match state {
            Fullscreen::InWindow => self.fill_window(tab),
            Fullscreen::OwnWindow => self.show_fullscreen_notice(tab),
            Fullscreen::Left => {
                self.hide_fullscreen_notice(tab);
                let entry = self
                    .page
                    .in_window
                    .borrow()
                    .iter()
                    .find_map(|(&window, &page)| (page == tab).then_some(window));
                if let Some(window) = entry {
                    self.clear_page_fullscreen(window);
                }
            }
        }
    }

    /// True while a page fills `window`, full screen.
    pub(in crate::browser) fn page_fullscreen(&self, window: WindowId) -> bool {
        self.page.in_window.borrow().contains_key(&window)
    }

    /// Takes the page that fills the current window out of full screen; false when none does.
    pub(in crate::browser) fn leave_page_fullscreen(&self) -> bool {
        let Some((window, _)) = self.current() else {
            return false;
        };
        let tab = self.page.in_window.borrow().get(&window).copied();
        let Some(tab) = tab else {
            return false;
        };
        self.exit_fullscreen(tab);
        true
    }

    /// When the user leaves native window fullscreen, the page that filled it leaves too.
    pub(in crate::browser) fn window_leaves_fullscreen(&self, window: &NSWindow) {
        let id = self
            .windows
            .borrow()
            .iter()
            .find(|(_, own)| ***own == *window)
            .map(|(&id, _)| id);
        let tab = id.and_then(|id| self.page.in_window.borrow().get(&id).copied());
        if let Some(tab) = tab {
            self.exit_fullscreen(tab);
        }
    }

    /// Forgets the page that filled `window` and restores the page frame's inset.
    pub(in crate::browser) fn forget_page_fullscreen(&self, window: WindowId) {
        if let Some(tab) = self.clear_page_fullscreen(window) {
            self.exit_fullscreen(tab);
        }
    }

    fn clear_page_fullscreen(&self, window: WindowId) -> Option<TabId> {
        let tab = self.page.in_window.borrow_mut().remove(&window)?;
        self.hide_fullscreen_notice(tab);
        if let Some(frame) = self.frames.borrow().get(&window) {
            frame.set_page_full_screen(false);
        }
        Some(tab)
    }

    /// The active page of `tab` fills its window and the browser's floating UI goes away.
    fn fill_window(&self, tab: TabId) {
        let Some(window) = self.showing(tab) else {
            self.exit_fullscreen(tab);
            return;
        };
        self.toasts.clear();
        self.page.chip.hide();
        if let Some(frame) = self.frames.borrow().get(&window) {
            frame.reset_loading();
            frame.set_page_full_screen(true);
        }
        if self.page.find_tab.get() == Some(tab) {
            self.hide_find();
        }
        self.page.in_window.borrow_mut().insert(window, tab);
        self.show_fullscreen_notice(tab);
    }

    fn exit_fullscreen(&self, tab: TabId) {
        if let Ok(mut browsing) = self.browsing.try_borrow_mut() {
            browsing.engine_mut().exit_fullscreen(tab);
        }
    }

    /// The notice for a page that entered full screen, over the window it fills.
    fn show_fullscreen_notice(&self, tab: TabId) {
        let shown: Option<(Retained<NSView>, Url)> = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            let url = browsing.session().tab(tab).map(|tab| tab.url.clone());
            let host = browsing
                .engine()
                .view(tab)
                .and_then(NSView::window)
                .and_then(|window| window.contentView());
            host.zip(url)
        };
        if let Some((host, url)) = shown {
            self.page.fullscreen_notice_tab.set(Some(tab));
            self.page.fullscreen.show(&host, &url);
        }
    }

    fn hide_fullscreen_notice(&self, tab: TabId) {
        if self.page.fullscreen_notice_tab.get() == Some(tab) {
            self.page.fullscreen_notice_tab.set(None);
            self.page.fullscreen.hide();
        }
    }
}

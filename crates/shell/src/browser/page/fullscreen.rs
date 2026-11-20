//! A page that goes full screen (the Fullscreen API: a video's full-screen button,
//! `requestFullscreen`) always gets the full-screen notice: whose page it is and how to leave.
//!
//! When the engine fills the page's own view ([`Fullscreen::InWindow`]), the browser takes the
//! tab's window full screen, on its own Space, as Chrome does, and back when the page leaves;
//! a window that was full screen already stays so. Meanwhile nothing of the browser's shows
//! over the page: toasts, the origin chip, the find bar, and the progress line are gone, and a
//! browser command first takes the page out of full screen, then runs. `⎋` takes it out; so do
//! the window leaving full screen, another tab coming forward, and the tab closing. Leaving
//! puts the window back where it was (`AppKit` keeps its frame) and the page where it was (the
//! engine keeps its scroll).

use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowStyleMask};
use quickie_application::{Fullscreen, PageAsks, PageSurface};
use quickie_domain_browsing::{TabId, WindowId};
use url::Url;

use super::super::Browser;

/// A page that fills its window, full screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) struct PageFullscreen {
    pub(in crate::browser) tab: TabId,
    /// The browser took the window full screen for it, so it takes it back afterwards.
    pub(in crate::browser) took_window: bool,
}

impl Browser {
    /// A page entered or left full screen.
    pub(in crate::browser) fn fullscreen_changed(&self, tab: TabId, state: Fullscreen) {
        match state {
            Fullscreen::InWindow => self.fill_window(tab),
            Fullscreen::OwnWindow => self.show_fullscreen_notice(tab),
            Fullscreen::Left => {
                self.page.fullscreen.hide();
                let entry = self
                    .page
                    .in_window
                    .borrow()
                    .iter()
                    .find(|(_, page)| page.tab == tab)
                    .map(|(&window, _)| window);
                if let Some(window) = entry {
                    self.forget_page_fullscreen(window);
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
        let tab = self
            .page
            .in_window
            .borrow()
            .get(&window)
            .map(|page| page.tab);
        let Some(tab) = tab else {
            return false;
        };
        self.exit_fullscreen(tab);
        true
    }

    /// `window` leaves full screen on the user's word (its green button, `⌃⌘F`, the menu):
    /// the page that filled it leaves too, and the window is not taken back twice.
    pub(in crate::browser) fn window_leaves_fullscreen(&self, window: &NSWindow) {
        let id = self
            .windows
            .borrow()
            .iter()
            .find(|(_, own)| ***own == *window)
            .map(|(&id, _)| id);
        let tab = id.and_then(|id| {
            let mut pages = self.page.in_window.borrow_mut();
            let page = pages.get_mut(&id)?;
            page.took_window = false;
            Some(page.tab)
        });
        if let Some(tab) = tab {
            self.exit_fullscreen(tab);
        }
    }

    /// Forgets the page that filled `window`: the engine takes it out (nothing for a closed
    /// tab), and the window leaves full screen if the browser took it there.
    pub(in crate::browser) fn forget_page_fullscreen(&self, window: WindowId) {
        let Some(page) = self.page.in_window.borrow_mut().remove(&window) else {
            return;
        };
        self.page.fullscreen.hide();
        self.exit_fullscreen(page.tab);
        if let Some(ns_window) = self.window(window)
            && page.took_window
            && ns_window
                .styleMask()
                .contains(NSWindowStyleMask::FullScreen)
        {
            ns_window.toggleFullScreen(None);
        }
    }

    /// The page of `tab` fills its window: the browser's own floating UI goes, and the window
    /// goes full screen. A tab in the background may not take its window.
    fn fill_window(&self, tab: TabId) {
        let Some(window) = self.showing(tab) else {
            self.exit_fullscreen(tab);
            return;
        };
        let Some(ns_window) = self.window(window) else {
            return;
        };
        self.toasts.clear();
        self.page.chip.hide();
        if let Some(line) = self.page.lines.borrow().get(&window) {
            line.reset();
        }
        if self.page.find_tab.get() == Some(tab) {
            self.hide_find();
        }
        let full = ns_window
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen);
        let previous = self.page.in_window.borrow_mut().insert(
            window,
            PageFullscreen {
                tab,
                took_window: !full,
            },
        );
        match previous {
            // Another element of the same page: the window is full screen already.
            Some(page) if page.tab == tab => {
                if let Some(entry) = self.page.in_window.borrow_mut().get_mut(&window) {
                    entry.took_window = page.took_window;
                }
            }
            _ if !full => ns_window.toggleFullScreen(None),
            _ => {}
        }
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
            self.page.fullscreen.show(&host, &url);
        }
    }
}

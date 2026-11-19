//! Pages that open pages and close themselves. A popup, a `target=_blank` link, or a
//! `⌘`-click becomes a tab in its opener's window and jar through `Browsing::adopt`: in front
//! for a popup or a plain `target=_blank`, behind for `⌘`-click and middle-click. A page that
//! calls `window.close()` closes its tab into the trash, as `⌘W` would.

use quickie_domain_browsing::Focus;

use super::super::Browser;

impl Browser {
    /// Adopts every page pages opened since the last time; a foreground one is shown.
    pub(in crate::browser) fn adopt_opened(&self) {
        let pages = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            browsing.engine_mut().take_opened()
        };
        for page in pages {
            let (focus, opener) = (page.focus, page.opener);
            let adopted =
                self.run(|browsing| browsing.adopt(page.opener, page.page, page.url, page.focus));
            if adopted.is_none() || focus == Focus::Background {
                continue;
            }
            if let Some(window) = self.window_of(opener) {
                if let Some(ns_window) = self.window(window) {
                    ns_window.makeKeyAndOrderFront(None);
                }
                self.settle(window);
            }
        }
    }

    /// Closes the tabs whose pages closed themselves.
    pub(in crate::browser) fn close_closed_pages(&self) {
        let closed = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            browsing.engine_mut().take_closed()
        };
        for tab in closed {
            if let Some(window) = self.window_of(tab) {
                self.close_tab_in(window, tab);
            }
        }
    }
}

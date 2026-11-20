//! Pages that open pages and close themselves. A popup, a `target=_blank` link, or a
//! `⌘`-click becomes a tab in its opener's window and jar through `Browsing::adopt`: in front
//! for a popup or a plain `target=_blank`, behind for `⌘`-click and middle-click. A page that
//! calls `window.close()` closes its tab into the trash, as `⌘W` would.

use quickie_domain_browsing::{Focus, TabId};
use url::Url;

use super::super::Browser;
use crate::OpenedPage;

impl Browser {
    /// Adopts a page a page opened; a foreground one is shown.
    pub(in crate::browser) fn adopt_opened(
        &self,
        opener: TabId,
        page: OpenedPage,
        url: Url,
        focus: Focus,
    ) {
        let adopted = self.run(|browsing| browsing.adopt(opener, page, url, focus));
        if adopted.is_none() || focus == Focus::Background {
            return;
        }
        if let Some(window) = self.window_of(opener) {
            if let Some(ns_window) = self.window(window) {
                ns_window.makeKeyAndOrderFront(None);
            }
            self.settle(window);
        }
    }

    /// Closes the tab whose page closed itself.
    pub(in crate::browser) fn closed_by_page(&self, tab: TabId) {
        if let Some(window) = self.window_of(tab) {
            self.close_tab_in(window, tab);
        }
    }
}

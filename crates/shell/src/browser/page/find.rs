//! Find in page: the bar's requests and keys become engine finds, and their answers come back
//! through [`Browser::found`]. The bar is an E2: opening another E2 closes it.

use quickie_application::{FindStep, Found, PageContent};
use quickie_ui_widgets::FindRequest;

use super::super::Browser;
use crate::input::keystroke::OverlayKey;

impl Browser {
    /// Hands the find bar's edits, buttons, and closing to the browser.
    pub(in crate::browser) fn wire_find(&self) {
        let this = self.this.clone();
        self.page.find_bar.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                match request {
                    FindRequest::Search => browser.search(FindStep::Refine),
                    FindRequest::Step { backwards } => browser.find_step(backwards),
                    FindRequest::Close => browser.close_find(),
                }
                browser.drain();
            }
        });
    }

    /// `⌘F`: opens the bar on the current window's active tab, or gives it the keyboard back.
    /// The last query stays, selected; the search goes on from where it was with `⏎` or `⌘G`.
    pub(in crate::browser) fn open_find(&self) {
        let Some((window, ns_window)) = self.current() else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        self.address.hide();
        self.command_palette.hide();
        self.hide_popovers();
        self.cancel_cycle();
        let tab = self.active(window);
        if self.page.find_tab.get() != tab {
            self.hide_find();
        }
        self.page.find_tab.set(tab);
        self.page.find_bar.show(&host, &ns_window);
    }

    /// `⏎` / `⌘G` and `⇧⏎` / `⇧⌘G`: the next or previous match, opening the bar first if
    /// needed.
    pub(in crate::browser) fn find_step(&self, backwards: bool) {
        if !self.page.find_bar.is_open() {
            self.open_find();
        }
        let step = if backwards {
            FindStep::Previous
        } else {
            FindStep::Next
        };
        self.search(step);
    }

    /// `⎋` in the bar or its close button: the marks go, and the page takes the keyboard.
    pub(in crate::browser) fn close_find(&self) {
        self.hide_find();
        if let Some((window, _)) = self.current() {
            self.settle(window);
        }
    }

    /// True when the find bar's field has the keyboard: a letter typed now is the query's.
    pub(in crate::browser) fn find_focused(&self) -> bool {
        self.page.find_bar.is_open() && self.page.find_bar.has_focus()
    }

    /// Takes the bar and the marks away, leaving the keyboard where it is: another E2 is about
    /// to take it, or another tab came forward.
    pub(in crate::browser) fn hide_find(&self) {
        if !self.page.find_bar.is_open() {
            return;
        }
        self.page.find_bar.hide();
        if let Some(tab) = self.page.find_tab.take() {
            // A tab whose page is gone has no marks left to clear.
            let _ = self.browsing.borrow_mut().engine_mut().clear_find(tab);
        }
    }

    /// The fixed keys of the bar while its field has the keyboard: `⏎` next, `⇧⏎` previous,
    /// `⎋` closes. Arrows and the rest stay the field's.
    pub(in crate::browser) fn find_key(&self, key: OverlayKey) -> bool {
        let bar = &self.page.find_bar;
        if !bar.has_focus() || bar.is_composing() {
            return false;
        }
        match key {
            OverlayKey::Accept { .. } => self.find_step(false),
            OverlayKey::Previous => self.find_step(true),
            OverlayKey::Dismiss => self.close_find(),
            OverlayKey::Move(_) => return false,
        }
        true
    }

    /// Runs the bar's query on its tab.
    fn search(&self, step: FindStep) {
        let Some(tab) = self.page.find_tab.get() else {
            return;
        };
        let query = self.page.find_bar.query();
        // A tab without a page has no matches to show, and the bar keeps the ones it shows.
        let _ = self
            .browsing
            .borrow_mut()
            .engine_mut()
            .find(tab, &query, step);
    }

    /// The matches of a find, shown when they answer the query the bar holds now.
    pub(in crate::browser) fn found(&self, found: &Found) {
        let bar = &self.page.find_bar;
        if bar.is_open()
            && self.page.find_tab.get() == Some(found.tab)
            && bar.query() == found.query
        {
            bar.set_matches(found.matches);
        }
    }
}

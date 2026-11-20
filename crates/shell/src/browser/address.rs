//! The address overlay (`⌘L`): it opens on the page with the page's actions, and each edit asks
//! the palette's index for suggestions, synchronously, from what it gathered when it opened:
//! open tabs, bookmarks, and history, never the network. `⏎` runs the selected row in this
//! tab, `⌘⏎` in a new one, `⌥⏎` in a new throwaway one; an open tab is switched to wherever.
//! `⎋` brings the page back, then closes.

use quickie_application::Target;
use quickie_domain_browsing::Destination;
use url::Url;

use super::Browser;
use super::palette::Opening;
use crate::input::keystroke::OverlayKey;

impl Browser {
    /// Hands the overlay's edits and clicks to the browser.
    pub(super) fn wire_address(&self) {
        let this = self.this.clone();
        self.address.on_change(move || {
            if let Some(browser) = this.upgrade() {
                browser.suggest();
            }
        });
        let this = self.this.clone();
        self.address.on_click(move |row| {
            if let Some(browser) = this.upgrade() {
                if let Some(target) = browser.address.target_at(row) {
                    browser.run_target(target, Opening::Here);
                }
                browser.drain();
            }
        });
    }

    /// `⌘L`: the overlay on the current page, or empty on the start page.
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
        // Shields does not count what it blocks on a page yet: no count, rather than a guess.
        self.address.show(
            &host,
            &ns_window,
            url.as_ref().map_or("", Url::as_str),
            None,
        );
        self.palette_sources_changed();
        let actions = self.with_index(|index| index.page_actions());
        self.address.set_actions(actions);
    }

    /// Answers what the user typed.
    fn suggest(&self) {
        let typed = self.address.typed();
        let complete = self.address.completes();
        let answer = self.with_index(|index| index.suggest(&typed, complete));
        self.address.set_suggestions(answer);
    }

    /// The overlay's fixed keys. While an input method composes, every key is the field's.
    pub(super) fn address_key(&self, key: OverlayKey) -> bool {
        if self.address.is_composing() {
            return false;
        }
        match key {
            OverlayKey::Move(delta) => self.address.move_selection(delta),
            OverlayKey::Previous => return false,
            OverlayKey::Accept { new_tab, throwaway } => {
                self.submit_address(Opening::of(new_tab, throwaway));
            }
            OverlayKey::Dismiss => {
                if !self.address.revert() {
                    self.close_address();
                }
            }
        }
        true
    }

    /// Runs the selected row; with no row (the field holds an address the user did not
    /// change), goes where the field's text says.
    fn submit_address(&self, opening: Opening) {
        let target = self
            .address
            .chosen()
            .or_else(|| Destination::of(&self.address.text()).map(|to| Target::Url(to.into_url())));
        match target {
            Some(target) => self.run_target(target, opening),
            None => self.close_address(),
        }
    }

    pub(super) fn close_address(&self) {
        self.address.hide();
        if let Some((window, _)) = self.current() {
            self.settle(window);
        }
    }
}

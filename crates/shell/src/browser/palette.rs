//! The command palette: showing it gathers what it offers once ([`Palette::open`]), and each
//! edit only ranks that again, synchronously, within a frame. When the session or the library
//! changes while it shows, the next edit gathers afresh. Choosing a row runs its [`Target`]. A
//! `?` row asks the agent: nothing goes to the endpoint while the user types, only on `⏎`.
//!
//! [`Palette::open`]: quickie_application::Palette::open

use quickie_application::{Navigation, PaletteIndex, Sources, Target};
use url::Url;

use super::Browser;
use crate::input::keystroke::OverlayKey;

/// How many answers a query asks for; the list shows eight and scrolls the rest.
const ANSWERS: usize = 50;

impl Browser {
    /// Hands the palette's edits and clicks to the browser.
    pub(super) fn wire_palette(&self) {
        let this = self.this.clone();
        self.command_palette.on_change(move || {
            if let Some(browser) = this.upgrade() {
                browser.requery();
            }
        });
        let this = self.this.clone();
        self.command_palette.on_click(move |row| {
            if let Some(browser) = this.upgrade() {
                browser.choose(row, false);
                browser.drain();
            }
        });
    }

    /// `⇧⌘P` / `⌘K` open it on `>`, `⌘P` empty for going to a tab or a page.
    pub(super) fn open_palette(&self, prefix: &str) {
        let Some((_, ns_window)) = self.current() else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        self.address.hide();
        self.hide_popovers();
        self.cancel_cycle();
        self.hide_find();
        self.command_palette.show(&host, &ns_window, prefix);
        self.palette_sources_changed();
        self.requery();
    }

    /// The session, the library, or the keymap changed: the palette gathers afresh at its next
    /// edit, and lets go of what it gathered until then.
    pub(super) fn palette_sources_changed(&self) {
        self.palette_index.borrow_mut().take();
    }

    /// Answers what the field holds now.
    fn requery(&self) {
        let text = self.command_palette.text();
        let items = {
            let mut index = self.palette_index.borrow_mut();
            index
                .get_or_insert_with(|| self.open_index())
                .query(&text, ANSWERS)
        };
        self.command_palette.set_items(&items);
        *self.results.borrow_mut() = items;
    }

    /// What the palette offers, gathered from the session, the library, and the keymap.
    fn open_index(&self) -> PaletteIndex {
        let browsing = self.browsing.borrow();
        let library = self.library.borrow();
        let keymap = self.keymap.borrow();
        self.palette.open(&Sources {
            session: browsing.session(),
            history: library.history(),
            bookmarks: library.bookmarks(),
            shortcuts: library.shortcuts(),
            keymap: &keymap,
        })
    }

    pub(super) fn palette_key(&self, key: OverlayKey) {
        match key {
            OverlayKey::Move(delta) => self.command_palette.move_selection(delta),
            OverlayKey::Accept { new_tab } => {
                if let Some(row) = self.command_palette.selected() {
                    self.choose(row, new_tab);
                }
            }
            OverlayKey::Previous => {}
            OverlayKey::Dismiss => {
                self.command_palette.hide();
                if let Some((window, _)) = self.current() {
                    self.settle(window);
                }
            }
        }
    }

    /// Runs what row `row` of the answer targets; `new_tab` (`⌘⏎`) opens a page in a new tab.
    fn choose(&self, row: usize, new_tab: bool) {
        let Some(target) = self
            .results
            .borrow()
            .get(row)
            .map(|item| item.target.clone())
        else {
            return;
        };
        match target {
            Target::Command(id) => {
                self.command_palette.hide();
                self.dispatch(&id, None);
            }
            Target::Tag(tag) => {
                self.command_palette
                    .set_text(&format!("#{} ", tag.as_str()));
                self.requery();
            }
            Target::Tab(tab) => {
                self.command_palette.hide();
                if let Some(window) = self.run(|browsing| browsing.activate(tab)) {
                    if let Some(ns_window) = self.window(window) {
                        ns_window.makeKeyAndOrderFront(None);
                    }
                    self.settle(window);
                }
            }
            Target::Url(url) | Target::Search(url) => self.go(url, new_tab),
            Target::Skill(id) => {
                self.command_palette.hide();
                self.ask_skill(&id);
            }
            Target::Ask(question) => {
                self.command_palette.hide();
                self.ask_question(question);
            }
        }
    }

    fn go(&self, url: Url, new_tab: bool) {
        self.command_palette.hide();
        if new_tab {
            self.open_in_new_tab(url);
        } else if let Some((window, _)) = self.current() {
            self.navigate(window, &Navigation::Load(url));
            self.settle(window);
        }
    }
}

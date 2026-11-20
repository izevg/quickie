//! The command palette: each edit re-queries the palette use case synchronously (it answers
//! from memory within a frame), and choosing a row runs its [`Target`]. A `?` row asks the
//! agent: nothing goes to the endpoint while the user types, only on `⏎`.

use quickie_application::{Navigation, PaletteItem, Sources, Target};
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
        self.requery();
    }

    /// Answers what the field holds now.
    fn requery(&self) {
        let text = self.command_palette.text();
        let items = self.answer(&text);
        self.command_palette.set_items(&items);
        *self.results.borrow_mut() = items;
    }

    /// The palette use case's answer to `text`, from the session, the library, and the keymap.
    fn answer(&self, text: &str) -> Vec<PaletteItem> {
        let browsing = self.browsing.borrow();
        let library = self.library.borrow();
        let keymap = self.keymap.borrow();
        let sources = Sources {
            session: browsing.session(),
            history: library.history(),
            bookmarks: library.bookmarks(),
            shortcuts: library.shortcuts(),
            keymap: &keymap,
        };
        self.palette.query(text, ANSWERS, &sources)
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

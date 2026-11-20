//! The command palette: showing it gathers what it offers once ([`Palette::open`]), and each
//! edit only ranks that again, synchronously, within a frame. When the session or the library
//! changes while it shows, the next edit gathers afresh. Choosing a row runs its [`Target`]. A
//! `?` row asks the agent: nothing goes to the endpoint while the user types, only on `⏎`.
//!
//! The address overlay answers from the same index, and runs what it chose here too.
//!
//! [`Palette::open`]: quickie_application::Palette::open

use quickie_application::{Navigation, PaletteIndex, Sources, Target};
use quickie_domain_browsing::{Focus, JarChoice};
use url::Url;

use super::Browser;
use crate::Addons;
use crate::input::keystroke::OverlayKey;

/// How many answers a query asks for; the list shows eight and scrolls the rest.
const ANSWERS: usize = 50;

/// Where a chosen page opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Opening {
    /// `⏎`: in the current tab.
    Here,
    /// `⌘⏎`: in a new tab.
    NewTab,
    /// `⌥⏎`: in a new tab in a new throwaway jar.
    Throwaway,
}

impl Opening {
    /// The opening an accepting key asks for.
    pub(super) const fn of(new_tab: bool, throwaway: bool) -> Self {
        match (new_tab, throwaway) {
            (_, true) => Self::Throwaway,
            (true, false) => Self::NewTab,
            (false, false) => Self::Here,
        }
    }
}

impl Browser {
    /// Hands the palette's edits and clicks to the browser, and the address overlay's, which
    /// answers from the same index.
    pub(super) fn wire_palette(&self) {
        self.wire_address();
        let this = self.this.clone();
        self.command_palette.on_change(move || {
            if let Some(browser) = this.upgrade() {
                browser.requery();
            }
        });
        let this = self.this.clone();
        self.command_palette.on_click(move |row| {
            if let Some(browser) = this.upgrade() {
                browser.choose(row, Opening::Here);
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

    /// Runs `ask` on the palette's index, gathering it first when nothing is gathered.
    pub(super) fn with_index<T>(&self, ask: impl FnOnce(&mut PaletteIndex) -> T) -> T {
        let mut index = self.palette_index.borrow_mut();
        ask(index.get_or_insert_with(|| self.open_index()))
    }

    /// Answers what the field holds now.
    fn requery(&self) {
        let text = self.command_palette.text();
        let items = self.with_index(|index| index.query(&text, ANSWERS));
        self.command_palette.set_items(&items);
        *self.results.borrow_mut() = items;
    }

    /// What the palette offers, gathered from the session, the library, and the keymap.
    fn open_index(&self) -> PaletteIndex {
        let browsing = self.browsing.borrow();
        let library = self.library.borrow();
        let keymap = self.keymap.borrow();
        let extensions = self
            .extensions
            .borrow()
            .as_ref()
            .map_or_default(Addons::offers);
        self.palette.open(&Sources {
            session: browsing.session(),
            history: library.history(),
            bookmarks: library.bookmarks(),
            shortcuts: library.shortcuts(),
            keymap: &keymap,
            extensions: &extensions,
        })
    }

    pub(super) fn palette_key(&self, key: OverlayKey) {
        match key {
            OverlayKey::Move(delta) => self.command_palette.move_selection(delta),
            OverlayKey::Accept { new_tab, throwaway } => {
                if let Some(row) = self.command_palette.selected() {
                    self.choose(row, Opening::of(new_tab, throwaway));
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

    /// Runs what row `row` of the answer targets; a page opens as `opening` says.
    fn choose(&self, row: usize, opening: Opening) {
        let Some(target) = self
            .results
            .borrow()
            .get(row)
            .map(|item| item.target.clone())
        else {
            return;
        };
        if let Target::Tag(tag) = target {
            self.command_palette
                .set_text(&format!("#{} ", tag.as_str()));
            self.requery();
        } else {
            self.run_target(target, opening);
        }
    }

    /// Runs what a palette or address row chose, closing both: switch to a tab, open a page
    /// as `opening` says, run a command or an extension, install one, or ask the agent. A tag
    /// narrows the palette instead, and nothing else chooses one.
    pub(super) fn run_target(&self, target: Target, opening: Opening) {
        self.command_palette.hide();
        self.address.hide();
        match target {
            Target::Command(id) => self.dispatch(&id, None),
            Target::Tag(_) => {}
            Target::Tab(tab) => {
                if let Some(window) = self.run(|browsing| browsing.activate(tab)) {
                    if let Some(ns_window) = self.window(window) {
                        ns_window.makeKeyAndOrderFront(None);
                    }
                    self.settle(window);
                }
            }
            Target::Url(url) | Target::Search(url) => self.go(url, opening),
            Target::Skill(id) => self.ask_skill(&id),
            Target::Ask(question) => self.ask_question(question),
            Target::InstallExtension(page) => self.install_from_store(&page),
            Target::Extension(trigger) => self.run_extension(&trigger),
        }
    }

    fn go(&self, url: Url, opening: Opening) {
        let Some((window, _)) = self.current() else {
            return;
        };
        match opening {
            Opening::Here => self.navigate(window, &Navigation::Load(url)),
            Opening::NewTab => self.open_in_new_tab(url),
            Opening::Throwaway => {
                self.run(|browsing| {
                    browsing.open(window, url, JarChoice::NewThrowaway, Focus::Foreground)
                });
            }
        }
        self.settle(window);
    }
}

#[cfg(test)]
mod tests {
    use super::Opening;

    #[test]
    fn the_accepting_key_picks_where_a_page_opens() {
        assert_eq!(Opening::of(false, false), Opening::Here);
        assert_eq!(Opening::of(true, false), Opening::NewTab);
        assert_eq!(Opening::of(false, true), Opening::Throwaway);
        assert_eq!(Opening::of(true, true), Opening::Throwaway);
    }
}

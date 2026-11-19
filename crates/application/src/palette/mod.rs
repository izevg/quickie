//! The palette query: what the user typed, answered from the open tabs, the library, and the
//! command catalog.
//!
//! The answer is computed whole, synchronously, from data already in memory: ranking ten
//! thousand candidates fits the palette's first-frame budget (`docs/architecture.md`, §5).

mod item;
mod offers;
#[cfg(test)]
mod tests;

use quickie_domain_browsing::Session;
use quickie_domain_commands::{Catalog, Keymap};
use quickie_domain_library::{Bookmarks, History, SearchShortcuts};
use quickie_domain_palette::{Mode, Query, rank};

pub use item::{PaletteItem, Target};
use offers::Offers;

use crate::Clock;

/// What the palette draws its candidates from, borrowed for one query.
#[derive(Clone, Copy, Debug)]
pub struct Sources<'a> {
    /// The open tabs.
    pub session: &'a Session,
    /// The visited pages, which also give tabs and bookmarks their frecency.
    pub history: &'a History,
    /// The saved pages and their tags.
    pub bookmarks: &'a Bookmarks,
    /// The keyword searches.
    pub shortcuts: &'a SearchShortcuts,
    /// The keys bound to each command.
    pub keymap: &'a Keymap,
}

/// The palette query use case.
#[derive(Debug)]
pub struct Palette<C> {
    catalog: Catalog,
    clock: C,
}

impl<C: Clock> Palette<C> {
    /// A palette over the commands of `catalog`, ranking by frecency at the `clock`'s now.
    #[must_use]
    pub fn new(catalog: Catalog, clock: C) -> Self {
        Self { catalog, clock }
    }

    /// The best `limit` answers to the palette input `text`, best first. The first character
    /// picks the mode (`>` commands, `#` tags, `!` keyword searches, none for tabs, bookmarks,
    /// and history); only that mode's candidates are gathered.
    #[must_use]
    pub fn query(&self, text: &str, limit: usize, sources: &Sources<'_>) -> Vec<PaletteItem> {
        let query = Query::parse(text);
        let offers = match query.mode() {
            Mode::GoTo => Offers::go_to(sources),
            Mode::Commands => Offers::commands(&self.catalog, sources.keymap),
            Mode::Tags => Offers::tags(sources.bookmarks),
            Mode::SearchShortcut => {
                let terms = query.keyword_search().map_or("", |search| search.terms);
                Offers::shortcuts(sources.shortcuts, terms)
            }
            // TODO: `?` goes to the agent (phase P4), which answers in its own panel, not in
            // a list. `@` lists the current page's headings, which only the engine can read.
            Mode::Agent | Mode::Headings => return Vec::new(),
        };
        rank(&query, &offers.candidates, self.clock.now(), limit)
            .into_iter()
            .filter_map(|ranked| offers.item(ranked))
            .collect()
    }
}

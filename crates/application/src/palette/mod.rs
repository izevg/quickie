//! The palette query: what the user typed, answered from the open tabs, the library, the
//! command catalog, and the agent's skills.
//!
//! The answer is computed whole, synchronously, from data already in memory: ranking ten
//! thousand candidates fits the palette's first-frame budget (`docs/architecture.md`, §5).

mod item;
mod offers;
#[cfg(test)]
mod tests;

use quickie_domain_agents::{Question, Skill};
use quickie_domain_browsing::Session;
use quickie_domain_commands::{Catalog, Keymap};
use quickie_domain_library::{Bookmarks, History, SearchShortcuts};
use quickie_domain_palette::{Kind, Mode, Query, rank};

pub use item::{PaletteItem, Target};
use offers::Offers;

use crate::Clock;

/// The start of the row that asks the agent the typed question.
const ASK_TITLE: &str = "Ask about this page: ";

/// What the row that asks the agent says it does.
const ASK_DETAIL: &str = "The agent reads this page and answers; nothing is sent before ⏎";

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
    skills: Vec<Skill>,
    clock: C,
}

impl<C: Clock> Palette<C> {
    /// A palette over the commands of `catalog` and the built-in skills, ranking by frecency
    /// at the `clock`'s now.
    #[must_use]
    pub fn new(catalog: Catalog, clock: C) -> Self {
        Self {
            catalog,
            skills: Skill::builtin(),
            clock,
        }
    }

    /// The best `limit` answers to the palette input `text`, best first. The first character
    /// picks the mode (`>` commands, `#` tags, `!` keyword searches, `?` the agent, none for
    /// tabs, bookmarks, and history); only that mode's candidates are gathered.
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
            Mode::Agent => return self.agent(&query, limit),
            // TODO: `@` lists the current page's headings, which only the engine can read.
            Mode::Headings => return Vec::new(),
        };
        self.ranked(&query, &offers, limit)
    }

    /// `?`: the skills the text matches (all of them for no text), then the row that asks the
    /// text itself as a question, which nothing ranks above or leaves out.
    fn agent(&self, query: &Query, limit: usize) -> Vec<PaletteItem> {
        let question = Question::try_from(query.text()).ok();
        let room = limit.saturating_sub(usize::from(question.is_some()));
        let mut items = self.ranked(query, &Offers::skills(&self.skills), room);
        if let Some(question) = question.filter(|_| limit > 0) {
            items.push(PaletteItem {
                title: format!("{ASK_TITLE}{}", question.as_str()),
                target: Target::Ask(question),
                kind: Kind::Question,
                detail: ASK_DETAIL.to_owned(),
                glyphs: Vec::new(),
                title_matches: Vec::new(),
                detail_matches: Vec::new(),
            });
        }
        items
    }

    fn ranked(&self, query: &Query, offers: &Offers, limit: usize) -> Vec<PaletteItem> {
        rank(query, &offers.candidates, self.clock.now(), limit)
            .into_iter()
            .filter_map(|ranked| offers.item(ranked))
            .collect()
    }
}

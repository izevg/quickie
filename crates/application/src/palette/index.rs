//! The open palette: every candidate gathered once, ranked again on each keystroke.

mod address;
mod gather;
mod shelf;

use quickie_domain_agents::{Question, Skill};
use quickie_domain_browsing::Timestamp;
use quickie_domain_commands::Catalog;
use quickie_domain_library::SearchShortcuts;
use quickie_domain_palette::{Kind, Mode, Query, Ranker};

use super::{ASK_DETAIL, ASK_TITLE, PaletteItem, Sources, Target};
pub use address::{AddressSuggestions, InlineCompletion, Suggestion, SuggestionKind};
use shelf::Shelf;

/// What the palette offers while it is open: the candidates of every mode, gathered and scored
/// once by [`Palette::open`](super::Palette::open), with what ranking them needs. It owns its
/// text, so it outlives the borrow of its sources; when they change, open a new one.
#[derive(Debug)]
pub struct PaletteIndex {
    ranker: Ranker,
    go_to: Shelf,
    commands: Shelf,
    tags: Shelf,
    skills: Shelf,
    shortcuts: SearchShortcuts,
}

impl PaletteIndex {
    pub(super) fn gather(
        catalog: &Catalog,
        skills: &[Skill],
        sources: &Sources<'_>,
        now: Timestamp,
    ) -> Self {
        Self {
            ranker: Ranker::default(),
            go_to: gather::go_to(sources, now),
            commands: gather::commands(catalog, sources.keymap),
            tags: gather::tags(sources.bookmarks),
            skills: gather::skills(skills),
            shortcuts: sources.shortcuts.clone(),
        }
    }

    /// The best `limit` answers to the palette input `text`, best first. The first character
    /// picks the mode (`>` commands, `#` tags, `!` keyword searches, `?` the agent, none for
    /// tabs, bookmarks, and history); only that mode's candidates are ranked, and only the
    /// answers kept become rows.
    pub fn query(&mut self, text: &str, limit: usize) -> Vec<PaletteItem> {
        let query = Query::parse(text);
        let shortcuts;
        let offered = match query.mode() {
            Mode::GoTo => &self.go_to,
            Mode::Commands => &self.commands,
            Mode::Tags => &self.tags,
            Mode::SearchShortcut => {
                let terms = query.keyword_search().map_or("", |search| search.terms);
                shortcuts = gather::shortcuts(&self.shortcuts, terms);
                &shortcuts
            }
            Mode::Agent => return self.agent(&query, limit),
            // TODO: `@` lists the current page's headings, which only the engine can read.
            Mode::Headings => return Vec::new(),
        };
        answer(&mut self.ranker, &query, offered, limit)
    }

    /// `?`: the skills the text matches (all of them for no text), then the row that asks the
    /// text itself as a question, which nothing ranks above or leaves out.
    fn agent(&mut self, query: &Query, limit: usize) -> Vec<PaletteItem> {
        let question = Question::try_from(query.text()).ok();
        let room = limit.saturating_sub(usize::from(question.is_some()));
        let mut items = answer(&mut self.ranker, query, &self.skills, room);
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
}

/// The rows of the best `limit` candidates on `shelf`.
fn answer(ranker: &mut Ranker, query: &Query, shelf: &Shelf, limit: usize) -> Vec<PaletteItem> {
    ranker
        .rank(query, shelf.candidates(), limit)
        .into_iter()
        .filter_map(|ranked| shelf.item(ranked))
        .collect()
}

//! The open palette: every candidate gathered once, ranked again on each keystroke.

mod address;
mod gather;
mod shelf;

use quickie_domain_agents::{Question, Skill};
use quickie_domain_browsing::Timestamp;
use quickie_domain_commands::Catalog;
use quickie_domain_library::SearchShortcuts;
use quickie_domain_palette::{Kind, Mode, Query, Ranker};
use quickie_domain_settings::ExtensionId;
use url::Url;

use super::{ASK_DETAIL, ASK_TITLE, INSTALL_TITLE, PageFacts, PaletteItem, Sources, Target};
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
            commands: gather::commands(catalog, sources.keymap, sources.extensions),
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
            Mode::GoTo => return self.go_to(text, &query, limit),
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

    /// Answers the dedicated Go To field, where punctuation is part of the page search.
    pub fn pages(&mut self, text: &str, limit: usize) -> Vec<PaletteItem> {
        answer(&mut self.ranker, &Query::go_to(text), &self.go_to, limit)
    }

    /// No prefix: tabs, bookmarks, and history; a Chrome Web Store page pasted whole first
    /// offers to install its extension, which nothing ranks above.
    fn go_to(&mut self, text: &str, query: &Query, limit: usize) -> Vec<PaletteItem> {
        let store = Url::parse(text.trim())
            .ok()
            .filter(|url| ExtensionId::from_store_url(url).is_some())
            .filter(|_| limit > 0);
        let room = limit.saturating_sub(usize::from(store.is_some()));
        let mut items = Vec::with_capacity(limit);
        if let Some(url) = store {
            items.push(PaletteItem {
                title: INSTALL_TITLE.to_owned(),
                detail: url.to_string(),
                target: Target::InstallExtension(url),
                kind: Kind::Command,
                glyphs: Vec::new(),
                title_matches: Vec::new(),
                detail_matches: Vec::new(),
                facts: PageFacts::default(),
            });
        }
        items.extend(answer(&mut self.ranker, query, &self.go_to, room));
        items
    }

    /// `?`: matching skills and explicit choices to ask about the page or open a link.
    fn agent(&mut self, query: &Query, limit: usize) -> Vec<PaletteItem> {
        let question = Question::try_from(query.text()).ok();
        let room = limit.saturating_sub(if question.is_some() { 2 } else { 0 });
        let mut items = answer(&mut self.ranker, query, &self.skills, room);
        if let Some(question) = question.filter(|_| limit > 0) {
            let title = format!("{ASK_TITLE}{}", question.as_str());
            let action = format!("Act on this page: {}", question.as_str());
            items.push(agent_item(title, ASK_DETAIL, Target::Ask(question.clone())));
            if items.len() < limit {
                items.push(agent_item(
                    action,
                    "Choose a link and open it in an isolated Agent window",
                    Target::Act(question),
                ));
            }
        }
        items
    }
}

fn agent_item(title: String, detail: &str, target: Target) -> PaletteItem {
    PaletteItem {
        target,
        kind: Kind::Question,
        title,
        detail: detail.to_owned(),
        glyphs: Vec::new(),
        title_matches: Vec::new(),
        detail_matches: Vec::new(),
        facts: PageFacts::default(),
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

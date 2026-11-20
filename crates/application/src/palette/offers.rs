//! The candidates each palette mode draws from, gathered from the session, the library, the
//! keymap, and the agent's skills.

use std::collections::HashSet;

use quickie_domain_agents::Skill;
use quickie_domain_commands::{Catalog, Keymap};
use quickie_domain_library::{Bookmarks, History, HistoryEntry, SearchShortcuts};
use quickie_domain_palette::{Candidate, Frecency, Kind, Ranked};
use url::Url;

use super::{PaletteItem, Sources, Target};

/// Candidates under their position as key, and beside each what the ranking does not carry.
#[derive(Debug, Default)]
pub(super) struct Offers {
    pub(super) candidates: Vec<Candidate<usize>>,
    extras: Vec<(Target, Vec<String>)>,
}

impl Offers {
    fn push(&mut self, target: Target, kind: Kind, title: String, detail: String) {
        self.push_with(target, kind, title, detail, Frecency::default(), Vec::new());
    }

    fn push_with(
        &mut self,
        target: Target,
        kind: Kind,
        title: String,
        detail: String,
        visits: Frecency,
        glyphs: Vec<String>,
    ) {
        self.candidates.push(Candidate {
            key: self.candidates.len(),
            kind,
            title,
            detail,
            visits,
        });
        self.extras.push((target, glyphs));
    }

    /// The row of a ranked candidate.
    pub(super) fn item(&self, ranked: Ranked<usize>) -> Option<PaletteItem> {
        let candidate = self.candidates.get(ranked.key)?;
        let (target, glyphs) = self.extras.get(ranked.key)?;
        Some(PaletteItem {
            target: target.clone(),
            kind: candidate.kind,
            title: candidate.title.clone(),
            detail: candidate.detail.clone(),
            glyphs: glyphs.clone(),
            title_matches: ranked.title_matches,
            detail_matches: ranked.detail_matches,
        })
    }

    /// No prefix: open tabs, then bookmarks, then history. A page appears once, as the best of
    /// what it is: switching to its tab beats opening its bookmark beats opening it from history.
    pub(super) fn go_to(sources: &Sources<'_>) -> Self {
        let mut offers = Self::default();
        let mut shown = HashSet::new();
        for window in sources.session.windows() {
            let Ok(tabs) = sources.session.tabs(window) else {
                continue;
            };
            for tab in tabs {
                shown.insert(page(&tab.url));
                offers.push_with(
                    Target::Tab(tab.id),
                    Kind::OpenTab,
                    title_or_url(&tab.title, &tab.url),
                    tab.url.to_string(),
                    frecency_of(sources.history, &tab.url),
                    Vec::new(),
                );
            }
        }
        for bookmark in sources.bookmarks.all() {
            if shown.insert(bookmark.url.clone()) {
                offers.push_with(
                    Target::Url(bookmark.url.clone()),
                    Kind::Bookmark,
                    title_or_url(&bookmark.title, &bookmark.url),
                    bookmark.url.to_string(),
                    frecency_of(sources.history, &bookmark.url),
                    Vec::new(),
                );
            }
        }
        for entry in sources.history.entries() {
            if !shown.contains(&entry.url) {
                offers.push_with(
                    Target::Url(entry.url.clone()),
                    Kind::History,
                    title_or_url(&entry.title, &entry.url),
                    entry.url.to_string(),
                    frecency(entry),
                    Vec::new(),
                );
            }
        }
        offers
    }

    /// `>`: every command, its category as detail, its keys as glyphs.
    pub(super) fn commands(catalog: &Catalog, keymap: &Keymap) -> Self {
        let mut offers = Self::default();
        for command in catalog.iter() {
            let glyphs = keymap
                .bindings_for(&command.id)
                .map(|binding| binding.chord.glyphs(keymap.platform()))
                .collect();
            offers.push_with(
                Target::Command(command.id.clone()),
                Kind::Command,
                command.title.to_owned(),
                command.category.label().to_owned(),
                Frecency::default(),
                glyphs,
            );
        }
        offers
    }

    /// `#`: every tag in use, with how many pages carry it.
    pub(super) fn tags(bookmarks: &Bookmarks) -> Self {
        let mut offers = Self::default();
        for (tag, pages) in bookmarks.tags() {
            let detail = if pages == 1 {
                "1 page".to_owned()
            } else {
                format!("{pages} pages")
            };
            offers.push(
                Target::Tag(tag.clone()),
                Kind::Tag,
                tag.as_str().to_owned(),
                detail,
            );
        }
        offers
    }

    /// `?`: every skill of the agent, what it does as detail.
    pub(super) fn skills(skills: &[Skill]) -> Self {
        let mut offers = Self::default();
        for skill in skills {
            offers.push(
                Target::Skill(skill.id().clone()),
                Kind::Skill,
                skill.name().to_owned(),
                skill.description().to_owned(),
            );
        }
        offers
    }

    /// `!`: every keyword, each aimed at its search for `terms`. A template the terms make no
    /// URL of is left out, since choosing it could open nothing.
    pub(super) fn shortcuts(shortcuts: &SearchShortcuts, terms: &str) -> Self {
        let mut offers = Self::default();
        for (keyword, template) in shortcuts.iter() {
            if let Some(search) = template.fill(terms) {
                offers.push(
                    Target::Search(search),
                    Kind::SearchShortcut,
                    keyword.as_str().to_owned(),
                    template.as_str().to_owned(),
                );
            }
        }
        offers
    }
}

/// The URL that names a page in the library: without its fragment.
fn page(url: &Url) -> Url {
    let mut page = url.clone();
    page.set_fragment(None);
    page
}

/// A row needs a main line even before the engine reported a title.
fn title_or_url(title: &str, url: &Url) -> String {
    if title.is_empty() {
        url.to_string()
    } else {
        title.to_owned()
    }
}

/// How often and how recently the page `url` names was visited; nothing for a page never
/// visited.
fn frecency_of(history: &History, url: &Url) -> Frecency {
    history.get(url).map_or_default(frecency)
}

fn frecency(entry: &HistoryEntry) -> Frecency {
    let count = u32::try_from(entry.visit_count).unwrap_or(u32::MAX);
    Frecency::sampled(count, &entry.recent_visits).unwrap_or_default()
}

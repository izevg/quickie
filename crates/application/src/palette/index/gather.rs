//! The candidates each palette mode draws from, gathered once when the palette opens from the
//! session, the library, the keymap, and the agent's skills.

use std::collections::HashSet;

use quickie_domain_agents::Skill;
use quickie_domain_browsing::Timestamp;
use quickie_domain_commands::{Catalog, Keymap};
use quickie_domain_library::{Bookmarks, History, HistoryEntry, SearchShortcuts};
use quickie_domain_palette::{Frecency, Kind};
use url::Url;

use super::shelf::{Aim, Shelf};
use crate::ExtensionOffer;
use crate::palette::{PageFacts, Sources, Target};

/// The second line of an extension's row: where it comes from, as a category would say.
const EXTENSION_DETAIL: &str = "Extension";

/// No prefix: open tabs, then bookmarks, then history. A page appears once, as the best of what
/// it is: switching to its tab beats opening its bookmark beats opening it from history. Each
/// is scored by the frecency of its page at `now`.
pub(super) fn go_to(sources: &Sources<'_>, now: Timestamp) -> Shelf {
    let history = sources.history;
    let mut shelf = Shelf::default();
    let mut shown: HashSet<Url> = HashSet::new();
    for window in sources.session.windows() {
        let Ok(tabs) = sources.session.tabs(window) else {
            continue;
        };
        for tab in tabs {
            let page = page(&tab.url);
            let facts = PageFacts {
                slot: sources.session.slot_of(tab.id),
                ..facts_of(history, &page)
            };
            shelf.push_page(
                Aim::at(Target::Tab(tab.id)),
                Kind::OpenTab,
                &tab.title,
                tab.url.as_str(),
                frecency_of(history, &page, now),
                facts,
            );
            shown.insert(page);
        }
    }
    for bookmark in sources.bookmarks.all() {
        if shown.insert(bookmark.url.clone()) {
            shelf.push_page(
                Aim::Page,
                Kind::Bookmark,
                &bookmark.title,
                bookmark.url.as_str(),
                frecency_of(history, &bookmark.url, now),
                facts_of(history, &bookmark.url),
            );
        }
    }
    for entry in history.entries() {
        if !shown.contains(&entry.url) {
            shelf.push_page(
                Aim::Page,
                Kind::History,
                &entry.title,
                entry.url.as_str(),
                frecency(entry, now),
                PageFacts::of(entry),
            );
        }
    }
    shelf
}

/// `>`: every command, its category as detail, its keys as glyphs.
pub(super) fn commands(catalog: &Catalog, keymap: &Keymap, extensions: &[ExtensionOffer]) -> Shelf {
    let mut shelf = Shelf::default();
    for command in catalog.iter() {
        let glyphs = keymap
            .bindings_for(&command.id)
            .map(|binding| binding.chord.glyphs(keymap.platform()))
            .collect();
        shelf.push(
            Aim::Fixed(Box::new((Target::Command(command.id.clone()), glyphs))),
            Kind::Command,
            command.title,
            command.category.label(),
            0,
        );
    }
    for extension in extensions {
        let glyphs = extension
            .stroke
            .map_or_default(|stroke| vec![stroke.glyphs(keymap.platform())]);
        shelf.push(
            Aim::Fixed(Box::new((
                Target::Extension(extension.trigger.clone()),
                glyphs,
            ))),
            Kind::Command,
            &extension.title,
            EXTENSION_DETAIL,
            0,
        );
    }
    shelf
}

/// `#`: every tag in use, with how many pages carry it.
pub(super) fn tags(bookmarks: &Bookmarks) -> Shelf {
    let mut shelf = Shelf::default();
    for (tag, pages) in bookmarks.tags() {
        let detail = if pages == 1 {
            "1 page".to_owned()
        } else {
            format!("{pages} pages")
        };
        shelf.push(
            Aim::at(Target::Tag(tag.clone())),
            Kind::Tag,
            tag.as_str(),
            &detail,
            0,
        );
    }
    shelf
}

/// `?`: every skill of the agent, what it does as detail.
pub(super) fn skills(skills: &[Skill]) -> Shelf {
    let mut shelf = Shelf::default();
    for skill in skills {
        shelf.push(
            Aim::at(Target::Skill(skill.id().clone())),
            Kind::Skill,
            skill.name(),
            skill.description(),
            0,
        );
    }
    shelf
}

/// `!`: every keyword, each aimed at its search for `terms`. A template the terms make no URL
/// of is left out, since choosing it could open nothing. Gathered per keystroke, since the
/// terms change the targets; there are only a handful.
pub(super) fn shortcuts(shortcuts: &SearchShortcuts, terms: &str) -> Shelf {
    let mut shelf = Shelf::default();
    for (keyword, template) in shortcuts.iter() {
        if let Some(search) = template.fill(terms) {
            shelf.push(
                Aim::at(Target::Search(search)),
                Kind::SearchShortcut,
                keyword.as_str(),
                template.as_str(),
                0,
            );
        }
    }
    shelf
}

/// The URL that names a page in the library: without its fragment.
fn page(url: &Url) -> Url {
    let mut page = url.clone();
    page.set_fragment(None);
    page
}

/// How often and how recently the page `url` names was visited, at `now`; nothing for a page
/// never visited.
fn frecency_of(history: &History, url: &Url, now: Timestamp) -> u64 {
    history.get(url).map_or(0, |entry| frecency(entry, now))
}

/// What history knows of the page `url` names; nothing for a page never visited.
fn facts_of(history: &History, url: &Url) -> PageFacts {
    history.get(url).map_or_default(PageFacts::of)
}

fn frecency(entry: &HistoryEntry, now: Timestamp) -> u64 {
    let count = u32::try_from(entry.visit_count).unwrap_or(u32::MAX);
    Frecency::sampled(count, &entry.recent_visits)
        .unwrap_or_default()
        .score(now)
}

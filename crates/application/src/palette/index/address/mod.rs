//! The address overlay's suggestions (`⌘L`), answered from the palette's go-to candidates: the
//! open tabs, the bookmarks, and history, gathered once when the overlay opens. Everything is
//! local; nothing typed leaves the browser before the user goes somewhere.
//!
//! For typed text the answer is an [`InlineCompletion`], when a known page's address starts
//! with it, and at most [`ROWS`] rows. The first row is what `⏎` does: the completed page, the
//! typed address, a `!keyword` search, or a search for the text. Then the pages whose address
//! starts with the text, by frecency with open tabs first; then the pages it fuzzily matches;
//! and last a search for the text, when the first row is no search.
//!
//! Before anything is typed, the overlay offers the page's own actions ([`PAGE_ACTIONS`]).

mod completion;
#[cfg(test)]
mod tests;

use std::ops::Range;

use quickie_domain_browsing::{Destination, SEARCH_ENGINE};
use quickie_domain_commands::CommandId;
use quickie_domain_library::SearchShortcuts;
use quickie_domain_palette::{Kind, Query, Ranked, Ranker};
use url::{Position, Url};

pub use completion::InlineCompletion;

use super::PaletteIndex;
use super::shelf::Shelf;
use crate::palette::{PaletteItem, Target};

/// At most this many rows answer typed text.
pub const ROWS: usize = 8;

/// What a suggestion is, for the words its row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SuggestionKind {
    /// Load an address: the typed one, or the completed one.
    GoTo,
    /// Search the default engine, or a `!keyword` shortcut's, for the text.
    Search,
    /// Switch to a tab that is open already, instead of opening the page twice.
    OpenTab,
    /// Open a saved page.
    Bookmark,
    /// Open a page visited before.
    History,
}

/// One row of the address overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// What choosing it does.
    pub target: Target,
    /// What it is.
    pub kind: SuggestionKind,
    /// The main line: a page title, `Go to …`, `Search … for “…”`.
    pub title: String,
    /// The URL it opens, for the second line.
    pub url: String,
    /// The characters of `url` that are its host, set apart.
    pub host: Range<u32>,
    /// Characters of `title` the text matched, ascending.
    pub title_matches: Vec<u32>,
    /// Characters of `url` the text matched, ascending.
    pub url_matches: Vec<u32>,
    /// What the field shows while the row is selected: the page's URL, or the typed text that
    /// a `Go to` or search row goes with.
    pub field: String,
}

/// The answer to typed text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressSuggestions {
    /// The completion to show in the field after the typed text.
    pub completion: Option<InlineCompletion>,
    /// The rows, the one `⏎` chooses first.
    pub rows: Vec<Suggestion>,
}

/// What the overlay offers for the page itself before anything is typed, in this order.
const PAGE_ACTIONS: [&str; 3] = [
    "quickie.page.copyUrl",
    "quickie.page.copyAsMarkdownLink",
    "quickie.shields.open",
];

impl PaletteIndex {
    /// The commands the address overlay offers for the page while it shows the page's URL
    /// unchanged (copy it, copy it as a Markdown link, its Shields), with their keys.
    #[must_use]
    pub fn page_actions(&self) -> Vec<PaletteItem> {
        PAGE_ACTIONS
            .iter()
            .filter_map(|&id| {
                let target = Target::Command(CommandId::try_from(id).ok()?);
                let key = self.commands.find_target(&target)?;
                self.commands.item(unranked(key))
            })
            .collect()
    }

    /// The address overlay's answer to `typed`. With `complete`, the best page whose address
    /// starts with the text offers an [`InlineCompletion`]; the overlay asks for none after a
    /// deletion or while an input method composes. Nothing answers blank text.
    pub fn suggest(&mut self, typed: &str, complete: bool) -> AddressSuggestions {
        let text = typed.trim();
        let Some(destination) = Destination::of(text) else {
            return AddressSuggestions::default();
        };
        let pages = &self.go_to;
        let starting = starting_with(&mut self.ranker, pages, text);
        let completion = starting
            .first()
            .filter(|_| complete && !typed.contains(char::is_whitespace))
            .and_then(|&(key, _)| completion::complete(typed, &page_url(pages, key)?));

        let mut shown = Vec::new();
        let first = match (&completion, shortcut(&self.shortcuts, text), &destination) {
            (Some(completion), _, _) => {
                go_to(pages, &completion.url, &completion.text(), &mut shown)
            }
            (None, Some(search), _) => search,
            (None, None, Destination::Address(url)) => go_to(pages, url, text, &mut shown),
            (None, None, Destination::Search(url)) => search_row(url, text),
        };
        let last = (first.kind != SuggestionKind::Search)
            .then(|| Destination::search(text))
            .flatten()
            .map(|search| search_row(search.url(), text));
        let room = ROWS - usize::from(last.is_some());

        let mut rows = vec![first];
        let starting = starting.into_iter().map(|(key, matched)| Ranked {
            key,
            score: 0,
            title_matches: Vec::new(),
            detail_matches: matched,
        });
        let fuzzy = self
            .ranker
            .rank(&Query::go_to(text), pages.candidates(), ROWS);
        for ranked in starting.chain(fuzzy) {
            if rows.len() >= room {
                break;
            }
            if !shown.contains(&ranked.key) {
                shown.push(ranked.key);
                rows.extend(page(pages, ranked));
            }
        }
        rows.extend(last);
        AddressSuggestions { completion, rows }
    }
}

/// The pages whose address starts with `text`, best first: by frecency, open tabs and bookmarks
/// boosted as in the palette. Each comes with the characters of its URL the text covers.
fn starting_with(ranker: &mut Ranker, shelf: &Shelf, text: &str) -> Vec<(usize, Vec<u32>)> {
    let candidates = shelf
        .candidates()
        .filter(|candidate| completion::matched(text, candidate.detail.as_str()).is_some());
    ranker
        .rank(&Query::go_to(""), candidates, ROWS)
        .into_iter()
        .filter_map(|ranked| {
            let matched = completion::matched(text, shelf.detail(ranked.key)?)?;
            Some((ranked.key, positions(matched)))
        })
        .collect()
}

/// The row that goes to `url`: the page's own row when a tab, a bookmark, or history has it
/// (an open tab is switched to, not opened twice), else `Go to <text>`.
fn go_to(shelf: &Shelf, url: &Url, text: &str, shown: &mut Vec<usize>) -> Suggestion {
    let known = shelf.find(url.as_str()).and_then(|key| {
        shown.push(key);
        page(shelf, unranked(key))
    });
    known.unwrap_or_else(|| Suggestion {
        target: Target::Url(url.clone()),
        kind: SuggestionKind::GoTo,
        title: format!("Go to {text}"),
        url: url.as_str().to_owned(),
        host: host(url),
        title_matches: Vec::new(),
        url_matches: Vec::new(),
        field: text.to_owned(),
    })
}

/// The row of the page under `ranked.key`, with what the text matched in it.
fn page(shelf: &Shelf, ranked: Ranked<usize>) -> Option<Suggestion> {
    let item = shelf.item(ranked)?;
    let kind = match item.kind {
        Kind::OpenTab => SuggestionKind::OpenTab,
        Kind::Bookmark => SuggestionKind::Bookmark,
        _ => SuggestionKind::History,
    };
    let host = host(&Url::parse(&item.detail).ok()?);
    Some(Suggestion {
        target: item.target,
        kind,
        title: item.title,
        host,
        title_matches: item.title_matches,
        url_matches: item.detail_matches,
        field: item.detail.clone(),
        url: item.detail,
    })
}

/// A page by its key, nothing matched.
const fn unranked(key: usize) -> Ranked<usize> {
    Ranked {
        key,
        score: 0,
        title_matches: Vec::new(),
        detail_matches: Vec::new(),
    }
}

/// The URL of the page under `key`.
fn page_url(shelf: &Shelf, key: usize) -> Option<Url> {
    Url::parse(shelf.detail(key)?).ok()
}

/// `!keyword terms`: the shortcut's search for the terms, when the keyword is known and there
/// are terms.
fn shortcut(shortcuts: &SearchShortcuts, text: &str) -> Option<Suggestion> {
    let (keyword, terms) = text.strip_prefix('!')?.split_once(char::is_whitespace)?;
    let terms = terms.trim_start();
    if terms.is_empty() {
        return None;
    }
    let url = shortcuts.expand(keyword, terms)?;
    let site = url.host_str()?.trim_start_matches("www.").to_owned();
    Some(Suggestion {
        title: format!("Search {site} for “{terms}”"),
        ..search_row(&url, text)
    })
}

/// `Search DuckDuckGo for “text”`, going to `url`.
fn search_row(url: &Url, text: &str) -> Suggestion {
    Suggestion {
        target: Target::Search(url.clone()),
        kind: SuggestionKind::Search,
        title: format!("Search {SEARCH_ENGINE} for “{text}”"),
        url: url.as_str().to_owned(),
        host: host(url),
        title_matches: Vec::new(),
        url_matches: Vec::new(),
        field: text.to_owned(),
    }
}

/// The characters of `url` that are its host. A URL is written in ASCII (its host in Punycode,
/// the rest percent-encoded), so its bytes are its characters.
fn host(url: &Url) -> Range<u32> {
    let start = url[..Position::BeforeHost].len();
    let end = url[..Position::AfterHost].len();
    let [start, end] = [start, end].map(|at| u32::try_from(at).unwrap_or(u32::MAX));
    start..end
}

/// The character positions of a byte range of a URL (ASCII, see [`host`]).
fn positions(range: Range<usize>) -> Vec<u32> {
    range.filter_map(|at| u32::try_from(at).ok()).collect()
}

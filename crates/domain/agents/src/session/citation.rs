//! Citations: quotes the answer takes from the page, checked against the page so that each one
//! can be found there.

use crate::PageText;

/// How many words of a quote its anchor keeps. Find in page matches a short run of words more
/// reliably than a long passage that crosses links, line breaks, and styling.
const ANCHOR_WORDS: usize = 12;

/// Quotation marks a model tends to wrap a quote in; they are not part of it.
const QUOTE_MARKS: &[char] = &['"', '\'', '“', '”', '‘', '’', '«', '»'];

/// A quote from the page that the page really contains. The answer panel shows it as a chip;
/// choosing the chip finds `anchor` in the page, scrolls to it, and highlights it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Citation {
    /// The quote, as the answer gave it.
    pub quote: String,
    /// The text to find in the page: the quote's first words, whitespace collapsed to single
    /// spaces.
    pub anchor: String,
}

/// The pages' text, normalized once so each quote is looked up cheaply: whitespace collapsed to
/// single spaces, lowercase.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Sources(Vec<String>);

impl Sources {
    pub(super) fn of(pages: &[PageText]) -> Self {
        Self(pages.iter().map(|page| normalize(page.text())).collect())
    }

    /// The citation for `quote` when a page contains it, ignoring case, whitespace, and the
    /// quotation marks around it; `None` for a quote no page contains, which is no citation.
    pub(super) fn cite(&self, quote: &str) -> Option<Citation> {
        let quote = quote.trim_matches(|c: char| c.is_whitespace() || QUOTE_MARKS.contains(&c));
        let wanted = normalize(quote);
        if wanted.is_empty() || !self.0.iter().any(|page| page.contains(&wanted)) {
            return None;
        }
        let anchor = quote
            .split_whitespace()
            .take(ANCHOR_WORDS)
            .collect::<Vec<_>>()
            .join(" ");
        Some(Citation {
            quote: quote.to_owned(),
            anchor,
        })
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

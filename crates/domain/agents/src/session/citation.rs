//! Citations: quotes the answer takes from the pages, checked against the text the model was
//! actually sent, so that each one can be found there.

/// The fewest words a quote needs to count as a citation: a word or two occurs on almost any
/// page, so it proves nothing.
const MIN_QUOTE_WORDS: usize = 3;

/// The fewest characters a quote needs to count as a citation, for the same reason.
const MIN_QUOTE_CHARS: usize = 15;

/// Quotation marks a model tends to wrap a quote in; they are not part of it.
const QUOTE_MARKS: &[char] = &['"', '\'', '“', '”', '‘', '’', '«', '»'];

/// A quote from a page that the page really contains. The answer panel shows it as a chip;
/// choosing the chip finds `anchor` in the page, scrolls to it, and highlights it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Citation {
    /// The quote, as the answer gave it.
    pub quote: String,
    /// The text to find in the page: the shortest start of the quote, at least three words,
    /// that occurs only once in the pages, so finding it lands on the quoted passage and not
    /// on an earlier one; the whole quote when no shorter start is unique. Whitespace is
    /// collapsed to single spaces.
    pub anchor: String,
}

/// The text of the pages as the model was sent it, normalized once so each quote is looked up
/// cheaply: whitespace collapsed to single spaces, lowercase.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Sources(Vec<String>);

impl Sources {
    pub(super) fn of(sent: &[String]) -> Self {
        Self(sent.iter().map(|text| normalize(text)).collect())
    }

    /// The citation for `quote` when a page as sent contains it, ignoring case, whitespace, and
    /// the quotation marks around it. `None` for a quote too short to prove anything, or one no
    /// page contains: that is no citation.
    pub(super) fn cite(&self, quote: &str) -> Option<Citation> {
        let quote = quote.trim_matches(|c: char| c.is_whitespace() || QUOTE_MARKS.contains(&c));
        let words: Vec<&str> = quote.split_whitespace().collect();
        if words.len() < MIN_QUOTE_WORDS || quote.chars().count() < MIN_QUOTE_CHARS {
            return None;
        }
        if self.occurrences(&words.join(" ").to_lowercase()) == 0 {
            return None;
        }
        let anchor = (MIN_QUOTE_WORDS..words.len())
            .map(|len| words[..len].join(" "))
            .find(|start| self.occurrences(&start.to_lowercase()) == 1)
            .unwrap_or_else(|| words.join(" "));
        Some(Citation {
            quote: quote.to_owned(),
            anchor,
        })
    }

    /// How often `wanted`, normalized, occurs in all the pages together.
    fn occurrences(&self, wanted: &str) -> usize {
        self.0.iter().map(|page| page.matches(wanted).count()).sum()
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

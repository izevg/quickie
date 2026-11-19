//! The page as the agent reads it: its text, cleaned so it cannot forge the fence it travels
//! in, and cut to a token budget.

use url::Url;

/// Opens and closes the block a page's content travels in. Page content never contains these
/// two characters, as [`clean`] replaces them, so a page cannot close its block early and write
/// outside it.
pub(super) const FENCE_OPEN: char = '⟦';
/// See [`FENCE_OPEN`].
pub(super) const FENCE_CLOSE: char = '⟧';

/// The longest title read, in characters; a page cannot spend the budget on its title.
const TITLE_MAX_CHARS: usize = 200;

/// A page's text as the engine extracted it. It is untrusted: whoever wrote the page wrote it,
/// and it may address the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageText {
    url: Url,
    title: String,
    text: String,
}

impl PageText {
    /// The text of the page at `url`, titled `title`.
    #[must_use]
    pub fn new(url: Url, title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            url,
            title: title.into(),
            text: text.into(),
        }
    }

    /// Where the page is.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The page's title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The page's text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The title on one line, cleaned and cut to [`TITLE_MAX_CHARS`].
    pub(super) fn clean_title(&self) -> String {
        let line = clean(&self.title)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        line.chars().take(TITLE_MAX_CHARS).collect()
    }

    /// The text cleaned and cut to `budget`.
    pub(super) fn excerpt(&self, budget: TokenBudget) -> Excerpt {
        let mut text = clean(&self.text);
        match text.char_indices().nth(budget.chars()) {
            Some((at, _)) => {
                text.truncate(at);
                Excerpt { text, cut: true }
            }
            None => Excerpt { text, cut: false },
        }
    }
}

/// A page's text as sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Excerpt {
    /// The cleaned text, at most the budget.
    pub text: String,
    /// True when the page goes on beyond `text`.
    pub cut: bool,
}

/// How many tokens of page text a prompt may carry. Tokens are estimated, not counted: a
/// tokenizer is model-specific, and for English prose and code about four characters make a
/// token ([`Self::CHARS_PER_TOKEN`]), which errs on the long side for other scripts.
///
/// [`Self::DEFAULT`] is small on purpose: local servers often run with a context of 4096 tokens,
/// and some cut an overlong prompt from its start, which would drop the rules that keep page
/// content untrusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenBudget(usize);

impl TokenBudget {
    /// Characters per token, the estimate's ratio.
    pub const CHARS_PER_TOKEN: usize = 4;

    /// The page budget unless settings say otherwise.
    pub const DEFAULT: Self = Self(3_000);

    /// A budget of `tokens`.
    #[must_use]
    pub const fn new(tokens: usize) -> Self {
        Self(tokens)
    }

    /// The budget in tokens.
    #[must_use]
    pub const fn tokens(self) -> usize {
        self.0
    }

    /// The estimated tokens of `text`: its characters over [`Self::CHARS_PER_TOKEN`], rounded
    /// up.
    #[must_use]
    pub fn estimate(text: &str) -> usize {
        text.chars().count().div_ceil(Self::CHARS_PER_TOKEN)
    }

    /// The budget in characters.
    pub(super) const fn chars(self) -> usize {
        self.0.saturating_mul(Self::CHARS_PER_TOKEN)
    }

    /// Each page's share when `pages` pages split the budget; no pages leave it whole.
    pub(super) const fn shared(self, pages: usize) -> Self {
        Self(self.0 / if pages == 0 { 1 } else { pages })
    }
}

/// Page text made safe to fence: the fence characters become plain brackets, control
/// characters go, and whitespace collapses to a space, a line break, or a paragraph break,
/// which also saves tokens.
pub(super) fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    let mut breaks = 0_usize;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
            breaks += usize::from(c == '\n');
            continue;
        }
        if c.is_control() {
            continue;
        }
        if space && !out.is_empty() {
            out.push_str(match breaks {
                0 => " ",
                1 => "\n",
                _ => "\n\n",
            });
        }
        space = false;
        breaks = 0;
        out.push(match c {
            FENCE_OPEN => '[',
            FENCE_CLOSE => ']',
            other => other,
        });
    }
    out
}

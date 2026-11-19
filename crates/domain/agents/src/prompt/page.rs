//! The page as the agent reads it: its address, title, and text, cleaned and cut to a token
//! budget that counts all three.

use url::Url;

use super::fence::clean;

/// The longest title read, in characters; a page cannot spend the budget on its title.
const TITLE_MAX_CHARS: usize = 200;

/// The longest address read, in characters. A page can make its own address megabytes long
/// with `history.replaceState`, so the address is cut like the text.
const URL_MAX_CHARS: usize = 200;

/// A page's text as the engine extracted it. It is untrusted: whoever wrote the page wrote it,
/// its address and title included, and any of it may address the agent.
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

    /// The address, cut to [`URL_MAX_CHARS`] with `…` when longer. It is ASCII: the URL parser
    /// percent-encodes everything else.
    pub(super) fn clean_url(&self) -> String {
        let url = self.url.as_str();
        match url.char_indices().nth(URL_MAX_CHARS) {
            Some((at, _)) => format!("{}…", &url[..at]),
            None => url.to_owned(),
        }
    }

    /// The title on one line, cleaned and cut to [`TITLE_MAX_CHARS`].
    pub(super) fn clean_title(&self) -> String {
        let line = clean(&self.title)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        line.chars().take(TITLE_MAX_CHARS).collect()
    }

    /// The page as sent: address, title, and the text cleaned and cut so the three together
    /// stay within `budget`.
    pub(super) fn excerpt(&self, budget: TokenBudget) -> Excerpt {
        let url = self.clean_url();
        let title = self.clean_title();
        let spent = TokenBudget::estimate(&url) + TokenBudget::estimate(&title);
        let mut text = clean(&self.text);
        let cut = TokenBudget::new(budget.tokens().saturating_sub(spent)).cut(&text);
        if let Some(at) = cut {
            text.truncate(at);
        }
        Excerpt {
            url,
            title,
            text,
            cut: cut.is_some(),
        }
    }
}

/// A page as sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Excerpt {
    /// The address, cut.
    pub url: String,
    /// The title, cleaned and cut.
    pub title: String,
    /// The cleaned text, cut to what the budget leaves.
    pub text: String,
    /// True when the page goes on beyond `text`.
    pub cut: bool,
}

/// How many tokens of page a prompt may carry. Tokens are estimated, not counted: a tokenizer
/// is model-specific. The estimate errs on the long side: about four ASCII characters make a
/// token ([`Self::CHARS_PER_TOKEN`]) in English prose and code, and every other character
/// counts as a whole token, as a character of Chinese or Japanese is about one token and an
/// emoji two or three, so a page in any script stays within the budget.
///
/// [`Self::DEFAULT`] is small on purpose: local servers often run with a context of 4096 tokens,
/// and some cut an overlong prompt from its start, which would drop the rules that keep page
/// content untrusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenBudget(usize);

impl TokenBudget {
    /// ASCII characters per token, the estimate's ratio.
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

    /// The estimated tokens of `text`: its ASCII characters over [`Self::CHARS_PER_TOKEN`],
    /// rounded up, plus one for every other character.
    #[must_use]
    pub fn estimate(text: &str) -> usize {
        let ascii = text.chars().filter(char::is_ascii).count();
        let other = text.chars().count() - ascii;
        ascii.div_ceil(Self::CHARS_PER_TOKEN) + other
    }

    /// Each page's share when `pages` pages split the budget; no pages leave it whole.
    pub(super) const fn shared(self, pages: usize) -> Self {
        Self(self.0 / if pages == 0 { 1 } else { pages })
    }

    /// Where `text` must be cut to fit, in bytes; `None` when it fits whole. Counted in
    /// quarter tokens: an ASCII character costs one, any other four.
    fn cut(self, text: &str) -> Option<usize> {
        let room = self.0.saturating_mul(Self::CHARS_PER_TOKEN);
        let mut spent = 0_usize;
        for (at, c) in text.char_indices() {
            spent += if c.is_ascii() {
                1
            } else {
                Self::CHARS_PER_TOKEN
            };
            if spent > room {
                return Some(at);
            }
        }
        None
    }
}

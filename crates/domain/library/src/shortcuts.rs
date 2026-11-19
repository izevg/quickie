//! Search shortcuts: `!gh quickie` in the palette searches GitHub for "quickie". A keyword names
//! a URL template, and the terms after it fill the template's `%s`.

mod keyword;
mod snapshot;
mod template;
#[cfg(test)]
mod tests;

use indexmap::IndexMap;
use url::Url;

pub use keyword::{Keyword, KeywordError};
pub use snapshot::{CorruptShortcuts, SearchShortcutsSnapshot};
pub use template::{TemplateError, UrlTemplate};

/// The shortcuts a new profile starts with. All https.
const DEFAULTS: [(&str, &str); 6] = [
    ("d", "https://duckduckgo.com/?q=%s"),
    ("w", "https://en.wikipedia.org/w/index.php?search=%s"),
    ("gh", "https://github.com/search?q=%s"),
    ("mdn", "https://developer.mozilla.org/en-US/search?q=%s"),
    ("crates", "https://crates.io/search?q=%s"),
    ("docs", "https://docs.rs/releases/search?query=%s"),
];

/// The keyword searches of one profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchShortcuts {
    /// In the order they were added.
    shortcuts: IndexMap<Keyword, UrlTemplate>,
}

impl SearchShortcuts {
    /// No shortcuts at all.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The shortcuts a new profile starts with: `d` searches duckduckgo.com, `w` Wikipedia, `gh`
    /// github.com, `mdn` the MDN docs, `crates` crates.io, `docs` docs.rs.
    #[must_use]
    pub fn defaults() -> Self {
        let shortcuts = DEFAULTS
            .iter()
            .filter_map(|&(keyword, template)| {
                Some((
                    Keyword::try_from(keyword).ok()?,
                    UrlTemplate::try_from(template).ok()?,
                ))
            })
            .collect();
        Self { shortcuts }
    }

    /// Binds a keyword to a template and returns the template it replaced.
    pub fn set(&mut self, keyword: Keyword, template: UrlTemplate) -> Option<UrlTemplate> {
        self.shortcuts.insert(keyword, template)
    }

    /// Unbinds a keyword and returns its template.
    pub fn remove(&mut self, keyword: &str) -> Option<UrlTemplate> {
        self.shortcuts.shift_remove(keyword)
    }

    /// The template a keyword names.
    #[must_use]
    pub fn get(&self, keyword: &str) -> Option<&UrlTemplate> {
        self.shortcuts.get(keyword)
    }

    /// Every shortcut, in the order it was added.
    pub fn iter(&self) -> impl Iterator<Item = (&Keyword, &UrlTemplate)> {
        self.shortcuts.iter()
    }

    /// The URL `!keyword terms` opens; `None` for an unknown keyword or terms that make no URL
    /// in that template.
    #[must_use]
    pub fn expand(&self, keyword: &str, terms: &str) -> Option<Url> {
        self.get(keyword)?.fill(terms)
    }
}

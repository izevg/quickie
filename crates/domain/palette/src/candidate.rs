//! Things the palette can offer: tabs, bookmarks, history, commands, headings, tags, search
//! shortcuts. The caller owns them and names each with its own key.

mod frecency;

pub use frecency::{Frecency, FrecencyError};

/// What a candidate is, which decides the modes it appears in and how much it is boosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A tab that is open now; switching to it beats opening the page again.
    OpenTab,
    /// A saved page.
    Bookmark,
    /// A page visited before.
    History,
    /// A command of the keymap.
    Command,
    /// A heading of the current page.
    Heading,
    /// A bookmark tag, for `#`.
    Tag,
    /// A keyword search engine, for `!`.
    SearchShortcut,
}

impl Kind {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 7] = [
        Self::OpenTab,
        Self::Bookmark,
        Self::History,
        Self::Command,
        Self::Heading,
        Self::Tag,
        Self::SearchShortcut,
    ];
}

/// One thing the palette can offer, under the caller's key `K` (a tab id, a command id, a URL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate<K> {
    /// The caller's name for the candidate, handed back in the ranking.
    pub key: K,
    /// What the candidate is.
    pub kind: Kind,
    /// The main line: a page title, a command name, a heading.
    pub title: String,
    /// The second line: a host or URL, a key binding, a shortcut keyword.
    pub detail: String,
    /// How often and how recently the user chose it.
    pub visits: Frecency,
}

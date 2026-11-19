//! What the palette hands the UI: one row per answer, with what choosing it does.

use quickie_domain_browsing::TabId;
use quickie_domain_commands::CommandId;
use quickie_domain_library::Tag;
use quickie_domain_palette::Kind;
use url::Url;

/// What choosing a palette row does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// Switch to this open tab.
    Tab(TabId),
    /// Open this page: a bookmark or a history entry.
    Url(Url),
    /// Run this command.
    Command(CommandId),
    /// List the bookmarks with this tag.
    Tag(Tag),
    /// Open the results of a keyword search.
    Search(Url),
}

/// One row of the palette, best first in a query's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// What choosing the row does.
    pub target: Target,
    /// What the row is, for its icon.
    pub kind: Kind,
    /// The main line: a page title, a command, a tag, a keyword.
    pub title: String,
    /// The second line: a URL, a command's category, a tag's page count, a search template.
    pub detail: String,
    /// The keys bound to a command, as macOS glyphs (`⇧⌘P`); empty for everything else.
    pub glyphs: Vec<String>,
    /// Character positions of `title` the query matched, ascending, for highlighting.
    pub title_matches: Vec<u32>,
    /// Character positions of `detail` the query matched, ascending, for highlighting.
    pub detail_matches: Vec<u32>,
}

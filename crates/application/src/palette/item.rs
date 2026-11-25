//! What the palette hands the UI: one row per answer, with what choosing it does.

use quickie_domain_agents::{Question, SkillId};
use quickie_domain_browsing::{PinnedSlot, TabId, Timestamp};
use quickie_domain_commands::CommandId;
use quickie_domain_library::{HistoryEntry, Tag};
use quickie_domain_palette::Kind;
use url::Url;

use crate::ExtensionTrigger;

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
    /// Run this skill of the agent on the current page.
    Skill(SkillId),
    /// Send this prompt to the agent workspace.
    Ask(Question),
    /// Install the extension this Chrome Web Store page shows.
    InstallExtension(Url),
    /// Run an extension's action or command.
    Extension(ExtensionTrigger),
}

/// One row of the palette, best first in a query's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// What choosing the row does.
    pub target: Target,
    /// What the row is, for its icon.
    pub kind: Kind,
    /// The main line: a page title, a command, a tag, a keyword, a skill, the question.
    pub title: String,
    /// The second line: a URL, a command's category, a tag's page count, a search template,
    /// what a skill does.
    pub detail: String,
    /// The keys bound to a command, as the keymap's platform prints them (`⇧⌘P` on macOS,
    /// `Ctrl+Shift+P` on Linux); empty for everything else.
    pub glyphs: Vec<String>,
    /// Character positions of `title` the query matched, ascending, for highlighting.
    pub title_matches: Vec<u32>,
    /// Character positions of `detail` the query matched, ascending, for highlighting.
    pub detail_matches: Vec<u32>,
    /// What the library knows of the page the row opens; nothing for other rows.
    pub facts: PageFacts,
}

/// What the library and the session know of a page: measured, never estimated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageFacts {
    /// How many visits history recorded; 0 for a page never visited.
    pub visits: u64,
    /// When the page was last visited.
    pub last_visit: Option<Timestamp>,
    /// The slot the page's open tab is pinned to (`⌘3`).
    pub slot: Option<PinnedSlot>,
}

impl PageFacts {
    /// What history knows of the page `entry` records: its visits and the latest of them.
    #[must_use]
    pub fn of(entry: &HistoryEntry) -> Self {
        Self {
            visits: entry.visit_count,
            last_visit: entry.recent_visits.iter().max().copied(),
            slot: None,
        }
    }
}

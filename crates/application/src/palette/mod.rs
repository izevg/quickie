//! The palette query: what the user typed, answered from the open tabs, the library, the
//! command catalog, the running extensions, and the agent's skills.
//!
//! Opening the palette gathers every candidate and scores its frecency once
//! ([`Palette::open`]); each keystroke then only ranks ([`PaletteIndex::query`]), synchronously,
//! from memory, within the palette's first-frame budget (`docs/architecture.md`, §5).

mod index;
mod item;
#[cfg(test)]
mod tests;

use quickie_domain_agents::Skill;
use quickie_domain_browsing::Session;
use quickie_domain_commands::{Catalog, Keymap};
use quickie_domain_library::{Bookmarks, History, SearchShortcuts};

pub use index::{AddressSuggestions, InlineCompletion, PaletteIndex, Suggestion, SuggestionKind};
pub use item::{PageFacts, PaletteItem, Target};

use crate::{Clock, ExtensionOffer};

/// The row that installs the extension a pasted Chrome Web Store page shows.
const INSTALL_TITLE: &str = "Install Extension from Chrome Web Store";

/// The start of the row that asks the agent the typed question.
const ASK_TITLE: &str = "Send to Agent: ";

/// What the row that asks the agent says it does.
const ASK_DETAIL: &str = "Open the Agent workspace and send this prompt with ⏎";

/// What the palette draws its candidates from, borrowed while it opens.
#[derive(Clone, Copy, Debug)]
pub struct Sources<'a> {
    /// The open tabs.
    pub session: &'a Session,
    /// The visited pages, which also give tabs and bookmarks their frecency.
    pub history: &'a History,
    /// The saved pages and their tags.
    pub bookmarks: &'a Bookmarks,
    /// The keyword searches.
    pub shortcuts: &'a SearchShortcuts,
    /// The keys bound to each command.
    pub keymap: &'a Keymap,
    /// The actions and commands of running extensions.
    pub extensions: &'a [ExtensionOffer],
}

/// The palette use case.
#[derive(Debug)]
pub struct Palette<C> {
    catalog: Catalog,
    skills: Vec<Skill>,
    clock: C,
}

impl<C: Clock> Palette<C> {
    /// A palette over the commands of `catalog` and the built-in skills, ranking by frecency
    /// at the `clock`'s now.
    #[must_use]
    pub fn new(catalog: Catalog, clock: C) -> Self {
        Self {
            catalog,
            skills: Skill::builtin(),
            clock,
        }
    }

    /// Opens the palette on `sources`: gathers the candidates of every mode and scores their
    /// frecency at the clock's now, once. The index answers keystroke after keystroke until the
    /// sources change; then the caller opens a new one.
    #[must_use]
    pub fn open(&self, sources: &Sources<'_>) -> PaletteIndex {
        PaletteIndex::gather(&self.catalog, &self.skills, sources, self.clock.now())
    }
}

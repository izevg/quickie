//! Commands and the catalog that lists them.

mod builtin;
#[cfg(test)]
mod tests;

use indexmap::IndexMap;
use quickie_domain_kernel::CommandId;

/// Something Quickie does on request, from the palette or a key binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The stable id `keybindings.json` refers to.
    pub id: CommandId,
    /// What the palette shows, an imperative: `Reopen Closed Tab`.
    pub title: &'static str,
    /// The group the palette and the keymap editor sort it into.
    pub category: Category,
}

/// The group a command belongs to in the palette and the keymap editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// The palette and the go-to switcher.
    Palette,
    /// Opening, closing, switching, and reloading tabs.
    Tabs,
    /// The address overlay and moving through history.
    Navigation,
    /// Acting on the current page: find, link hints, reader mode, copying its address.
    Page,
    /// Files the browser downloaded.
    Downloads,
    /// The developer tools and their panels.
    Developer,
}

impl Category {
    /// The heading shown above the group.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Palette => "Palette",
            Self::Tabs => "Tabs",
            Self::Navigation => "Navigation",
            Self::Page => "Page",
            Self::Downloads => "Downloads",
            Self::Developer => "Developer",
        }
    }
}

/// Every command Quickie knows, in palette order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog(IndexMap<CommandId, Command>);

impl Catalog {
    /// The commands built into Quickie.
    #[must_use]
    pub fn builtin() -> Self {
        Self(
            builtin::commands()
                .map(|command| (command.id.clone(), command))
                .collect(),
        )
    }

    /// The command with this id, if Quickie has it.
    #[must_use]
    pub fn get(&self, id: &CommandId) -> Option<&Command> {
        self.0.get(id)
    }

    /// Whether Quickie has a command with this id.
    #[must_use]
    pub fn contains(&self, id: &CommandId) -> bool {
        self.0.contains_key(id)
    }

    /// The commands in palette order.
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.0.values()
    }
}

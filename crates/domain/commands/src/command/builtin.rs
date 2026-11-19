//! The commands built into Quickie.

use quickie_domain_kernel::CommandId;

use super::{Category, Command};

/// Id, title, and category of every built-in command, in palette order. The tests check that
/// every id parses, so a typo here fails the build's tests, not the user.
pub(super) const COMMANDS: [(&str, &str, Category); 37] = [
    (
        "quickie.palette.openCommands",
        "Show Command Palette",
        Category::Palette,
    ),
    (
        "quickie.palette.goto",
        "Go to Tab or History",
        Category::Palette,
    ),
    ("quickie.tab.new", "Open New Tab", Category::Tabs),
    ("quickie.tab.close", "Close Tab", Category::Tabs),
    (
        "quickie.tab.reopenClosed",
        "Reopen Closed Tab",
        Category::Tabs,
    ),
    (
        "quickie.tab.nextRecent",
        "Switch to Next Recent Tab",
        Category::Tabs,
    ),
    (
        "quickie.tab.previousRecent",
        "Switch to Previous Recent Tab",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot1",
        "Go to Pinned Slot 1",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot2",
        "Go to Pinned Slot 2",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot3",
        "Go to Pinned Slot 3",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot4",
        "Go to Pinned Slot 4",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot5",
        "Go to Pinned Slot 5",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot6",
        "Go to Pinned Slot 6",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot7",
        "Go to Pinned Slot 7",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot8",
        "Go to Pinned Slot 8",
        Category::Tabs,
    ),
    (
        "quickie.tab.goToSlot9",
        "Go to Pinned Slot 9",
        Category::Tabs,
    ),
    (
        "quickie.tab.openInThrowawayJar",
        "Open in Throwaway Jar",
        Category::Tabs,
    ),
    ("quickie.tab.reload", "Reload Tab", Category::Tabs),
    (
        "quickie.tab.hardReload",
        "Reload Tab Without Cache",
        Category::Tabs,
    ),
    (
        "quickie.address.open",
        "Show Address Overlay",
        Category::Navigation,
    ),
    ("quickie.navigation.back", "Go Back", Category::Navigation),
    (
        "quickie.navigation.forward",
        "Go Forward",
        Category::Navigation,
    ),
    ("quickie.page.find", "Find in Page", Category::Page),
    ("quickie.hints.toggle", "Toggle Link Hints", Category::Page),
    ("quickie.page.copyUrl", "Copy URL", Category::Page),
    (
        "quickie.page.copyAsMarkdownLink",
        "Copy as Markdown Link",
        Category::Page,
    ),
    (
        "quickie.reader.toggle",
        "Toggle Reader Mode",
        Category::Page,
    ),
    (
        "quickie.downloads.show",
        "Show Downloads",
        Category::Downloads,
    ),
    (
        "quickie.devtools.toggle",
        "Toggle Developer Tools",
        Category::Developer,
    ),
    (
        "quickie.devtools.inspectElement",
        "Inspect Element",
        Category::Developer,
    ),
    (
        "quickie.devtools.elements",
        "Show Elements Panel",
        Category::Developer,
    ),
    (
        "quickie.devtools.console",
        "Show Console Panel",
        Category::Developer,
    ),
    (
        "quickie.devtools.network",
        "Show Network Panel",
        Category::Developer,
    ),
    (
        "quickie.devtools.sources",
        "Show Sources Panel",
        Category::Developer,
    ),
    (
        "quickie.devtools.performance",
        "Show Performance Panel",
        Category::Developer,
    ),
    (
        "quickie.devtools.application",
        "Show Application Panel",
        Category::Developer,
    ),
    (
        "quickie.devtools.memory",
        "Show Memory Panel",
        Category::Developer,
    ),
];

/// The built-in commands, in palette order.
pub(super) fn commands() -> impl Iterator<Item = Command> {
    COMMANDS.into_iter().filter_map(|(id, title, category)| {
        let id = CommandId::try_from(id).ok()?;
        Some(Command {
            id,
            title,
            category,
        })
    })
}

//! The commands built into Quickie.

use quickie_domain_kernel::CommandId;

use super::{Category, Command};

/// Id, title, and category of every built-in command, in palette order. The tests check that
/// every id parses, so a typo here fails the build's tests, not the user.
pub(super) const COMMANDS: [(&str, &str, Category); 78] = [
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
    (
        "quickie.tab.newThrowaway",
        "Open New Throwaway Tab",
        Category::Tabs,
    ),
    ("quickie.tab.close", "Close Tab", Category::Tabs),
    ("quickie.window.new", "Open New Window", Category::Tabs),
    ("quickie.window.close", "Close Window", Category::Tabs),
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
        "quickie.tab.pinToSlot1",
        "Pin Tab to Slot 1",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot2",
        "Pin Tab to Slot 2",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot3",
        "Pin Tab to Slot 3",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot4",
        "Pin Tab to Slot 4",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot5",
        "Pin Tab to Slot 5",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot6",
        "Pin Tab to Slot 6",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot7",
        "Pin Tab to Slot 7",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot8",
        "Pin Tab to Slot 8",
        Category::Tabs,
    ),
    (
        "quickie.tab.pinToSlot9",
        "Pin Tab to Slot 9",
        Category::Tabs,
    ),
    ("quickie.tab.overview", "Show Tab Overview", Category::Tabs),
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
    ("quickie.page.findNext", "Find Next", Category::Page),
    ("quickie.page.findPrevious", "Find Previous", Category::Page),
    ("quickie.hints.toggle", "Toggle Link Hints", Category::Page),
    (
        "quickie.hints.openInBackground",
        "Open Link Hint in Background Tab",
        Category::Page,
    ),
    ("quickie.page.copyUrl", "Copy URL", Category::Page),
    ("quickie.page.bookmark", "Bookmark Page", Category::Page),
    (
        "quickie.page.copyAsMarkdownLink",
        "Copy as Markdown Link",
        Category::Page,
    ),
    ("quickie.page.copyTitle", "Copy Page Title", Category::Page),
    ("quickie.page.zoomIn", "Zoom In", Category::Page),
    ("quickie.page.zoomOut", "Zoom Out", Category::Page),
    ("quickie.page.zoomReset", "Reset Zoom", Category::Page),
    ("quickie.page.print", "Print Page", Category::Page),
    ("quickie.page.savePdf", "Save Page as PDF", Category::Page),
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
    ("quickie.shields.open", "Show Shields", Category::Shields),
    (
        "quickie.shields.toggleSite",
        "Toggle Shields for This Site",
        Category::Shields,
    ),
    (
        "quickie.shields.enableLists",
        "Enable Filter Lists",
        Category::Shields,
    ),
    (
        "quickie.shields.refreshLists",
        "Refresh Filter Lists",
        Category::Shields,
    ),
    (
        "quickie.extensions.installFromUrl",
        "Install Extension from URL…",
        Category::Extensions,
    ),
    (
        "quickie.extensions.installFromFile",
        "Install Extension from File…",
        Category::Extensions,
    ),
    (
        "quickie.extensions.checkUpdates",
        "Check Extension Updates",
        Category::Extensions,
    ),
    (
        "quickie.extensions.offOnSite",
        "Turn Extensions Off on This Site",
        Category::Extensions,
    ),
    (
        "quickie.extensions.onOnSite",
        "Turn Extensions On on This Site",
        Category::Extensions,
    ),
    (
        "quickie.extensions.settings",
        "Show Extensions",
        Category::Extensions,
    ),
    (
        "quickie.agent.ask",
        "Ask Agent About This Page",
        Category::Agent,
    ),
    ("quickie.agent.stop", "Stop Agent", Category::Agent),
    (
        "quickie.agent.showAnswer",
        "Show Agent Answer",
        Category::Agent,
    ),
    (
        "quickie.agent.settings",
        "Show Agent Settings",
        Category::Agent,
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
    (
        "quickie.app.setDefaultBrowser",
        "Set as Default Browser",
        Category::App,
    ),
    (
        "quickie.app.revealProfile",
        "Show Profile in Finder",
        Category::App,
    ),
    ("quickie.app.settings", "Settings…", Category::App),
    (
        "quickie.debug.keys",
        "Show Key Debug Overlay",
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

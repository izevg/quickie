//! The default key bindings, written as `keybindings.json` would write them.

use super::binding::KeyBinding;

/// Key, command, and `when` clause of every default binding: the README's "Default keys" table,
/// `⌘K` as a second key for the palette, `⌥⌘1` … `⌥⌘9` to pin a tab to a slot, and the Chrome
/// keys for reloading, bookmarking, a throwaway window, the next and previous find match, and
/// the developer tools.
///
/// `⌥⌘C` stays with "Copy as Markdown Link" as the README promises, so "Inspect Element" has no
/// default key. The tests check that every row parses and that the keymap they make is valid.
pub(super) const BINDINGS: [(&str, &str, Option<&str>); 46] = [
    ("cmd+shift+p", "quickie.palette.openCommands", None),
    ("cmd+k", "quickie.palette.openCommands", None),
    ("cmd+p", "quickie.palette.goto", None),
    ("cmd+l", "quickie.address.open", None),
    ("cmd+t", "quickie.tab.new", None),
    ("cmd+shift+n", "quickie.tab.newThrowaway", None),
    ("cmd+w", "quickie.tab.close", None),
    ("cmd+n", "quickie.window.new", None),
    ("cmd+shift+w", "quickie.window.close", None),
    ("cmd+shift+t", "quickie.tab.reopenClosed", None),
    ("ctrl+tab", "quickie.tab.nextRecent", None),
    ("ctrl+shift+tab", "quickie.tab.previousRecent", None),
    ("cmd+1", "quickie.tab.goToSlot1", None),
    ("cmd+2", "quickie.tab.goToSlot2", None),
    ("cmd+3", "quickie.tab.goToSlot3", None),
    ("cmd+4", "quickie.tab.goToSlot4", None),
    ("cmd+5", "quickie.tab.goToSlot5", None),
    ("cmd+6", "quickie.tab.goToSlot6", None),
    ("cmd+7", "quickie.tab.goToSlot7", None),
    ("cmd+8", "quickie.tab.goToSlot8", None),
    ("cmd+9", "quickie.tab.goToSlot9", None),
    ("alt+cmd+1", "quickie.tab.pinToSlot1", None),
    ("alt+cmd+2", "quickie.tab.pinToSlot2", None),
    ("alt+cmd+3", "quickie.tab.pinToSlot3", None),
    ("alt+cmd+4", "quickie.tab.pinToSlot4", None),
    ("alt+cmd+5", "quickie.tab.pinToSlot5", None),
    ("alt+cmd+6", "quickie.tab.pinToSlot6", None),
    ("alt+cmd+7", "quickie.tab.pinToSlot7", None),
    ("alt+cmd+8", "quickie.tab.pinToSlot8", None),
    ("alt+cmd+9", "quickie.tab.pinToSlot9", None),
    ("cmd+r", "quickie.tab.reload", None),
    ("cmd+[", "quickie.navigation.back", None),
    ("cmd+]", "quickie.navigation.forward", None),
    ("cmd+f", "quickie.page.find", None),
    ("cmd+g", "quickie.page.findNext", None),
    ("cmd+shift+g", "quickie.page.findPrevious", None),
    (
        "f",
        "quickie.hints.toggle",
        Some("pageFocused && !inputFocused"),
    ),
    (
        "shift+f",
        "quickie.hints.openInBackground",
        Some("pageFocused && !inputFocused"),
    ),
    ("cmd+shift+\\", "quickie.tab.overview", None),
    ("cmd+shift+c", "quickie.page.copyUrl", None),
    ("alt+cmd+c", "quickie.page.copyAsMarkdownLink", None),
    ("cmd+d", "quickie.page.bookmark", None),
    ("cmd+j", "quickie.downloads.show", None),
    ("alt+cmd+s", "quickie.shields.open", None),
    ("alt+cmd+i", "quickie.devtools.toggle", None),
    ("alt+cmd+j", "quickie.devtools.console", None),
];

/// The default bindings, in order.
pub(super) fn bindings() -> Vec<KeyBinding> {
    BINDINGS
        .into_iter()
        .filter_map(|(key, command, when)| KeyBinding::parse(key, command, when).ok())
        .collect()
}

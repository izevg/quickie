//! The default key bindings and the reserved strokes, written as `keybindings.json` would write
//! them, with `mod` for the primary modifier so one table serves every platform.

use super::binding::KeyBinding;
use crate::key::{KeyStroke, Platform};

/// Key, command, and `when` clause of every default binding: the README's "Default keys" table,
/// `⌘K` as a second key for the palette, `⌥⌘1` … `⌥⌘9` to pin a tab to a slot, and the Chrome
/// keys for reloading, bookmarking, a throwaway window, the next and previous find match, and
/// the developer tools.
///
/// `mod` is `⌘` on macOS and Ctrl on Linux. Recent-tab cycling is written `ctrl` on purpose:
/// it is `⌃⇥` on macOS too, and `⌘⇥` belongs to the system there.
///
/// `⌥⌘C` stays with "Copy as Markdown Link" as the README promises, so "Inspect Element" has no
/// default key. The tests check that every row parses and that the keymap they make is valid.
pub(super) const BINDINGS: [(&str, &str, Option<&str>); 47] = [
    ("mod+shift+p", "quickie.palette.openCommands", None),
    ("mod+k", "quickie.palette.openCommands", None),
    ("mod+p", "quickie.palette.goto", None),
    ("mod+l", "quickie.address.open", None),
    ("mod+t", "quickie.tab.new", None),
    ("mod+shift+n", "quickie.tab.newThrowaway", None),
    ("mod+w", "quickie.tab.close", None),
    ("mod+n", "quickie.window.new", None),
    ("mod+shift+w", "quickie.window.close", None),
    ("mod+shift+t", "quickie.tab.reopenClosed", None),
    ("ctrl+tab", "quickie.tab.nextRecent", None),
    ("ctrl+shift+tab", "quickie.tab.previousRecent", None),
    ("mod+1", "quickie.tab.goToSlot1", None),
    ("mod+2", "quickie.tab.goToSlot2", None),
    ("mod+3", "quickie.tab.goToSlot3", None),
    ("mod+4", "quickie.tab.goToSlot4", None),
    ("mod+5", "quickie.tab.goToSlot5", None),
    ("mod+6", "quickie.tab.goToSlot6", None),
    ("mod+7", "quickie.tab.goToSlot7", None),
    ("mod+8", "quickie.tab.goToSlot8", None),
    ("mod+9", "quickie.tab.goToSlot9", None),
    ("alt+mod+1", "quickie.tab.pinToSlot1", None),
    ("alt+mod+2", "quickie.tab.pinToSlot2", None),
    ("alt+mod+3", "quickie.tab.pinToSlot3", None),
    ("alt+mod+4", "quickie.tab.pinToSlot4", None),
    ("alt+mod+5", "quickie.tab.pinToSlot5", None),
    ("alt+mod+6", "quickie.tab.pinToSlot6", None),
    ("alt+mod+7", "quickie.tab.pinToSlot7", None),
    ("alt+mod+8", "quickie.tab.pinToSlot8", None),
    ("alt+mod+9", "quickie.tab.pinToSlot9", None),
    ("mod+r", "quickie.tab.reload", None),
    ("mod+[", "quickie.navigation.back", None),
    ("mod+]", "quickie.navigation.forward", None),
    ("mod+f", "quickie.page.find", None),
    ("mod+g", "quickie.page.findNext", None),
    ("mod+shift+g", "quickie.page.findPrevious", None),
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
    ("mod+shift+\\", "quickie.tab.overview", None),
    ("mod+shift+c", "quickie.page.copyUrl", None),
    ("alt+mod+c", "quickie.page.copyAsMarkdownLink", None),
    ("mod+d", "quickie.page.bookmark", None),
    ("mod+j", "quickie.downloads.show", None),
    ("alt+mod+s", "quickie.shields.open", None),
    ("mod+.", "quickie.agent.stop", None),
    ("alt+mod+i", "quickie.devtools.toggle", None),
    ("alt+mod+j", "quickie.devtools.console", None),
];

/// The strokes the shell keeps from the page: quit, close, new tab, address, both palettes, and
/// recent-tab cycling. Quit is no command, so it is reserved without a binding.
pub(super) const RESERVED: [&str; 7] = [
    "mod+q",
    "mod+w",
    "mod+t",
    "mod+l",
    "mod+p",
    "mod+shift+p",
    "ctrl+tab",
];

/// The default bindings on `platform`, in order.
pub(super) fn bindings(platform: Platform) -> Vec<KeyBinding> {
    BINDINGS
        .into_iter()
        .filter_map(|(key, command, when)| KeyBinding::parse(key, command, when, platform).ok())
        .collect()
}

/// The reserved strokes on `platform`.
pub(super) fn reserved(platform: Platform) -> Vec<KeyStroke> {
    RESERVED
        .into_iter()
        .filter_map(|text| KeyStroke::parse(text, platform).ok())
        .collect()
}

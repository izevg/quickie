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
/// Zoom has Safari's keys: `⌘=` and `⌘+` (`⇧⌘=` on the key code) in, `⌘-` out, `⌘0` back to
/// 100 %. Printing takes `⌥⌘P`, since `⌘P` goes to a tab.
///
/// Link hints take `⌥F` (`⇧⌥F` into the background), not a bare `F`: no shortcut types (see
/// [`Chord::check_shortcut`](crate::key::Chord::check_shortcut)), so they work in a text field
/// too, where a bare letter would only type.
///
/// `⌥⌘C` stays with "Copy as Markdown Link" as the README promises, so "Inspect Element" has no
/// default key. The tests check that every row parses and that the keymap they make is valid.
pub(super) const BINDINGS: [(&str, &str, Option<&str>); 55] = [
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
    ("mod+shift+r", "quickie.tab.hardReload", None),
    ("mod+=", "quickie.page.zoomIn", None),
    ("mod+shift+=", "quickie.page.zoomIn", None),
    ("mod+-", "quickie.page.zoomOut", None),
    ("mod+0", "quickie.page.zoomReset", None),
    ("alt+mod+p", "quickie.page.print", None),
    ("mod+[", "quickie.navigation.back", None),
    ("mod+]", "quickie.navigation.forward", None),
    ("mod+f", "quickie.page.find", None),
    ("mod+g", "quickie.page.findNext", None),
    ("mod+shift+g", "quickie.page.findPrevious", None),
    ("alt+f", "quickie.hints.toggle", Some("pageFocused")),
    (
        "shift+alt+f",
        "quickie.hints.openInBackground",
        Some("pageFocused"),
    ),
    ("mod+shift+\\", "quickie.tab.overview", None),
    ("mod+shift+c", "quickie.page.copyUrl", None),
    ("alt+mod+c", "quickie.page.copyAsMarkdownLink", None),
    ("mod+d", "quickie.page.bookmark", None),
    ("mod+j", "quickie.downloads.show", None),
    ("alt+mod+s", "quickie.shields.open", None),
    ("mod+.", "quickie.agent.stop", None),
    ("f12", "quickie.devtools.toggle", None),
    ("alt+mod+i", "quickie.devtools.toggle", None),
    ("alt+mod+j", "quickie.devtools.console", None),
    ("mod+,", "quickie.app.settings", None),
];

/// Existing Linux alternatives. Putting them first gives native shortcuts the first keycap.
const LINUX_ALIASES: [(&str, &str, Option<&str>); 3] = [
    ("alt+left", "quickie.navigation.back", None),
    ("alt+right", "quickie.navigation.forward", None),
    ("ctrl+shift+i", "quickie.devtools.toggle", None),
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
    let aliases = match platform {
        Platform::Linux => Some(LINUX_ALIASES),
        Platform::MacOs => None,
    };
    aliases
        .into_iter()
        .flatten()
        .chain(BINDINGS)
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

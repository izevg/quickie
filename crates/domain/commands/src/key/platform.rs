//! The desktop whose keyboard conventions a keymap follows.

use super::Modifiers;

/// The desktop Quickie runs on, as far as keys care: which modifier is the primary one and how
/// keycaps are written.
///
/// The composition root picks it once; everything downstream receives it as a value, so the
/// domain stays free of `cfg` and the lookup of a stroke never asks which desktop it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// macOS: `⌘` is primary, keycaps are glyphs in the order `⌃⌥⇧⌘`.
    MacOs,
    /// Linux: Ctrl is primary, keycaps are words joined by `+` in the order Ctrl, Alt, Shift,
    /// Super.
    Linux,
}

impl Platform {
    /// The modifier `mod` stands for in `keybindings.json`: `⌘` on macOS, Ctrl on Linux.
    #[must_use]
    pub const fn primary(self) -> Modifiers {
        match self {
            Self::MacOs => Modifiers::COMMAND,
            Self::Linux => Modifiers::CONTROL,
        }
    }
}

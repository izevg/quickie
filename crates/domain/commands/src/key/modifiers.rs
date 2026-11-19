//! The modifier keys held during a stroke.

use std::fmt;
use std::ops::BitOr;

/// A set of held modifiers: `⌃` Control, `⌥` Option, `⇧` Shift, `⌘` Command.
///
/// Combine with `|`: `Modifiers::COMMAND | Modifiers::SHIFT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifier held.
    pub const NONE: Self = Self(0);
    /// `⌃` Control.
    pub const CONTROL: Self = Self(1);
    /// `⌥` Option, `alt` in `keybindings.json`.
    pub const OPTION: Self = Self(1 << 1);
    /// `⇧` Shift.
    pub const SHIFT: Self = Self(1 << 2);
    /// `⌘` Command, `cmd` in `keybindings.json`.
    pub const COMMAND: Self = Self(1 << 3);

    /// Each modifier with its glyph and its `keybindings.json` name, in the order macOS menus
    /// print them: `⌃⌥⇧⌘`.
    const ORDER: [(Self, &'static str, &'static str); 4] = [
        (Self::CONTROL, "⌃", "ctrl"),
        (Self::OPTION, "⌥", "alt"),
        (Self::SHIFT, "⇧", "shift"),
        (Self::COMMAND, "⌘", "cmd"),
    ];

    /// Whether every modifier of `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// The single modifier `keybindings.json` names so, in any case: `cmd` (`command`, `meta`),
    /// `shift`, `alt` (`option`, `opt`), `ctrl` (`control`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::CONTROL),
            "alt" | "option" | "opt" => Some(Self::OPTION),
            "shift" => Some(Self::SHIFT),
            "cmd" | "command" | "meta" => Some(Self::COMMAND),
            _ => None,
        }
    }

    /// The `keybindings.json` names of the held modifiers, in macOS order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::ORDER
            .into_iter()
            .filter(move |&(modifier, _, _)| self.contains(modifier))
            .map(|(_, _, name)| name)
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Display for Modifiers {
    /// The glyphs of the held modifiers in macOS order, `⌃⌥⇧⌘`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Self::ORDER
            .into_iter()
            .filter(|&(modifier, _, _)| self.contains(modifier))
            .try_for_each(|(_, glyph, _)| f.write_str(glyph))
    }
}

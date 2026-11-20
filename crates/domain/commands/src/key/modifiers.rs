//! The modifier keys held during a stroke.

use std::ops::BitOr;

use super::Platform;

/// A set of held modifiers: Control, Option (Alt), Shift, and the logo key, which is `⌘` Command
/// on macOS and Super on Linux.
///
/// Combine with `|`: `Modifiers::COMMAND | Modifiers::SHIFT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifier held.
    pub const NONE: Self = Self(0);
    /// `⌃` Control, Ctrl on Linux.
    pub const CONTROL: Self = Self(1);
    /// `⌥` Option, `alt` in `keybindings.json` and Alt on Linux.
    pub const OPTION: Self = Self(1 << 1);
    /// `⇧` Shift.
    pub const SHIFT: Self = Self(1 << 2);
    /// `⌘` Command, `cmd` in `keybindings.json`; the Super key on Linux, a modifier of its own
    /// there, distinct from Ctrl.
    pub const COMMAND: Self = Self(1 << 3);

    /// Each modifier with its macOS glyph, its Linux label, and its `keybindings.json` name, in
    /// the order both desktops print them: `⌃⌥⇧⌘`, Ctrl Alt Shift Super.
    const ORDER: [(Self, &'static str, &'static str, &'static str); 4] = [
        (Self::CONTROL, "⌃", "Ctrl", "ctrl"),
        (Self::OPTION, "⌥", "Alt", "alt"),
        (Self::SHIFT, "⇧", "Shift", "shift"),
        (Self::COMMAND, "⌘", "Super", "cmd"),
    ];

    /// Whether every modifier of `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// The single modifier `keybindings.json` names so, in any case: `cmd` (`command`, `meta`,
    /// `super`), `shift`, `alt` (`option`, `opt`), `ctrl` (`control`). The platform-dependent
    /// `mod` is not one of them: the parser of a stroke resolves it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::CONTROL),
            "alt" | "option" | "opt" => Some(Self::OPTION),
            "shift" => Some(Self::SHIFT),
            "cmd" | "command" | "meta" | "super" => Some(Self::COMMAND),
            _ => None,
        }
    }

    /// The `keybindings.json` names of the held modifiers, in the order Control, Option, Shift,
    /// Command.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::ORDER
            .into_iter()
            .filter(move |&(modifier, _, _, _)| self.contains(modifier))
            .map(|(_, _, _, name)| name)
    }

    /// How `platform` prints each held modifier, in its order: `⌃`, `⇧`, `⌘` on macOS; `Ctrl`,
    /// `Shift`, `Super` on Linux.
    pub fn glyphs(self, platform: Platform) -> impl Iterator<Item = &'static str> {
        Self::ORDER
            .into_iter()
            .filter(move |&(modifier, _, _, _)| self.contains(modifier))
            .map(move |(_, glyph, label, _)| match platform {
                Platform::MacOs => glyph,
                Platform::Linux => label,
            })
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

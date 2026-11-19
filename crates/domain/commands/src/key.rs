//! Key strokes and chords, parsed from `keybindings.json` (`cmd+shift+p`) and shown as macOS
//! glyphs (`⇧⌘P`).

mod code;
mod modifiers;
#[cfg(test)]
mod tests;

use std::fmt;

pub use code::{Digit, FunctionKey, Key, Letter};
pub use modifiers::Modifiers;

/// One key pressed with a set of modifiers held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyStroke {
    /// The key, layout independent.
    pub key: Key,
    /// The modifiers held while it went down.
    pub modifiers: Modifiers,
}

impl KeyStroke {
    /// The stroke of `key` with `modifiers` held.
    #[must_use]
    pub const fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// The stroke as `keybindings.json` writes it: `ctrl+shift+tab`, modifiers in macOS order.
    #[must_use]
    pub fn keybinding_text(&self) -> String {
        let mut parts: Vec<String> = self.modifiers.names().map(str::to_owned).collect();
        parts.push(self.key.name());
        parts.join("+")
    }
}

impl fmt::Display for KeyStroke {
    /// Glyphs as macOS menus print them: `⌃⇧⇥`, `⌥⌘I`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.modifiers, self.key)
    }
}

impl TryFrom<&str> for KeyStroke {
    type Error = KeyStrokeError;

    /// Reads `cmd+shift+p`: modifiers, then the key, joined by `+`, in any case and any order of
    /// modifiers.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let mut parts = text.split('+');
        let key_name = parts.next_back().unwrap_or_default();
        let mut modifiers = Modifiers::NONE;
        for name in parts {
            if name.is_empty() {
                return Err(KeyStrokeError::EmptyPart);
            }
            let modifier = Modifiers::from_name(name)
                .ok_or_else(|| KeyStrokeError::UnknownModifier(name.to_owned()))?;
            if modifiers.contains(modifier) {
                return Err(KeyStrokeError::DuplicateModifier(name.to_owned()));
            }
            modifiers = modifiers | modifier;
        }
        if key_name.is_empty() {
            return Err(KeyStrokeError::EmptyPart);
        }
        if Modifiers::from_name(key_name).is_some() {
            return Err(KeyStrokeError::MissingKey);
        }
        let key = Key::from_name(key_name)
            .ok_or_else(|| KeyStrokeError::UnknownKey(key_name.to_owned()))?;
        Ok(Self { key, modifiers })
    }
}

/// Text that is not a key stroke.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyStrokeError {
    /// Nothing between two `+`, or at either end.
    #[error("a key stroke has an empty part")]
    EmptyPart,
    /// Only modifiers, no key: `cmd+shift`.
    #[error("a key stroke ends with a key, not a modifier")]
    MissingKey,
    /// A modifier name that is not `cmd`, `shift`, `alt`, `ctrl`, or one of their aliases.
    #[error("`{0}` is not a modifier")]
    UnknownModifier(String),
    /// The same modifier twice, even under two aliases.
    #[error("the modifier `{0}` is held twice")]
    DuplicateModifier(String),
    /// A key name Quickie does not know.
    #[error("`{0}` is not a key")]
    UnknownKey(String),
}

/// What a binding listens for: one stroke, or two in a row (`cmd+k cmd+s`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The stroke that starts the chord.
    pub first: KeyStroke,
    /// The stroke that completes a two-stroke chord.
    pub second: Option<KeyStroke>,
}

impl Chord {
    /// The chord of the strokes `cmd+k cmd+s` as `keybindings.json` writes it.
    #[must_use]
    pub fn keybinding_text(&self) -> String {
        match self.second {
            Some(second) => format!(
                "{} {}",
                self.first.keybinding_text(),
                second.keybinding_text()
            ),
            None => self.first.keybinding_text(),
        }
    }
}

impl From<KeyStroke> for Chord {
    fn from(first: KeyStroke) -> Self {
        Self {
            first,
            second: None,
        }
    }
}

impl fmt::Display for Chord {
    /// Both strokes as glyphs, apart: `⌘K ⌘S`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.second {
            Some(second) => write!(f, "{} {second}", self.first),
            None => write!(f, "{}", self.first),
        }
    }
}

impl TryFrom<&str> for Chord {
    type Error = ChordError;

    /// Reads one or two strokes separated by whitespace.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let mut strokes = text.split_whitespace();
        let first = strokes.next().ok_or(ChordError::Empty)?;
        let second = strokes.next();
        if strokes.next().is_some() {
            return Err(ChordError::TooManyStrokes);
        }
        Ok(Self {
            first: KeyStroke::try_from(first)?,
            second: second.map(KeyStroke::try_from).transpose()?,
        })
    }
}

/// Text that is not a chord.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChordError {
    /// No stroke at all.
    #[error("a chord has at least one key stroke")]
    Empty,
    /// More than two strokes.
    #[error("a chord has at most two key strokes")]
    TooManyStrokes,
    /// One of the strokes does not parse.
    #[error(transparent)]
    Stroke(#[from] KeyStrokeError),
}

//! Key strokes and chords, parsed from `keybindings.json` (`mod+shift+p`) and shown as the
//! platform writes them (`⇧⌘P` on macOS, `Ctrl+Shift+P` on Linux).

mod code;
mod modifiers;
mod platform;
#[cfg(test)]
mod tests;

pub use code::{Digit, FunctionKey, Key, Letter};
pub use modifiers::Modifiers;
pub use platform::Platform;

/// The `keybindings.json` name of the primary modifier, which each platform resolves itself.
const PRIMARY: &str = "mod";

/// One key pressed with a set of modifiers held.
///
/// A stroke is always concrete: `mod` is resolved when the stroke is parsed, so comparing two
/// strokes never depends on the platform.
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

    /// Reads `mod+shift+p` on `platform`: `mod` is its primary modifier, every other name is read
    /// as [`KeyStroke::try_from`] reads it.
    ///
    /// # Errors
    ///
    /// [`KeyStrokeError`] says which part is not a stroke.
    pub fn parse(text: &str, platform: Platform) -> Result<Self, KeyStrokeError> {
        Self::read(text, Some(platform.primary()))
    }

    /// The stroke as `keybindings.json` writes it: `ctrl+shift+tab`, modifiers in the order
    /// Control, Option, Shift, Command.
    #[must_use]
    pub fn keybinding_text(&self) -> String {
        let mut parts: Vec<String> = self.modifiers.names().map(str::to_owned).collect();
        parts.push(self.key.name());
        parts.join("+")
    }

    /// The stroke as `platform` prints it beside a menu item: `⌃⇧⇥` and `⌥⌘I` on macOS,
    /// `Ctrl+Shift+Tab` and `Alt+Super+I` on Linux.
    #[must_use]
    pub fn glyphs(self, platform: Platform) -> String {
        let key = self.key.glyph(platform);
        let mut parts: Vec<&str> = self.modifiers.glyphs(platform).collect();
        parts.push(&key);
        match platform {
            Platform::MacOs => parts.concat(),
            Platform::Linux => parts.join("+"),
        }
    }

    /// Reads a stroke whose `mod` means `primary`; without one, `mod` is no modifier at all.
    fn read(text: &str, primary: Option<Modifiers>) -> Result<Self, KeyStrokeError> {
        let modifier = |name: &str| {
            if name.eq_ignore_ascii_case(PRIMARY) {
                primary
            } else {
                Modifiers::from_name(name)
            }
        };
        let mut parts = text.split('+');
        let key_name = parts.next_back().unwrap_or_default();
        let mut modifiers = Modifiers::NONE;
        for name in parts {
            if name.is_empty() {
                return Err(KeyStrokeError::EmptyPart);
            }
            let held =
                modifier(name).ok_or_else(|| KeyStrokeError::UnknownModifier(name.to_owned()))?;
            if modifiers.contains(held) {
                return Err(KeyStrokeError::DuplicateModifier(name.to_owned()));
            }
            modifiers = modifiers | held;
        }
        if key_name.is_empty() {
            return Err(KeyStrokeError::EmptyPart);
        }
        if modifier(key_name).is_some() {
            return Err(KeyStrokeError::MissingKey);
        }
        let key = Key::from_name(key_name)
            .ok_or_else(|| KeyStrokeError::UnknownKey(key_name.to_owned()))?;
        Ok(Self { key, modifiers })
    }
}

impl TryFrom<&str> for KeyStroke {
    type Error = KeyStrokeError;

    /// Reads `cmd+shift+p`: modifiers, then the key, joined by `+`, in any case and any order of
    /// modifiers. Only concrete modifiers: `mod` needs a platform, see [`KeyStroke::parse`].
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        Self::read(text, None)
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
    /// A modifier name that is not `mod`, `cmd`, `shift`, `alt`, `ctrl`, or one of their aliases.
    #[error("`{0}` is not a modifier")]
    UnknownModifier(String),
    /// The same modifier twice, even under two aliases, or as `mod` and the name it stands for.
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
    /// Reads one or two strokes separated by whitespace, `mod` meaning the primary modifier of
    /// `platform`.
    ///
    /// # Errors
    ///
    /// [`ChordError`] says what is wrong with the chord or with one of its strokes.
    pub fn parse(text: &str, platform: Platform) -> Result<Self, ChordError> {
        Self::read(text, |stroke| KeyStroke::parse(stroke, platform))
    }

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

    /// Both strokes as `platform` prints them, apart: `⌘K ⌘S`, `Ctrl+K Ctrl+S`.
    #[must_use]
    pub fn glyphs(&self, platform: Platform) -> String {
        match self.second {
            Some(second) => format!(
                "{} {}",
                self.first.glyphs(platform),
                second.glyphs(platform)
            ),
            None => self.first.glyphs(platform),
        }
    }

    fn read(
        text: &str,
        stroke: impl Fn(&str) -> Result<KeyStroke, KeyStrokeError>,
    ) -> Result<Self, ChordError> {
        let mut strokes = text.split_whitespace();
        let first = strokes.next().ok_or(ChordError::Empty)?;
        let second = strokes.next();
        if strokes.next().is_some() {
            return Err(ChordError::TooManyStrokes);
        }
        Ok(Self {
            first: stroke(first)?,
            second: second.map(stroke).transpose()?,
        })
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

impl TryFrom<&str> for Chord {
    type Error = ChordError;

    /// Reads one or two strokes separated by whitespace, each as [`KeyStroke::try_from`] does.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        Self::read(text, |stroke: &str| KeyStroke::try_from(stroke))
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

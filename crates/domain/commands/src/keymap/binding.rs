//! Key bindings and the user's entries that add or remove them.

use quickie_domain_kernel::{CommandId, CommandIdError};

use super::when::{WhenContext, WhenError, WhenExpr};
use crate::key::{Chord, ChordError, Platform, TypingStroke};

/// A chord that runs a command, while its `when` clause holds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    /// The keys.
    pub chord: Chord,
    /// The command they run.
    pub command: CommandId,
    /// The condition; `None` binds the chord everywhere.
    pub when: Option<WhenExpr>,
}

impl KeyBinding {
    /// Reads a binding from the fields of a `keybindings.json` entry, `mod` in its key meaning
    /// the primary modifier of `platform`.
    ///
    /// # Errors
    ///
    /// [`BindingError`] names the field that does not parse, or [`BindingError::Typing`] when
    /// the key would type into text fields instead of being a shortcut.
    pub fn parse(
        key: &str,
        command: &str,
        when: Option<&str>,
        platform: Platform,
    ) -> Result<Self, BindingError> {
        let binding = Self::read(key, command, when, platform)?;
        binding.chord.check_shortcut()?;
        Ok(binding)
    }

    /// The fields read as they are written, before the rule that a shortcut never types, so a
    /// removal of an old bare key still reads.
    fn read(
        key: &str,
        command: &str,
        when: Option<&str>,
        platform: Platform,
    ) -> Result<Self, BindingError> {
        Ok(Self {
            chord: Chord::parse(key, platform)?,
            command: CommandId::try_from(command)?,
            when: when.map(WhenExpr::try_from).transpose()?,
        })
    }

    /// Whether the clause of this binding holds in `context`; a binding without one always does.
    pub(super) fn applies(&self, context: &WhenContext) -> bool {
        self.when.as_ref().is_none_or(|when| when.evaluate(context))
    }
}

/// A user entry `-<command>` that removes default bindings of a command.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Unbinding {
    /// The chord of the default bindings to remove.
    pub chord: Chord,
    /// The command whose default bindings go.
    pub command: CommandId,
    /// Removes only the default with this clause; `None` removes the chord under any clause.
    pub when: Option<WhenExpr>,
}

impl Unbinding {
    /// Whether `binding` is one of the defaults this entry removes.
    pub(super) fn removes(&self, binding: &KeyBinding) -> bool {
        self.command == binding.command
            && self.chord == binding.chord
            && self
                .when
                .as_ref()
                .is_none_or(|when| binding.when.as_ref() == Some(when))
    }
}

/// One entry of the user's `keybindings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeymapEntry {
    /// Adds a binding after the defaults, so it wins over them.
    Bind(KeyBinding),
    /// Removes default bindings: the command is written `-<id>`.
    Unbind(Unbinding),
}

impl KeymapEntry {
    /// Reads an entry from the fields of a `keybindings.json` entry on `platform`; a command
    /// written `-<id>` removes instead of adding.
    ///
    /// A removal may name a key that types (`f`, from before shortcuts needed `⌘`, `⌥` or
    /// `⌃`): no binding has such a key any more, so it removes nothing, and a file written
    /// back then still loads.
    ///
    /// # Errors
    ///
    /// [`BindingError`] names the field that does not parse; an added binding whose key would
    /// type is [`BindingError::Typing`].
    pub fn parse(
        key: &str,
        command: &str,
        when: Option<&str>,
        platform: Platform,
    ) -> Result<Self, BindingError> {
        match command.strip_prefix('-') {
            Some(removed) => {
                let KeyBinding {
                    chord,
                    command,
                    when,
                } = KeyBinding::read(key, removed, when, platform)?;
                Ok(Self::Unbind(Unbinding {
                    chord,
                    command,
                    when,
                }))
            }
            None => KeyBinding::parse(key, command, when, platform).map(Self::Bind),
        }
    }

    /// The command the entry adds or removes.
    #[must_use]
    pub fn command(&self) -> &CommandId {
        match self {
            Self::Bind(binding) => &binding.command,
            Self::Unbind(unbinding) => &unbinding.command,
        }
    }
}

/// A `keybindings.json` entry with a field that does not parse.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    /// The `key` field.
    #[error("key: {0}")]
    Chord(#[from] ChordError),
    /// The `command` field.
    #[error("command: {0}")]
    Command(#[from] CommandIdError),
    /// The `when` field.
    #[error("when: {0}")]
    When(#[from] WhenError),
    /// The `key` field parses, but its first stroke types a character: it cannot be a shortcut.
    #[error("key: {0}")]
    Typing(#[from] TypingStroke),
}

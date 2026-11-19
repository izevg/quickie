//! Identifiers shared across bounded contexts.

use std::fmt;

/// Identifies a tab for its whole life, across restarts and the trash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(u64);

impl TabId {
    /// Wraps a raw value, as read from storage or allocated by a sequence.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for storage.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tab#{}", self.0)
    }
}

/// Identifies a browser window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// Wraps a raw value, as read from storage or allocated by a sequence.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for storage.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window#{}", self.0)
    }
}

/// Names a command for good: `quickie.<area>.<action>`, such as `quickie.tab.reopenClosed`.
///
/// The keymap, the palette, and `keybindings.json` all refer to commands by this id, so it is
/// validated once, at the boundary, and stays stable when a title changes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandId(String);

impl CommandId {
    /// The namespace every command id starts with.
    pub const NAMESPACE: &'static str = "quickie";

    /// The id as written in `keybindings.json`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for CommandId {
    type Error = CommandIdError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let rest = text
            .strip_prefix(Self::NAMESPACE)
            .and_then(|rest| rest.strip_prefix('.'))
            .ok_or(CommandIdError::ForeignNamespace)?;
        let (area, action) = rest.split_once('.').ok_or(CommandIdError::Shape)?;
        if action.contains('.') {
            return Err(CommandIdError::Shape);
        }
        if !is_area(area) {
            return Err(CommandIdError::Area(area.to_owned()));
        }
        if !is_action(action) {
            return Err(CommandIdError::Action(action.to_owned()));
        }
        Ok(Self(text.to_owned()))
    }
}

/// Lowercase ASCII, starting with a letter: `tab`, `devtools`.
fn is_area(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// lowerCamel ASCII, starting with a lowercase letter: `reopenClosed`, `goToSlot1`.
fn is_action(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Text that is not a command id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommandIdError {
    /// The id does not start with `quickie.`.
    #[error("a command id starts with `quickie.`")]
    ForeignNamespace,
    /// The id does not have exactly an area and an action after the namespace.
    #[error("a command id has the shape `quickie.<area>.<action>`")]
    Shape,
    /// The area is not lowercase ASCII starting with a letter.
    #[error("the area `{0}` is not lowercase ASCII starting with a letter")]
    Area(String),
    /// The action is not lowerCamel ASCII starting with a lowercase letter.
    #[error("the action `{0}` is not lowerCamel ASCII starting with a lowercase letter")]
    Action(String),
}

#[cfg(test)]
mod tests {
    use super::{CommandId, CommandIdError, TabId, WindowId};

    #[test]
    fn tab_id_round_trips_and_displays() {
        let id = TabId::new(42);

        assert_eq!(id.get(), 42);
        assert_eq!(id.to_string(), "tab#42");
    }

    #[test]
    fn window_id_round_trips_and_displays() {
        let id = WindowId::new(7);

        assert_eq!(id.get(), 7);
        assert_eq!(id.to_string(), "window#7");
    }

    fn parse(text: &str) -> Result<CommandId, CommandIdError> {
        CommandId::try_from(text)
    }

    #[test]
    fn a_well_formed_id_keeps_its_text() {
        let id = parse("quickie.tab.reopenClosed").unwrap();

        assert_eq!(id.as_str(), "quickie.tab.reopenClosed");
        assert_eq!(id.to_string(), "quickie.tab.reopenClosed");
    }

    #[test]
    fn digits_follow_the_first_letter_of_area_and_action() {
        let id = parse("quickie.tab2.goToSlot1");

        assert!(id.is_ok());
    }

    #[test]
    fn another_namespace_is_rejected() {
        let error = parse("vscode.tab.close").unwrap_err();

        assert_eq!(error, CommandIdError::ForeignNamespace);
    }

    #[test]
    fn the_namespace_needs_its_dot() {
        let error = parse("quickietab.close").unwrap_err();

        assert_eq!(error, CommandIdError::ForeignNamespace);
    }

    #[test]
    fn an_id_without_an_action_is_misshapen() {
        let error = parse("quickie.tab").unwrap_err();

        assert_eq!(error, CommandIdError::Shape);
    }

    #[test]
    fn an_id_with_a_fourth_part_is_misshapen() {
        let error = parse("quickie.tab.close.now").unwrap_err();

        assert_eq!(error, CommandIdError::Shape);
    }

    #[test]
    fn an_empty_area_is_rejected() {
        let error = parse("quickie..close").unwrap_err();

        assert_eq!(error, CommandIdError::Area(String::new()));
    }

    #[test]
    fn an_uppercase_area_is_rejected() {
        let error = parse("quickie.Tab.close").unwrap_err();

        assert_eq!(error, CommandIdError::Area("Tab".to_owned()));
    }

    #[test]
    fn an_area_starting_with_a_digit_is_rejected() {
        let error = parse("quickie.1tab.close").unwrap_err();

        assert_eq!(error, CommandIdError::Area("1tab".to_owned()));
    }

    #[test]
    fn an_area_with_an_uppercase_letter_inside_is_rejected() {
        let error = parse("quickie.devTools.console").unwrap_err();

        assert_eq!(error, CommandIdError::Area("devTools".to_owned()));
    }

    #[test]
    fn an_area_with_punctuation_inside_is_rejected() {
        let error = parse("quickie.dev-tools.console").unwrap_err();

        assert_eq!(error, CommandIdError::Area("dev-tools".to_owned()));
    }

    #[test]
    fn an_empty_action_is_rejected() {
        let error = parse("quickie.tab.").unwrap_err();

        assert_eq!(error, CommandIdError::Action(String::new()));
    }

    #[test]
    fn an_action_in_upper_camel_case_is_rejected() {
        let error = parse("quickie.tab.ReopenClosed").unwrap_err();

        assert_eq!(error, CommandIdError::Action("ReopenClosed".to_owned()));
    }

    #[test]
    fn an_action_starting_with_a_digit_is_rejected() {
        let error = parse("quickie.tab.1slot").unwrap_err();

        assert_eq!(error, CommandIdError::Action("1slot".to_owned()));
    }

    #[test]
    fn an_action_with_an_underscore_is_rejected() {
        let error = parse("quickie.tab.reopen_closed").unwrap_err();

        assert_eq!(error, CommandIdError::Action("reopen_closed".to_owned()));
    }

    #[test]
    fn errors_explain_the_expected_shape() {
        let message = CommandIdError::Area("Tab".to_owned()).to_string();

        assert_eq!(
            message,
            "the area `Tab` is not lowercase ASCII starting with a letter"
        );
    }
}

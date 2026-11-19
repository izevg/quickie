//! The keyword after `!` that picks a search shortcut.

use std::borrow::Borrow;
use std::fmt;

/// A search shortcut's keyword, typed after `!`: ASCII lowercase letters only, 1 to
/// [`Keyword::MAX_LEN`] of them, so it is quick to type and never clashes with the terms.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Keyword(String);

impl Keyword {
    /// The longest keyword, in letters.
    pub const MAX_LEN: usize = 16;

    /// The keyword as typed, without the `!`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lets the shortcuts be looked up by the text the palette parsed, without validating it first.
impl Borrow<str> for Keyword {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "!{}", self.0)
    }
}

/// Why a text is not a keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeywordError {
    /// The text is empty.
    #[error("a keyword needs at least one letter")]
    Empty,
    /// The text is longer than [`Keyword::MAX_LEN`]; holds its length in characters.
    #[error("a keyword has at most 16 letters, not {0}")]
    TooLong(usize),
    /// The text has a character other than `a` to `z`.
    #[error("a keyword has only the letters a to z, not {0:?}")]
    NotALetter(char),
}

impl TryFrom<&str> for Keyword {
    type Error = KeywordError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        if let Some(c) = text.chars().find(|c| !c.is_ascii_lowercase()) {
            return Err(KeywordError::NotALetter(c));
        }
        // Only ASCII is left, so bytes are characters.
        match text.len() {
            0 => Err(KeywordError::Empty),
            len if len > Self::MAX_LEN => Err(KeywordError::TooLong(len)),
            _ => Ok(Self(text.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Keyword, KeywordError};

    #[test]
    fn lowercase_letters_make_a_keyword() {
        let keyword = Keyword::try_from("gh");

        assert_eq!(keyword.as_ref().map(Keyword::as_str), Ok("gh"));
    }

    #[test]
    fn a_keyword_displays_with_its_bang() {
        let keyword = Keyword::try_from("gh").unwrap();

        assert_eq!(keyword.to_string(), "!gh");
    }

    #[test]
    fn one_letter_is_enough() {
        assert!(Keyword::try_from("w").is_ok());
    }

    #[test]
    fn sixteen_letters_are_the_most() {
        let text = "a".repeat(16);

        assert!(Keyword::try_from(text.as_str()).is_ok());
    }

    #[test]
    fn seventeen_letters_are_too_long() {
        let text = "a".repeat(17);

        assert_eq!(
            Keyword::try_from(text.as_str()),
            Err(KeywordError::TooLong(17))
        );
    }

    #[test]
    fn an_empty_text_is_not_a_keyword() {
        assert_eq!(Keyword::try_from(""), Err(KeywordError::Empty));
    }

    #[test]
    fn anything_but_a_to_z_is_rejected() {
        for (text, bad) in [("Gh", 'G'), ("g2", '2'), ("wé", 'é'), ("g h", ' ')] {
            assert_eq!(Keyword::try_from(text), Err(KeywordError::NotALetter(bad)));
        }
    }
}

//! A tag: the only way bookmarks are grouped.

use std::fmt;

/// A bookmark tag, typed after `#` in the palette: lowercase, without whitespace or `#`, 1 to
/// [`Tag::MAX_CHARS`] characters.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(String);

impl Tag {
    /// The longest tag, in characters.
    pub const MAX_CHARS: usize = 32;

    /// `later`: "read later" is this tag, not a separate list.
    #[must_use]
    pub fn later() -> Self {
        Self("later".to_owned())
    }

    /// The tag as typed, without the `#`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Why a text is not a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    /// The text is empty.
    #[error("a tag needs at least one character")]
    Empty,
    /// The text is longer than [`Tag::MAX_CHARS`]; holds its length in characters.
    #[error("a tag has at most 32 characters, not {0}")]
    TooLong(usize),
    /// The text has an uppercase letter.
    #[error("a tag is lowercase, but has {0:?}")]
    Uppercase(char),
    /// The text has whitespace: a space would end the tag in the palette.
    #[error("a tag has no whitespace")]
    Whitespace,
    /// The text has a `#`, which only introduces a tag.
    #[error("a tag has no '#'")]
    Hash,
}

impl TryFrom<&str> for Tag {
    type Error = TagError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let len = text.chars().count();
        if len == 0 {
            return Err(TagError::Empty);
        }
        if len > Self::MAX_CHARS {
            return Err(TagError::TooLong(len));
        }
        for c in text.chars() {
            if c == '#' {
                return Err(TagError::Hash);
            }
            if c.is_whitespace() {
                return Err(TagError::Whitespace);
            }
            if c.is_uppercase() {
                return Err(TagError::Uppercase(c));
            }
        }
        Ok(Self(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Tag, TagError};

    #[test]
    fn a_lowercase_word_is_a_tag() {
        let tag = Tag::try_from("rust-lang");

        assert_eq!(tag.as_ref().map(Tag::as_str), Ok("rust-lang"));
    }

    #[test]
    fn a_tag_displays_with_its_hash() {
        let tag = Tag::try_from("rust").unwrap();

        assert_eq!(tag.to_string(), "#rust");
    }

    #[test]
    fn later_is_an_ordinary_tag() {
        assert_eq!(Tag::try_from("later"), Ok(Tag::later()));
    }

    #[test]
    fn one_character_is_enough() {
        assert!(Tag::try_from("a").is_ok());
    }

    #[test]
    fn an_empty_text_is_not_a_tag() {
        assert_eq!(Tag::try_from(""), Err(TagError::Empty));
    }

    #[test]
    fn length_counts_characters_up_to_thirty_two() {
        let longest = "ü".repeat(32);

        assert!(Tag::try_from(longest.as_str()).is_ok());
    }

    #[test]
    fn thirty_three_characters_are_too_long() {
        let text = "a".repeat(33);

        assert_eq!(Tag::try_from(text.as_str()), Err(TagError::TooLong(33)));
    }

    #[test]
    fn uppercase_is_rejected() {
        assert_eq!(Tag::try_from("readLater"), Err(TagError::Uppercase('L')));
    }

    #[test]
    fn whitespace_is_rejected() {
        assert_eq!(Tag::try_from("read later"), Err(TagError::Whitespace));
        assert_eq!(Tag::try_from("read\tlater"), Err(TagError::Whitespace));
    }

    #[test]
    fn a_hash_is_rejected() {
        assert_eq!(Tag::try_from("#later"), Err(TagError::Hash));
    }
}

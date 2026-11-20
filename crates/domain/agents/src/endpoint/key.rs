//! The endpoint's API key: the secret itself, and the reference under which the Keychain keeps
//! it.

use std::fmt;

use super::BaseUrl;

/// Names an API key in the Keychain: the origin of the endpoint it was saved for, such as
/// `https://api.openai.com`. The store keeps only this, never the key.
///
/// Deriving the name from the origin binds a key to its host: an endpoint moved to another
/// origin looks under another name and finds nothing to send.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyRef(String);

impl KeyRef {
    /// The reference of the key for an endpoint's origin.
    #[must_use]
    pub fn for_origin(base: &BaseUrl) -> Self {
        Self(base.as_url().origin().ascii_serialization())
    }

    /// The reference as the Keychain account name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An API key, as typed into settings or read from the Keychain. It goes into an
/// `Authorization` header, so it is visible ASCII only: a line break could not smuggle in a
/// header of its own. `Debug` hides it, and it has no `Display`, so it does not end up in a log.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// The key itself, for the one place that sends it.
    #[must_use]
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(hidden)")
    }
}

/// Why a text is not an API key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApiKeyError {
    /// Nothing but whitespace.
    #[error("an API key is not empty")]
    Empty,
    /// A character that is not visible ASCII: a space, a line break, a letter outside ASCII.
    #[error("an API key is visible ASCII only")]
    NotVisibleAscii,
}

impl TryFrom<&str> for ApiKey {
    type Error = ApiKeyError;

    /// Parses a key as pasted; surrounding whitespace, such as a trailing line break, is ignored.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ApiKeyError::Empty);
        }
        if !text.chars().all(|c| c.is_ascii_graphic()) {
            return Err(ApiKeyError::NotVisibleAscii);
        }
        Ok(Self(text.to_owned()))
    }
}

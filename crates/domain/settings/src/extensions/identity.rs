//! What names an extension: its id, its version, and where it came from.

use std::cmp::Ordering;
use std::fmt;

use url::Url;

/// An extension's id as Chrome writes it: 32 letters `a` to `p`, each one half of a byte of a
/// SHA-256 digest (of the developer's public key for a package from the Chrome Web Store, of
/// where it was found for one from disk). The same id keeps an extension's storage and the
/// user's choices across updates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(String);

/// Text that is not an extension id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not an extension id: 32 letters a to p")]
pub struct ExtensionIdError(pub String);

impl ExtensionId {
    /// How many letters an id has.
    pub const LEN: usize = 32;

    /// The id the first 16 bytes of `digest` make: each byte's high half, then its low half,
    /// as `a` (0) to `p` (15).
    #[must_use]
    pub fn from_digest(digest: &[u8; 16]) -> Self {
        let letter = |half: u8| char::from(b'a' + half);
        Self(
            digest
                .iter()
                .flat_map(|byte| [letter(byte >> 4), letter(byte & 0x0f)])
                .collect(),
        )
    }

    /// The id a Chrome Web Store page names: the last segment of its path on
    /// `chromewebstore.google.com/detail/…` or `chrome.google.com/webstore/detail/…`. `None`
    /// for any other page.
    #[must_use]
    pub fn from_store_url(url: &Url) -> Option<Self> {
        if url.scheme() != "https" {
            return None;
        }
        let segments: Vec<&str> = url
            .path_segments()?
            .filter(|segment| !segment.is_empty())
            .collect();
        let prefix: &[&str] = match url.host_str()? {
            "chromewebstore.google.com" => &["detail"],
            "chrome.google.com" => &["webstore", "detail"],
            _ => return None,
        };
        let rest = segments.strip_prefix(prefix)?;
        Self::try_from(*rest.last()?).ok()
    }

    /// The id as Chrome writes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ExtensionId {
    type Error = ExtensionIdError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        if text.len() == Self::LEN && text.bytes().all(|byte| (b'a'..=b'p').contains(&byte)) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ExtensionIdError(text.to_owned()))
        }
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An extension's version as its manifest gives it: one to four numbers from 0 to 65535,
/// separated by dots, without leading zeros. Versions compare number by number, a missing
/// number counting as 0, so `1.2` and `1.2.0` are the same version.
#[derive(Clone, Debug)]
pub struct ExtensionVersion {
    parts: [u16; 4],
    text: String,
}

/// Text that is not an extension version.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not an extension version: one to four numbers separated by dots")]
pub struct VersionError(pub String);

impl ExtensionVersion {
    /// The version as the manifest writes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl TryFrom<&str> for ExtensionVersion {
    type Error = VersionError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let fail = || VersionError(text.to_owned());
        let mut parts = [0_u16; 4];
        for (at, piece) in text.split('.').enumerate() {
            let slot = parts.get_mut(at).ok_or_else(fail)?;
            let digits = !piece.is_empty() && piece.bytes().all(|byte| byte.is_ascii_digit());
            if !digits || (piece.len() > 1 && piece.starts_with('0')) {
                return Err(fail());
            }
            *slot = piece.parse().map_err(|_| fail())?;
        }
        Ok(Self {
            parts,
            text: text.to_owned(),
        })
    }
}

impl PartialEq for ExtensionVersion {
    fn eq(&self, other: &Self) -> bool {
        self.parts == other.parts
    }
}

impl Eq for ExtensionVersion {}

impl PartialOrd for ExtensionVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExtensionVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts.cmp(&other.parts)
    }
}

impl fmt::Display for ExtensionVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Where an installed extension came from, which decides how it updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtensionSource {
    /// The Chrome Web Store: updates come from there, on request or when the user lets them.
    ChromeWebStore,
    /// A package file the user chose (`.crx` or `.zip`): it updates when installed again.
    File,
    /// A folder the user chose, loaded where it is: it changes with the folder.
    Folder,
}

impl ExtensionSource {
    /// How the store writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChromeWebStore => "chrome_web_store",
            Self::File => "file",
            Self::Folder => "folder",
        }
    }

    /// The source the store wrote, `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::ChromeWebStore, Self::File, Self::Folder]
            .into_iter()
            .find(|source| source.as_str() == text)
    }

    /// What the Settings pane calls it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ChromeWebStore => "Chrome Web Store",
            Self::File => "Package file",
            Self::Folder => "Folder",
        }
    }
}

//! The allowed filter list sources. Quickie fetches a list only from here: a list is chosen by
//! its id, never by a URL the user or a page typed.

use std::fmt;

/// A filter list Quickie can block with, named by where it comes from. Each is fetched over
/// https from its maintainer; none ships with Quickie, as their licenses differ from ours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilterListId {
    /// EasyList: ads.
    EasyList,
    /// EasyPrivacy: trackers.
    EasyPrivacy,
    /// uBlock filters: what uBlock Origin adds on top of EasyList.
    UblockFilters,
    /// Peter Lowe's list: ad and tracking servers.
    PeterLowe,
}

impl FilterListId {
    /// Every allowed source, in the order the shields popover lists them.
    pub const ALL: [Self; 4] = [
        Self::EasyList,
        Self::EasyPrivacy,
        Self::UblockFilters,
        Self::PeterLowe,
    ];

    /// The id as stored and as typed: `easylist`, `easyprivacy`, `ublock-filters`, `peter-lowe`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EasyList => "easylist",
            Self::EasyPrivacy => "easyprivacy",
            Self::UblockFilters => "ublock-filters",
            Self::PeterLowe => "peter-lowe",
        }
    }

    /// What the shields popover calls the list.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::EasyList => "EasyList",
            Self::EasyPrivacy => "EasyPrivacy",
            Self::UblockFilters => "uBlock filters",
            Self::PeterLowe => "Peter Lowe's list",
        }
    }

    /// Where the list is fetched from: always https, always its maintainer's own address.
    #[must_use]
    pub const fn source(self) -> &'static str {
        match self {
            Self::EasyList => "https://easylist.to/easylist/easylist.txt",
            Self::EasyPrivacy => "https://easylist.to/easylist/easyprivacy.txt",
            Self::UblockFilters => "https://ublockorigin.github.io/uAssets/filters/filters.txt",
            Self::PeterLowe => {
                "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext"
            }
        }
    }

    /// Where the list sits in [`Self::ALL`], which is declaration order.
    pub(super) const fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for FilterListId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A text that names no allowed filter list.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not an allowed filter list")]
pub struct UnknownList(pub String);

impl TryFrom<&str> for FilterListId {
    type Error = UnknownList;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|id| id.as_str() == text)
            .ok_or_else(|| UnknownList(text.to_owned()))
    }
}

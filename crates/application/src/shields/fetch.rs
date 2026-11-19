//! Fetching filter lists: what to fetch, the fetching itself (on a worker thread), and what
//! came of it.

use quickie_domain_shields::FilterListId;
use url::Url;

use crate::{HttpError, HttpPort};

/// A filter list to fetch, and from where: always its allowed source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListFetch {
    /// Which list.
    pub list: FilterListId,
    /// Its source.
    pub url: Url,
}

impl ListFetch {
    /// Fetches of these lists, each from its source.
    pub(super) fn of(lists: impl IntoIterator<Item = FilterListId>) -> Vec<Self> {
        lists
            .into_iter()
            .filter_map(|list| {
                Some(Self {
                    list,
                    url: Url::parse(list.source()).ok()?,
                })
            })
            .collect()
    }
}

/// What a fetch brought: the list's content, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedList {
    /// Which list.
    pub list: FilterListId,
    /// The content, or the failure.
    pub body: Result<Vec<u8>, HttpError>,
}

/// Fetches each list in turn. Blocks on the network: the shell runs it on a worker thread and
/// hands the result to `Shields::lists_fetched` back on the main thread.
pub fn fetch_lists(http: &impl HttpPort, fetches: &[ListFetch]) -> Vec<FetchedList> {
    fetches
        .iter()
        .map(|fetch| FetchedList {
            list: fetch.list,
            body: http.get(&fetch.url),
        })
        .collect()
}

/// What a batch of fetches changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// Lists whose content is new: fetched for the first time, or changed.
    pub updated: Vec<FilterListId>,
    /// Lists that came back as they were.
    pub unchanged: Vec<FilterListId>,
    /// Lists that could not be fetched; they keep their previous content.
    pub failed: Vec<(FilterListId, HttpError)>,
}

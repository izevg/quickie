//! Downloads: what the engine is fetching to disk, or fetched. The engine moves the bytes; this
//! list is what the downloads sheet shows and what survives a restart.

mod snapshot;
#[cfg(test)]
mod tests;

use std::fmt;

use indexmap::IndexMap;
use quickie_domain_kernel::Timestamp;
use url::Url;

pub use snapshot::{CorruptDownloads, DownloadsSnapshot};

/// Identifies a download for as long as it is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DownloadId(u64);

impl DownloadId {
    /// Wraps a raw value, as read from storage.
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

impl fmt::Display for DownloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "download#{}", self.0)
    }
}

/// Where a download is. Only a download in progress moves; the other three are where it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    /// Bytes are arriving.
    InProgress {
        /// Bytes received so far.
        received: u64,
        /// The size the server announced, if it did.
        total: Option<u64>,
    },
    /// Every byte arrived and the file is in place.
    Finished,
    /// The engine gave up; holds its reason, as the sheet shows it.
    Failed(String),
    /// The person stopped it.
    Cancelled,
}

/// One download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// Never reused within a list.
    pub id: DownloadId,
    /// Where the file comes from.
    pub url: Url,
    /// The file's name on disk.
    pub file_name: String,
    /// Where it is.
    pub state: DownloadState,
    /// When it started.
    pub started_at: Timestamp,
}

/// A command a download cannot take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DownloadError {
    /// No listed download has this id.
    #[error("no {0} is listed")]
    Unknown(DownloadId),
    /// The download already finished, failed or was cancelled, so it no longer moves.
    #[error("{0} is no longer in progress")]
    Ended(DownloadId),
}

/// The downloads of one profile, in the order they started.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Downloads {
    downloads: IndexMap<DownloadId, Download>,
    next_id: u64,
}

impl Downloads {
    /// No downloads.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lists a download the engine just started, with nothing received yet.
    pub fn start(&mut self, url: Url, file_name: String, now: Timestamp) -> DownloadId {
        let id = DownloadId::new(self.next_id);
        self.next_id += 1;
        self.downloads.insert(
            id,
            Download {
                id,
                url,
                file_name,
                state: DownloadState::InProgress {
                    received: 0,
                    total: None,
                },
                started_at: now,
            },
        );
        id
    }

    /// Records the bytes received so far and the size, as the engine now reports them.
    ///
    /// # Errors
    ///
    /// [`DownloadError::Unknown`] for a download not listed, [`DownloadError::Ended`] for one no
    /// longer in progress.
    pub fn progress(
        &mut self,
        id: DownloadId,
        received: u64,
        total: Option<u64>,
    ) -> Result<(), DownloadError> {
        self.move_to(id, DownloadState::InProgress { received, total })
    }

    /// Marks a download finished.
    ///
    /// # Errors
    ///
    /// [`DownloadError::Unknown`] for a download not listed, [`DownloadError::Ended`] for one no
    /// longer in progress.
    pub fn finish(&mut self, id: DownloadId) -> Result<(), DownloadError> {
        self.move_to(id, DownloadState::Finished)
    }

    /// Marks a download failed, with the engine's reason.
    ///
    /// # Errors
    ///
    /// [`DownloadError::Unknown`] for a download not listed, [`DownloadError::Ended`] for one no
    /// longer in progress.
    pub fn fail(&mut self, id: DownloadId, reason: String) -> Result<(), DownloadError> {
        self.move_to(id, DownloadState::Failed(reason))
    }

    /// Marks a download cancelled; the engine stops it.
    ///
    /// # Errors
    ///
    /// [`DownloadError::Unknown`] for a download not listed, [`DownloadError::Ended`] for one no
    /// longer in progress.
    pub fn cancel(&mut self, id: DownloadId) -> Result<(), DownloadError> {
        self.move_to(id, DownloadState::Cancelled)
    }

    /// Clears every download that ended, whether it finished, failed or was cancelled, and
    /// returns them in the order they started. Downloads in progress stay.
    pub fn clear_finished(&mut self) -> Vec<Download> {
        let (running, ended): (IndexMap<_, _>, IndexMap<_, _>) =
            std::mem::take(&mut self.downloads)
                .into_iter()
                .partition(|(_, download)| {
                    matches!(download.state, DownloadState::InProgress { .. })
                });
        self.downloads = running;
        ended.into_values().collect()
    }

    /// A listed download.
    #[must_use]
    pub fn get(&self, id: DownloadId) -> Option<&Download> {
        self.downloads.get(&id)
    }

    /// Every listed download, in the order they started.
    pub fn all(&self) -> impl Iterator<Item = &Download> {
        self.downloads.values()
    }

    fn move_to(&mut self, id: DownloadId, state: DownloadState) -> Result<(), DownloadError> {
        let download = self
            .downloads
            .get_mut(&id)
            .ok_or(DownloadError::Unknown(id))?;
        if !matches!(download.state, DownloadState::InProgress { .. }) {
            return Err(DownloadError::Ended(id));
        }
        download.state = state;
        Ok(())
    }
}

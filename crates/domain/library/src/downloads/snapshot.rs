//! Downloads as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;

use super::{Download, DownloadId, Downloads};

/// The whole download list as plain data, for the store to write and read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadsSnapshot {
    /// Downloads in the order they started.
    pub downloads: Vec<Download>,
    /// The next free [`DownloadId`]; ids are never reused.
    pub next_id: u64,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptDownloads {
    /// Two downloads share an id.
    #[error("{0} appears twice")]
    DuplicateDownload(DownloadId),
    /// An id at or above the counter: it could be handed out again.
    #[error("{0} is not below the download counter")]
    IdNotAllocated(DownloadId),
}

impl Downloads {
    /// Rebuilds the list from a snapshot, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptDownloads`] found.
    pub fn restore(snapshot: DownloadsSnapshot) -> Result<Self, CorruptDownloads> {
        let DownloadsSnapshot { downloads, next_id } = snapshot;
        let mut listed = IndexMap::with_capacity(downloads.len());
        for download in downloads {
            let id = download.id;
            if id.get() >= next_id {
                return Err(CorruptDownloads::IdNotAllocated(id));
            }
            if listed.insert(id, download).is_some() {
                return Err(CorruptDownloads::DuplicateDownload(id));
            }
        }
        Ok(Self {
            downloads: listed,
            next_id,
        })
    }

    /// The list as plain data; `restore(snapshot())` gives back an equal list.
    #[must_use]
    pub fn snapshot(&self) -> DownloadsSnapshot {
        DownloadsSnapshot {
            downloads: self.downloads.values().cloned().collect(),
            next_id: self.next_id,
        }
    }
}

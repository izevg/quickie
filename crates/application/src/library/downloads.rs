//! Download use cases: the engine reports a download's start, progress and end, and the user
//! cancels, retries and clears them.
//!
//! The list saves when a download starts and when it ends, never on progress: progress arrives
//! several times a second and is worth nothing after a restart, where an unfinished download
//! has failed anyway.
//!
//! **Throwaway jars.** A download from a throwaway jar's page still saves its file to the
//! Downloads folder, where the user asked for it. It is listed while Quickie runs, so it can be
//! watched, cancelled, and opened, but it is never written to the store: once Quickie quits, the
//! library keeps no trace of where the file came from, as with the jar's visits.

#[cfg(test)]
mod tests;

use quickie_domain_browsing::Jar;
use quickie_domain_library::{
    Download, DownloadError, DownloadId, DownloadState, Downloads, DownloadsSnapshot,
};
use url::Url;

use super::Library;
use crate::{AppError, Clock, LibraryRepo};

/// Why a download listed as in progress at start failed.
const INTERRUPTED: &str = "Quickie quit before it finished";

/// Fails every download still in progress: at start, nothing is fetching any more.
pub(super) fn interrupt(downloads: &mut Downloads) {
    let running: Vec<DownloadId> = downloads
        .all()
        .filter(|download| matches!(download.state, DownloadState::InProgress { .. }))
        .map(|download| download.id)
        .collect();
    for id in running {
        // Each of them is listed and in progress, so this cannot fail.
        let _ = downloads.fail(id, INTERRUPTED.to_owned());
    }
}

impl<R: LibraryRepo, C: Clock> Library<R, C> {
    /// The downloads, for reading: those saved before, and this session's.
    #[must_use]
    pub fn downloads(&self) -> &Downloads {
        &self.downloads
    }

    /// Whether a download is a throwaway jar's, listed only until Quickie quits.
    #[must_use]
    pub fn is_session_only(&self, id: DownloadId) -> bool {
        self.session_only.contains(&id)
    }

    /// Lists a download the engine started into `file_name`, from a page in `jar`. A main-jar
    /// download is saved; a throwaway jar's is only listed.
    ///
    /// # Errors
    ///
    /// The store failed; nothing is listed.
    pub fn download_started(
        &mut self,
        url: Url,
        file_name: String,
        jar: Jar,
    ) -> Result<DownloadId, AppError> {
        let now = self.clock.now();
        let mut next = self.downloads.clone();
        let id = next.start(url, file_name, now);
        if let Jar::Throwaway(_) = jar {
            // Marked before the commit, which then has nothing to save.
            self.session_only.insert(id);
        }
        self.commit_downloads(next)?;
        Ok(id)
    }

    /// Records the bytes received so far and the size; kept in memory only.
    ///
    /// # Errors
    ///
    /// The download is not listed or no longer in progress.
    pub fn download_progressed(
        &mut self,
        id: DownloadId,
        received: u64,
        total: Option<u64>,
    ) -> Result<(), AppError> {
        Ok(self.downloads.progress(id, received, total)?)
    }

    /// Every byte arrived.
    ///
    /// # Errors
    ///
    /// The download is not listed or no longer in progress, or the store failed.
    pub fn download_finished(&mut self, id: DownloadId) -> Result<(), AppError> {
        self.end_download(|next| next.finish(id))
    }

    /// The engine gave up, for `reason`.
    ///
    /// # Errors
    ///
    /// The download is not listed or no longer in progress, or the store failed.
    pub fn download_failed(&mut self, id: DownloadId, reason: String) -> Result<(), AppError> {
        self.end_download(|next| next.fail(id, reason))
    }

    /// The user stopped a download; the shell stops the engine's.
    ///
    /// # Errors
    ///
    /// The download is not listed or no longer in progress, or the store failed.
    pub fn cancel_download(&mut self, id: DownloadId) -> Result<(), AppError> {
        self.end_download(|next| next.cancel(id))
    }

    /// Takes an ended download off the list so it can be fetched again; returns it. The new
    /// fetch is listed when the engine starts it.
    ///
    /// # Errors
    ///
    /// The download is not listed or still in progress, or the store failed.
    pub fn retry_download(&mut self, id: DownloadId) -> Result<Download, AppError> {
        let mut next = self.downloads.clone();
        let removed = next.remove(id)?;
        self.commit_downloads(next)?;
        self.session_only.remove(&id);
        Ok(removed)
    }

    /// Clears every ended download from the list, not from the disk; returns them.
    ///
    /// # Errors
    ///
    /// The store failed; nothing is cleared.
    pub fn clear_finished_downloads(&mut self) -> Result<Vec<Download>, AppError> {
        let mut next = self.downloads.clone();
        let cleared = next.clear_finished();
        self.commit_downloads(next)?;
        for download in &cleared {
            self.session_only.remove(&download.id);
        }
        Ok(cleared)
    }

    fn end_download(
        &mut self,
        end: impl FnOnce(&mut Downloads) -> Result<(), DownloadError>,
    ) -> Result<(), AppError> {
        let mut next = self.downloads.clone();
        end(&mut next)?;
        self.commit_downloads(next)
    }

    /// Saves `next` when what is saved of it changed, then makes it the list.
    fn commit_downloads(&mut self, next: Downloads) -> Result<(), AppError> {
        let saved = self.saved_part(&next);
        if saved.downloads != self.saved_part(&self.downloads).downloads {
            self.repo.save_downloads(&saved)?;
        }
        self.downloads = next;
        Ok(())
    }

    /// What of `downloads` is saved: every download but the throwaway jars'.
    fn saved_part(&self, downloads: &Downloads) -> DownloadsSnapshot {
        let mut snapshot = downloads.snapshot();
        snapshot
            .downloads
            .retain(|download| !self.session_only.contains(&download.id));
        snapshot
    }
}

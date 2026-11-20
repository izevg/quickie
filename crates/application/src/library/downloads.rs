//! Download use cases: the engine reports a download's start, progress and end, and the user
//! cancels, retries and clears them.
//!
//! The library knows which listed download each of the engine's downloads is (the engine names
//! them by [`DownloadToken`]), and the tab each came from, for as long as Quickie runs; through
//! the engine's [`Downloads`](crate::Downloads) port it stops and fetches again what the user
//! asks.
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

use std::path::Path;

use quickie_domain_browsing::{Jar, TabId};
use quickie_domain_library::{
    Download, DownloadError, DownloadId, DownloadState, Downloads, DownloadsSnapshot,
};
use url::Url;

use super::Library;
use crate::{AppError, Clock, DownloadEvent, DownloadToken, LibraryRepo};

/// Why a download listed as in progress at start failed.
const INTERRUPTED: &str = "Quickie quit before it finished";

/// What a download event changed, for the shell to tell the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadMoved {
    /// The engine started a download, now listed under `id`.
    Started { id: DownloadId, file_name: String },
    /// The engine refused a download before it started, for `reason`.
    Refused { reason: String },
    /// A file made from a page, such as a PDF of it, is written whole and listed under `id`.
    Saved { id: DownloadId, file_name: String },
    /// More of a download arrived.
    Progressed(DownloadId),
    /// Every byte of a download is in its file.
    Finished(DownloadId),
    /// A download stopped before it finished.
    Failed(DownloadId),
    /// Nothing listed changed: the download is not one the library lists any more, or it
    /// ended first (the user cancelled it, and the engine's last word came after).
    Unlisted,
}

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

    /// A download moved on, as the engine reported it: a started one is listed, a file made
    /// from a page is listed whole, and the others move the listed download they are. A start
    /// that cannot be listed is cancelled through `engine`, so no file is fetched that the
    /// list does not show.
    ///
    /// # Errors
    ///
    /// The store failed, or the listed download cannot move that way.
    pub fn download_moved(
        &mut self,
        event: DownloadEvent,
        engine: &mut impl crate::Downloads,
    ) -> Result<DownloadMoved, AppError> {
        let token = event.token();
        match event {
            DownloadEvent::Started {
                tab,
                jar,
                url,
                file,
                ..
            } => {
                let file_name = file_name(&file);
                match self.download_started(url, file_name.clone(), jar) {
                    Ok(id) => {
                        self.listed.insert(token, id);
                        self.fetched.insert(id, (token, tab));
                        Ok(DownloadMoved::Started { id, file_name })
                    }
                    Err(error) => {
                        engine.cancel(token);
                        Err(error)
                    }
                }
            }
            DownloadEvent::Refused { reason, .. } => Ok(DownloadMoved::Refused { reason }),
            DownloadEvent::Saved { jar, url, file, .. } => {
                let file_name = file_name(&file);
                let id = self.download_started(url, file_name.clone(), jar)?;
                self.download_finished(id)?;
                Ok(DownloadMoved::Saved { id, file_name })
            }
            DownloadEvent::Progress {
                received, total, ..
            } => self.move_listed(token, |library, id| {
                library
                    .download_progressed(id, received, total)
                    .map(|()| DownloadMoved::Progressed(id))
            }),
            DownloadEvent::Finished { .. } => self.move_listed(token, |library, id| {
                library
                    .download_finished(id)
                    .map(|()| DownloadMoved::Finished(id))
            }),
            DownloadEvent::Failed { reason, .. } => self.move_listed(token, |library, id| {
                library
                    .download_failed(id, reason)
                    .map(|()| DownloadMoved::Failed(id))
            }),
        }
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

    /// The user stopped a download: it is listed as cancelled, and the engine's download, when
    /// it fetches one this session, stops through `engine`.
    ///
    /// # Errors
    ///
    /// The download is not listed or no longer in progress, or the store failed; the engine's
    /// download goes on.
    pub fn cancel_download(
        &mut self,
        id: DownloadId,
        engine: &mut impl crate::Downloads,
    ) -> Result<(), AppError> {
        self.end_download(|next| next.cancel(id))?;
        if let Some(&(token, _)) = self.fetched.get(&id) {
            engine.cancel(token);
        }
        Ok(())
    }

    /// Fetches an ended download again through `engine`: from the tab whose page started it
    /// while that tab is `open`, else from the `current` tab. A throwaway jar's download is
    /// fetched only from its own tab, so its cookies never reach another jar. The download
    /// leaves the list; the new fetch is listed when the engine starts it. False when no tab
    /// may fetch it, and nothing changed.
    ///
    /// # Errors
    ///
    /// The download is not listed or still in progress, or the store failed, and nothing
    /// changed; or the engine could not fetch it, and it is off the list.
    pub fn retry_download(
        &mut self,
        id: DownloadId,
        engine: &mut impl crate::Downloads,
        open: impl Fn(TabId) -> bool,
        current: Option<TabId>,
    ) -> Result<bool, AppError> {
        let origin = self
            .fetched
            .get(&id)
            .map(|&(_, tab)| tab)
            .filter(|&tab| open(tab));
        let tab = match origin {
            Some(tab) => tab,
            None if self.is_session_only(id) => return Ok(false),
            None => match current {
                Some(tab) => tab,
                None => return Ok(false),
            },
        };
        let download = self.forget_download(id)?;
        engine.download(tab, &download.url)?;
        Ok(true)
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
            self.unlist(download.id);
        }
        Ok(cleared)
    }

    /// Takes an ended download off the list; returns it.
    fn forget_download(&mut self, id: DownloadId) -> Result<Download, AppError> {
        let mut next = self.downloads.clone();
        let removed = next.remove(id)?;
        self.commit_downloads(next)?;
        self.unlist(id);
        Ok(removed)
    }

    /// Forgets all the library knew of a download that left the list.
    fn unlist(&mut self, id: DownloadId) {
        self.session_only.remove(&id);
        if let Some((token, _)) = self.fetched.remove(&id) {
            self.listed.remove(&token);
        }
    }

    /// Moves the listed download the engine's `token` is, as `step` says. A download not
    /// listed, or one that ended first, moves nothing.
    fn move_listed(
        &mut self,
        token: DownloadToken,
        step: impl FnOnce(&mut Self, DownloadId) -> Result<DownloadMoved, AppError>,
    ) -> Result<DownloadMoved, AppError> {
        let Some(&id) = self.listed.get(&token) else {
            return Ok(DownloadMoved::Unlisted);
        };
        match step(self, id) {
            Err(AppError::Download(DownloadError::Ended(_))) => Ok(DownloadMoved::Unlisted),
            moved => moved,
        }
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

/// The name a download's file has in its folder.
fn file_name(file: &Path) -> String {
    file.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

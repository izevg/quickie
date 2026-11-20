//! Downloads: the engine's reports become library transitions (`Library::download_*`), and the
//! popover's requests become library commands and engine calls. A toast says when one starts,
//! ends, or fails. A file Quickie writes itself, a page saved as a PDF, is listed started and
//! finished at once.

use std::path::PathBuf;

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSString, NSURL};
use quickie_application::AppError;
use quickie_domain_browsing::{Jar, TabId};
use quickie_domain_library::{DownloadError, DownloadId};
use quickie_engine_webkit::{DownloadKey, DownloadReport};
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::DownloadsRequest;

use super::super::Browser;
use crate::input::keystroke::OverlayKey;

impl Browser {
    /// `⌘J`: opens the popover, or closes it when it shows.
    pub(in crate::browser) fn toggle_downloads(&self, trigger: Trigger) {
        if self.popovers.downloads.is_open() {
            self.close_popovers();
            return;
        }
        self.close_overlays();
        self.hide_find();
        let Some(host) = self.current().and_then(|(_, window)| window.contentView()) else {
            return;
        };
        let library = self.library.borrow();
        let downloads: Vec<_> = library.downloads().all().cloned().collect();
        self.popovers.downloads.show(&host, &downloads, trigger);
    }

    /// The engine's download reports since the last time, into the library.
    pub(in crate::browser) fn download_reports(&self) {
        let reports = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            browsing.engine_mut().take_download_reports()
        };
        if reports.is_empty() {
            return;
        }
        for report in reports {
            self.download_report(report);
        }
        self.refresh_downloads();
    }

    fn download_report(&self, report: DownloadReport) {
        let key = report.key();
        let id = self.popovers.links.borrow().ids.get(&key).copied();
        let mut library = self.library.borrow_mut();
        let outcome = match (report, id) {
            (
                DownloadReport::Started {
                    tab,
                    jar,
                    url,
                    file,
                    ..
                },
                _,
            ) => {
                let name = file_name(&file);
                drop(library);
                return self.download_started(key, tab, jar, url, name);
            }
            (DownloadReport::Refused { reason, .. }, _) => {
                drop(library);
                return self.toast(&format!("Couldn't download: {reason}"));
            }
            (DownloadReport::Saved { jar, url, file, .. }, _) => {
                let name = file_name(&file);
                let toast = format!("Saved {name} to Downloads");
                let listed = library
                    .download_started(url, name, jar)
                    .and_then(|id| library.download_finished(id));
                drop(library);
                return match listed {
                    Ok(()) => self.toast(&toast),
                    Err(error) => self.report(&error),
                };
            }
            (_, None) => return,
            (
                DownloadReport::Progress {
                    received, total, ..
                },
                Some(id),
            ) => library.download_progressed(id, received, total),
            (DownloadReport::Finished { .. }, Some(id)) => library
                .download_finished(id)
                .map(|()| self.ended(&library, id, "Downloaded")),
            (DownloadReport::Failed { reason, .. }, Some(id)) => library
                .download_failed(id, reason)
                .map(|()| self.ended(&library, id, "Download failed:")),
        };
        drop(library);
        match outcome {
            // The user cancelled it first; the engine's last word comes after.
            Ok(()) | Err(AppError::Download(DownloadError::Ended(_))) => {}
            Err(error) => self.report(&error),
        }
    }

    fn download_started(
        &self,
        key: DownloadKey,
        tab: TabId,
        jar: Jar,
        url: url::Url,
        name: String,
    ) {
        let toast = format!("Downloading {name}");
        let started = self.library.borrow_mut().download_started(url, name, jar);
        match started {
            Ok(id) => {
                let mut links = self.popovers.links.borrow_mut();
                links.ids.insert(key, id);
                links.keys.insert(id, (key, tab));
                drop(links);
                self.toast(&toast);
            }
            Err(error) => {
                self.browsing.borrow_mut().engine_mut().cancel_download(key);
                self.report(&error);
            }
        }
    }

    /// Tells that a download ended, by its name.
    fn ended(&self, library: &crate::Lib, id: DownloadId, what: &str) {
        if let Some(download) = library.downloads().get(id) {
            self.toast(&format!("{what} {}", download.file_name));
        }
    }

    /// What a click or a key in the popover asked.
    pub(in crate::browser) fn download_request(&self, request: DownloadsRequest) {
        match request {
            DownloadsRequest::Open(id) => self.open_download(id, false),
            DownloadsRequest::Reveal(id) => self.open_download(id, true),
            DownloadsRequest::Cancel(id) => self.cancel_download(id),
            DownloadsRequest::Retry(id) => self.retry_download(id),
            DownloadsRequest::ClearFinished => {
                let cleared = self.library.borrow_mut().clear_finished_downloads();
                match cleared {
                    Ok(cleared) => {
                        let mut links = self.popovers.links.borrow_mut();
                        for download in cleared {
                            if let Some((key, _)) = links.keys.remove(&download.id) {
                                links.ids.remove(&key);
                            }
                        }
                    }
                    Err(error) => self.report(&error),
                }
            }
        }
        self.refresh_downloads();
    }

    /// Opens the file with its app, or shows it in the Finder when `reveal`.
    fn open_download(&self, id: DownloadId, reveal: bool) {
        let Some(path) = self.download_path(id) else {
            return;
        };
        if !reveal {
            self.close_popovers();
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let workspace = NSWorkspace::sharedWorkspace();
        if reveal {
            workspace.activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        } else if !workspace.openURL(&url) {
            self.toast(&format!("Couldn't open {}", path.display()));
        }
    }

    fn cancel_download(&self, id: DownloadId) {
        let cancelled = self.library.borrow_mut().cancel_download(id);
        if let Err(error) = cancelled {
            return self.report(&error);
        }
        let key = self
            .popovers
            .links
            .borrow()
            .keys
            .get(&id)
            .map(|&(key, _)| key);
        if let Some(key) = key {
            self.browsing.borrow_mut().engine_mut().cancel_download(key);
        }
    }

    /// Fetches an ended download again from the page that started it, or, when that tab is
    /// gone, from the current page; a throwaway jar's download only from its own tab.
    fn retry_download(&self, id: DownloadId) {
        let origin = self
            .popovers
            .links
            .borrow()
            .keys
            .get(&id)
            .map(|&(_, tab)| tab);
        let session_only = self.library.borrow().is_session_only(id);
        let open = |tab: TabId| self.browsing.borrow().session().tab(tab).is_some();
        let tab = match origin.filter(|&tab| open(tab)) {
            Some(tab) => Some(tab),
            None if session_only => None,
            None => self.current().and_then(|(window, _)| self.active(window)),
        };
        let Some(tab) = tab else {
            return self.toast("Its throwaway tab is closed, so it can't be fetched again");
        };
        let retried = self.library.borrow_mut().retry_download(id);
        let download = match retried {
            Ok(download) => download,
            Err(error) => return self.report(&error),
        };
        {
            let mut links = self.popovers.links.borrow_mut();
            if let Some((key, _)) = links.keys.remove(&id) {
                links.ids.remove(&key);
            }
        }
        self.run(|browsing| Ok(browsing.engine_mut().download(tab, &download.url)?));
    }

    /// The file of a listed download: in the Downloads folder under its name.
    fn download_path(&self, id: DownloadId) -> Option<PathBuf> {
        let library = self.library.borrow();
        let download = library.downloads().get(id)?;
        Some(self.popovers.folder.join(&download.file_name))
    }

    /// The popover shows the list as it is now.
    fn refresh_downloads(&self) {
        if !self.popovers.downloads.is_open() {
            return;
        }
        let downloads: Vec<_> = self.library.borrow().downloads().all().cloned().collect();
        self.popovers.downloads.update(&downloads);
    }

    pub(super) fn downloads_key(&self, key: OverlayKey) -> bool {
        match key {
            OverlayKey::Move(delta) => self.popovers.downloads.move_selection(delta),
            OverlayKey::Accept { new_tab } => {
                if let Some(request) = self.popovers.downloads.enter(new_tab) {
                    self.download_request(request);
                }
            }
            OverlayKey::Previous => {}
            OverlayKey::Dismiss => self.close_popovers(),
        }
        true
    }
}

fn file_name(file: &std::path::Path) -> String {
    file.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

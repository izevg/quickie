//! Downloads: the engine's reports go to the `Library` use case, which lists them and knows
//! which listed download each of the engine's is; the popover's requests become library
//! commands, which drive the engine's downloads. A toast says when one starts, ends, or fails.
//! A file Quickie writes itself, a page saved as a PDF, is listed started and finished at once.

use std::path::PathBuf;

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSString, NSURL};
use quickie_application::{DownloadEvent, DownloadMoved};
use quickie_domain_library::DownloadId;
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{DownloadsRequest, Toast, ToastKind};

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

    /// A download the engine reported on, into the library, and a toast for what the user
    /// should hear of it. The popover catches up once per drain ([`Self::refresh_downloads`]).
    pub(in crate::browser) fn download_report(&self, report: DownloadEvent) {
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let moved = self
            .library
            .borrow_mut()
            .download_moved(report, browsing.engine_mut());
        drop(browsing);
        match moved {
            Ok(DownloadMoved::Started { file_name, .. }) => {
                self.notify(Toast::info("Downloading").with_detail(&file_name));
            }
            Ok(DownloadMoved::Refused { reason }) => {
                self.notify(Toast::problem(format!("Couldn't download: {reason}")));
            }
            Ok(DownloadMoved::Saved { file_name, .. }) => {
                self.notify(
                    Toast::new(ToastKind::Success, "Saved to Downloads").with_detail(&file_name),
                );
            }
            Ok(DownloadMoved::Finished(id)) => self.ended(id, ToastKind::Success, "Downloaded"),
            Ok(DownloadMoved::Failed(id)) => self.ended(id, ToastKind::Problem, "Download failed"),
            Ok(DownloadMoved::Progressed(_) | DownloadMoved::Unlisted) => {}
            Err(error) => self.report(&error),
        }
    }

    /// Tells that a download ended, by its name.
    fn ended(&self, id: DownloadId, kind: ToastKind, what: &str) {
        let library = self.library.borrow();
        if let Some(download) = library.downloads().get(id) {
            self.notify(Toast::new(kind, what).with_detail(&download.file_name));
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
                if let Err(error) = cleared {
                    self.report(&error);
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
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let cancelled = self
            .library
            .borrow_mut()
            .cancel_download(id, browsing.engine_mut());
        drop(browsing);
        if let Err(error) = cancelled {
            self.report(&error);
        }
    }

    /// Fetches an ended download again: the library picks the page that started it, or, when
    /// that tab is gone, the current page; a throwaway jar's download only its own tab.
    fn retry_download(&self, id: DownloadId) {
        let current = self.current().and_then(|(window, _)| self.active(window));
        self.palette_sources_changed();
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let (session, engine) = browsing.session_and_engine();
        let retried = self.library.borrow_mut().retry_download(
            id,
            engine,
            |tab| session.tab(tab).is_some(),
            current,
        );
        drop(browsing);
        match retried {
            Ok(true) => {}
            Ok(false) => self.toast("Its throwaway tab is closed, so it can't be fetched again"),
            Err(error) => self.report(&error),
        }
    }

    /// The file of a listed download: in the Downloads folder under its name.
    fn download_path(&self, id: DownloadId) -> Option<PathBuf> {
        let library = self.library.borrow();
        let download = library.downloads().get(id)?;
        Some(self.popovers.folder.join(&download.file_name))
    }

    /// The popover shows the list as it is now.
    pub(in crate::browser) fn refresh_downloads(&self) {
        if !self.popovers.downloads.is_open() {
            return;
        }
        let downloads: Vec<_> = self.library.borrow().downloads().all().cloned().collect();
        self.popovers.downloads.update(&downloads);
    }

    pub(super) fn downloads_key(&self, key: OverlayKey) -> bool {
        match key {
            OverlayKey::Move(delta) => self.popovers.downloads.move_selection(delta),
            OverlayKey::Accept { new_tab, .. } => {
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

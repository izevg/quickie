//! The popovers at the top right: downloads (`⌘J`) and the site's shields (`⌥⌘S`). Each is an
//! E2, so opening one closes the other overlays, and opening another overlay closes it. Their
//! fixed keys come before the keymap's; any other key goes on to the keymap and the page.

mod downloads;
mod shields;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use objc2::MainThreadMarker;
use quickie_application::DownloadToken;
use quickie_domain_browsing::TabId;
use quickie_domain_library::DownloadId;
use quickie_ui_widgets::{DownloadsPopover, ShieldsPopover};

use super::Browser;
use crate::input::keystroke::OverlayKey;

/// The popovers and what the browser keeps for them.
pub struct Popovers {
    downloads: DownloadsPopover,
    shields: ShieldsPopover,
    /// Where downloads are saved.
    folder: PathBuf,
    /// The engine's downloads and the library's, both ways, with the tab each came from.
    links: RefCell<Links>,
    /// The tab whose shields changed since its page loaded: `⏎` reloads it.
    reload_offered: Cell<Option<TabId>>,
}

#[derive(Default)]
struct Links {
    ids: HashMap<DownloadToken, DownloadId>,
    keys: HashMap<DownloadId, (DownloadToken, TabId)>,
}

impl Popovers {
    pub fn new(folder: PathBuf, mtm: MainThreadMarker) -> Self {
        let downloads = DownloadsPopover::new(mtm);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let shown = home
            .and_then(|home| folder.strip_prefix(home).ok().map(Path::to_path_buf))
            .map_or_else(
                || folder.display().to_string(),
                |rest| format!("~/{}", rest.display()),
            );
        downloads.set_folder(&shown);
        Self {
            downloads,
            shields: ShieldsPopover::new(mtm),
            folder,
            links: RefCell::new(Links::default()),
            reload_offered: Cell::new(None),
        }
    }

    /// Whether a popover shows.
    pub fn is_open(&self) -> bool {
        self.downloads.is_open() || self.shields.is_open()
    }
}

impl Browser {
    /// Hands the popovers' clicks to the browser.
    pub(super) fn wire_popovers(&self) {
        let this = self.this.clone();
        self.popovers.downloads.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.download_request(request);
                browser.drain();
            }
        });
        let this = self.this.clone();
        self.popovers.shields.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.shields_request(request);
                browser.drain();
            }
        });
    }

    /// Takes the popovers away, the agent's panel and settings too: another overlay is about
    /// to show, or a command runs.
    pub(super) fn hide_popovers(&self) {
        self.popovers.downloads.hide();
        self.popovers.shields.hide();
        self.hide_extension_popup();
        self.hide_agent();
    }

    /// A popover's fixed key; `false` when no popover shows or the key is not one of its own.
    /// An extension's popup takes only `⎋`: every other key is its page's.
    pub(super) fn popover_key(&self, key: OverlayKey) -> bool {
        if self.extension_ui.popup_open() {
            if key == OverlayKey::Dismiss {
                self.close_popovers();
                return true;
            }
            return false;
        }
        if self.popovers.downloads.is_open() {
            return self.downloads_key(key);
        }
        if self.popovers.shields.is_open() {
            return self.shields_key(key);
        }
        false
    }

    /// `⎋` or a finished action: the popover goes and the page takes the keyboard.
    fn close_popovers(&self) {
        self.hide_popovers();
        if let Some((window, _)) = self.current() {
            self.settle(window);
        }
    }
}

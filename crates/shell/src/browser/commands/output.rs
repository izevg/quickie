//! A page out of the browser: printed (`⌥⌘P`, since `⌘P` goes to a tab), saved as a PDF into
//! the downloads list, or open in Web Inspector.
//!
//! **Web Inspector.** `WKWebView` has no public call that opens Web Inspector, only
//! `isInspectable`, which every page sets; Quickie calls no private API, so the developer
//! tools' commands say where Web Inspector is: right-click → Inspect Element, or Safari's
//! Develop menu. On Chromium (P1) they open `DevTools` docked in the window and on the asked
//! panel, through the bridge.

use quickie_application::Downloads;
use quickie_domain_browsing::WindowId;

use super::Browser;

impl Browser {
    /// Opens the print panel as a sheet on the window: print, or save a paginated PDF from it.
    pub(super) fn print(&self, window: WindowId) {
        let Some(tab) = self.active(window) else {
            return;
        };
        let printed = self.browsing.borrow_mut().engine_mut().print(tab);
        if let Err(error) = printed {
            self.toast(&format!("Couldn't print: {error}"));
        }
    }

    /// Saves the active page as a PDF in the downloads folder; it is listed with the downloads
    /// once written.
    pub(super) fn save_pdf(&self, window: WindowId) {
        let Some(tab) = self.active(window) else {
            return;
        };
        let saving = self.browsing.borrow_mut().engine_mut().save_pdf(tab);
        if let Err(error) = saving {
            self.toast(&format!("Couldn't save a PDF: {error}"));
        }
    }

    /// The developer tools' commands: where Web Inspector opens from.
    pub(super) fn devtools_hint(&self) {
        let inspectable = self.current().is_some_and(|(window, _)| {
            self.active(window)
                .is_some_and(|tab| self.browsing.borrow_mut().engine_mut().inspectable(tab))
        });
        self.toast(if inspectable {
            "Right-click → Inspect Element opens Web Inspector"
        } else {
            "Web Inspector needs macOS 13.3 or later"
        });
    }
}

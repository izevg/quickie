//! A page out of the browser: printed (`⌥⌘P`, since `⌘P` goes to a tab), saved as a PDF into
//! the downloads list, or open in the developer tools.
//!
//! **Developer tools.** How they open is the engine's capability ([`Inspector`]). On `WebKit`
//! (`Inspector::FromContextMenu`) `WKWebView` has no public call that opens Web Inspector, only
//! `isInspectable`, which every page sets. That lets Safari's Develop menu attach Web Inspector;
//! the page's own context menu offers Inspect Element only with a private preference. Quickie
//! calls no private API, so the developer tools' commands point to Safari's Develop menu. On Chromium (P1, `Inspector::Programmatic`) they open `DevTools` docked in the
//! window and on the asked panel, through the bridge.

use quickie_application::{Downloads, Engine, EngineError, Inspector, PageSurface};
use quickie_domain_browsing::WindowId;

use super::Browser;
use crate::MainEngine;

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

    /// The developer tools' commands: they open on the active page, or a toast says where
    /// they open from.
    pub(super) fn devtools_hint(&self) {
        let Some(tab) = self.current().and_then(|(window, _)| self.active(window)) else {
            return;
        };
        let inspected = self.browsing.borrow_mut().engine_mut().inspect(tab);
        if let Some(told) = inspector_told::<MainEngine>(inspected) {
            self.toast(&told);
        }
    }
}

/// What the user is told after asking `E` for the developer tools, which answered `inspected`:
/// where they open from when only the page's context menu opens them, why they cannot when
/// they cannot, and nothing when they opened.
pub(super) fn inspector_told<E: Engine>(inspected: Result<(), EngineError>) -> Option<String> {
    match (E::CAPS.inspector, inspected) {
        (Inspector::FromContextMenu, Ok(())) => {
            Some("Web Inspector opens from Safari: Develop → Quickie → this page".to_owned())
        }
        (Inspector::FromContextMenu, Err(EngineError(why))) => Some(why),
        (Inspector::Programmatic, Ok(())) => None,
        (Inspector::Programmatic, Err(error)) => {
            Some(format!("Couldn't open the developer tools: {error}"))
        }
    }
}

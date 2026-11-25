//! A page out of the browser: printed (`⌥⌘P`, since `⌘P` goes to a tab), saved as a PDF into
//! the downloads list, or open in the developer tools.
//!
//! **Developer tools.** How they open is the engine's capability ([`Inspector`]). On our
//! `WebKit` fork and on Chromium it is `Inspector::Programmatic`: each `quickie.devtools.*`
//! command opens them on the active page, docked in the window, toggled, picking an element,
//! or on its panel ([`DevTools`]). An engine that cannot (the system `WebKit` of a plain
//! `cargo run`) answers with an error that says where they open from, and the toast shows it.

use quickie_application::{DevTools, Downloads, Engine, EngineError, Inspector, PageSurface};
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

    /// Does what a developer tools' command asks on the window's active page; a toast says why
    /// it could not.
    pub(super) fn devtools(&self, window: WindowId, asked: DevTools) {
        let Some(tab) = self.active(window) else {
            return;
        };
        self.mcp_inspector_takeover(tab);
        let inspected = self.browsing.borrow_mut().engine_mut().inspect(tab, asked);
        if let Some(told) = inspector_told(<MainEngine as Engine>::CAPS.inspector, inspected) {
            self.toast(&told);
        }
    }
}

/// What the user is told after asking an engine whose developer tools open as `inspector`
/// says, which answered `inspected`: nothing when they opened, why when they could not, and
/// where they open from when only the page's context menu opens them.
pub(super) fn inspector_told(
    inspector: Inspector,
    inspected: Result<(), EngineError>,
) -> Option<String> {
    match (inspector, inspected) {
        (Inspector::Programmatic, Ok(())) => None,
        (Inspector::Programmatic, Err(EngineError(why))) => {
            Some(format!("Couldn't open the developer tools: {why}"))
        }
        (Inspector::FromContextMenu, Ok(())) => {
            Some("Web Inspector opens from the page's context menu: Inspect Element".to_owned())
        }
        (Inspector::FromContextMenu, Err(EngineError(why))) => Some(why),
    }
}

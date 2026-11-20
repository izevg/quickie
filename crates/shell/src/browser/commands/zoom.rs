//! Page zoom: `⌘=` / `⌘+` in, `⌘-` out, `⌘0` back to 100 %, in Safari's steps, remembered per
//! site by the `Zoom` use case. Every page shows its site's zoom from the moment it commits, and
//! a zoom changed on one page reaches every open page of the site in the same jar. Pinch stays
//! the tab overview's gesture.

use quickie_application::AppError;
use quickie_domain_browsing::{JarId, TabId, WindowId};
use quickie_domain_settings::ZoomStep;

use super::Browser;

impl Browser {
    /// Zooms the active page's site a step, and says the new zoom in a toast.
    pub(super) fn zoom(&self, window: WindowId, step: ZoomStep) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let stepped = self.zoom.borrow_mut().step(&tab.url, tab.jar, step);
        match stepped {
            Ok(level) => {
                self.show_zooms(None);
                if let Some(host) = self.current().and_then(|(_, window)| window.contentView()) {
                    self.toasts.show_tagged(&host, level.to_string(), "zoom");
                }
            }
            Err(AppError::NoSite(_)) => self.toast("Only websites zoom"),
            Err(error) => self.report(&error),
        }
    }

    /// Puts pages at their site's zoom: those of `only`, or every page.
    pub(in crate::browser) fn show_zooms(&self, only: Option<&[TabId]>) {
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let zoom = self.zoom.borrow();
        let session = browsing.session();
        let pages: Vec<(TabId, f64)> = session
            .windows()
            .filter_map(|window| session.tabs(window).ok())
            .flatten()
            .filter(|tab| only.is_none_or(|only| only.contains(&tab.id)))
            .map(|tab| (tab.id, zoom.level(&tab.url, tab.jar).factor()))
            .collect();
        let engine = browsing.engine_mut();
        for (tab, factor) in pages {
            engine.set_zoom(tab, factor);
        }
    }

    /// Throwaway jars that closed: what they zoomed goes with them.
    pub(in crate::browser) fn forget_zooms(&self, jars: impl IntoIterator<Item = JarId>) {
        let mut zoom = self.zoom.borrow_mut();
        for jar in jars {
            zoom.forget_jar(jar);
        }
    }
}

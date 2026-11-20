//! A page that goes full screen (the Fullscreen API) always gets the full-screen notice: whose
//! page it is and how to leave, over the page in the window that now fills the screen.

use objc2::rc::Retained;
use objc2_app_kit::NSView;
use quickie_domain_browsing::TabId;
use url::Url;

use super::super::Browser;

impl Browser {
    /// Shows the notice for a page that entered full screen, and takes it away when one left.
    pub(in crate::browser) fn fullscreen_changed(&self, tab: TabId, entered: bool) {
        if !entered {
            self.page.fullscreen.hide();
            return;
        }
        let shown: Option<(Retained<NSView>, Url)> = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            let url = browsing.session().tab(tab).map(|tab| tab.url.clone());
            let host = browsing
                .engine_mut()
                .view(tab)
                .and_then(NSView::window)
                .and_then(|window| window.contentView());
            host.zip(url)
        };
        if let Some((host, url)) = shown {
            self.page.fullscreen.show(&host, &url);
        }
    }
}

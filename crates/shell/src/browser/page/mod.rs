//! What floats over the page: find in page (`⌘F`, `⌘G`, `⇧⌘G`), the origin chip after a
//! cross-origin navigation or a tab switch, the load glow, the sheets of what pages
//! ask ([`asks`]), and the full-screen notice ([`fullscreen`]). The engine's reports reach them
//! from [`Browser::drain`], tab switches from [`Browser::settle`].

mod asks;
mod find;
mod fullscreen;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use objc2::MainThreadMarker;
use quickie_application::PageSurface;
use quickie_domain_browsing::{Origin, TabId, WindowId, is_cross_origin};
use quickie_ui_widgets::{FindBar, FullscreenNotice, OriginChip};
use url::Url;

pub use asks::SheetRoute;

use super::Browser;

/// The page widgets and what they last showed.
pub struct PageWidgets {
    asks: asks::PageAsks,
    fullscreen: Rc<FullscreenNotice>,
    fullscreen_notice_tab: Cell<Option<TabId>>,
    find_bar: FindBar,
    /// The tab the find bar searches.
    find_tab: Cell<Option<TabId>>,
    chip: Rc<OriginChip>,
    /// Per window, the active tab and its committed URL when last looked at.
    seen: RefCell<BTreeMap<WindowId, (TabId, Url)>>,
    /// Per window, the page that fills it, full screen.
    in_window: RefCell<BTreeMap<WindowId, TabId>>,
}

impl PageWidgets {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            asks: asks::PageAsks::new(mtm),
            fullscreen: FullscreenNotice::new(mtm),
            fullscreen_notice_tab: Cell::new(None),
            find_bar: FindBar::new(mtm),
            find_tab: Cell::new(None),
            chip: OriginChip::new(mtm),
            seen: RefCell::new(BTreeMap::new()),
            in_window: RefCell::new(BTreeMap::new()),
        }
    }
}

/// What changed in a window since it was last looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Nothing,
    /// The active tab committed a page of another origin.
    Origin,
    /// Another tab is active, or the window is new.
    Tab,
}

/// The change from `before` to `tab` showing `url`.
fn change(before: Option<&(TabId, Url)>, tab: TabId, url: &Url) -> Change {
    match before {
        Some((was, _)) if *was != tab => Change::Tab,
        None => Change::Tab,
        Some((_, at)) if is_cross_origin(at, url) => Change::Origin,
        Some(_) => Change::Nothing,
    }
}

impl Browser {
    /// A window closed: what it last showed goes.
    pub(super) fn forget_window_page(&self, window: WindowId) {
        self.page.seen.borrow_mut().remove(&window);
        self.forget_page_fullscreen(window);
    }

    /// A tab's page loaded more, or stopped: the load glow of every window it is active in.
    pub(super) fn loading(&self, tab: TabId, progress: Option<f64>) {
        let windows: Vec<WindowId> = self.windows.borrow().keys().copied().collect();
        for window in windows {
            if self.active(window) == Some(tab) {
                self.set_progress(window, progress);
            }
        }
    }

    /// For every window whose active tab or origin changed: the origin chip, and on a tab
    /// switch the new tab's loading and the mini player; the find bar closes when its tab
    /// leaves.
    pub(super) fn page_changes(&self) {
        let active: Vec<(WindowId, TabId, Url)> = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            let session = browsing.session();
            self.windows
                .borrow()
                .keys()
                .filter_map(|&window| {
                    let tab = session.active(window)?;
                    Some((window, tab, session.tab(tab)?.url.clone()))
                })
                .collect()
        };
        // A page in full screen whose tab is no longer in front (another tab, or it closed)
        // leaves fullscreen before floating video or showing the new tab's overlays.
        let behind: Vec<WindowId> = self
            .page
            .in_window
            .borrow()
            .iter()
            .filter(|(window, fullscreen_tab)| {
                !active
                    .iter()
                    .any(|(shown, tab, _)| shown == *window && tab == *fullscreen_tab)
            })
            .map(|(&window, _)| window)
            .collect();
        for window in behind {
            self.forget_page_fullscreen(window);
        }
        // Only then the mini player: a tab left in full screen floats its video once the
        // engine was told to take it out, so the float script runs after the page left.
        self.player_tab_closed();
        for (window, tab, url) in active {
            let before = self.page.seen.borrow().get(&window).cloned();
            let change = change(before.as_ref(), tab, &url);
            self.page
                .seen
                .borrow_mut()
                .insert(window, (tab, url.clone()));
            if let (Change::Tab, Some((left, _))) = (change, &before) {
                self.left_tab(window, *left, tab);
            }
            if change == Change::Tab {
                if let Some(frame) = self.frames.borrow().get(&window) {
                    frame.reset_loading();
                }
                self.show_progress(window);
                if self.page.find_tab.get().is_some_and(|find| find != tab) {
                    self.hide_find();
                }
                self.crash_notice(window, tab);
            }
            if change != Change::Nothing {
                self.show_origin(window, &url);
            }
        }
    }

    fn show_origin(&self, window: WindowId, url: &Url) {
        if self.page_fullscreen(window) {
            return;
        }
        let host = self.window(window).and_then(|window| window.contentView());
        match (Origin::of(url), host) {
            (Some(origin), Some(host)) => self.page.chip.show(&host, &origin, url.path(), None),
            _ => self.page.chip.hide(),
        }
    }

    /// The window's active tab's loading, in its load glow.
    fn show_progress(&self, window: WindowId) {
        let progress = self.active(window).and_then(|tab| {
            let browsing = self.browsing.try_borrow().ok()?;
            browsing.engine().loading(tab)
        });
        self.set_progress(window, progress);
    }

    /// The window's load glow at `progress`; fading out for `None`, and while a page fills the
    /// window full screen.
    fn set_progress(&self, window: WindowId, progress: Option<f64>) {
        let progress = progress.filter(|_| !self.page_fullscreen(window));
        if let Some(frame) = self.frames.borrow().get(&window) {
            frame.set_loading(progress);
        }
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::{Profile, TabId};
    use url::Url;

    use super::{Change, change};

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn a_new_window_or_another_tab_is_a_tab_switch() {
        let a = url("https://a.example/");

        assert_eq!(change(None, TabId::new(Profile::Main, 1), &a), Change::Tab);
        assert_eq!(
            change(
                Some(&(TabId::new(Profile::Main, 1), a.clone())),
                TabId::new(Profile::Main, 2),
                &a
            ),
            Change::Tab
        );
    }

    #[test]
    fn the_same_tab_on_another_origin_changes_the_origin() {
        let before = (TabId::new(Profile::Main, 1), url("https://a.example/one"));

        assert_eq!(
            change(
                Some(&before),
                TabId::new(Profile::Main, 1),
                &url("https://b.example/")
            ),
            Change::Origin
        );
        assert_eq!(
            change(
                Some(&before),
                TabId::new(Profile::Main, 1),
                &url("https://a.example/two")
            ),
            Change::Nothing
        );
    }
}

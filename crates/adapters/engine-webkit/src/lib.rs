//! Development engine of Quickie: pages on `WKWebView`, so UI work never waits for a Chromium
//! build (`docs/architecture.md` §3, P0). It implements the application's [`EnginePort`].
//!
//! The engine keeps one page per tab, keyed by the domain's [`TabId`]. Main-jar pages share the
//! default data store; each throwaway jar gets one non-persistent store, shared by its tabs and
//! dropped with the jar. The shell registers each window's host view; [`EnginePort::show`]
//! puts a tab's page into it, under any overlays.
//!
//! Page events flow back without re-entering the use cases: pages record what they committed,
//! the engine schedules the shell's waker on the main run loop, and the shell drains
//! [`WebKitEngine::take_navigations`] into `Browsing::navigated`.
//!
//! Every page shares one user content controller, so the [`WebKitContentBlocker`] the engine
//! hands out ([`WebKitEngine::content_blocker`]) blocks on all of them. [`UrlSessionHttp`] is the
//! network for the browser's own requests, such as fetching filter lists.
//!
//! Navigations that become downloads go to one download center, which saves them into the
//! folder [`WebKitEngine::set_download_directory`] names and reports through
//! [`WebKitEngine::take_download_reports`].

#![expect(
    unsafe_code,
    reason = "WebKit through objc2: its bindings mark most methods unsafe; KVO, the navigation \
              and download delegates and the scheme handler need classes; content rule lists \
              cross threads"
)]

mod downloads;
mod network;
mod page;
mod reports;
mod scheme;
mod shields;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSImage, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSRunLoop, NSString, NSURL};
use objc2_web_kit::{WKContentRuleListStore, WKUserContentController, WKWebsiteDataStore};
use quickie_application::{EngineError, EnginePort, Navigation};
use quickie_domain_browsing::{FindMatches, Jar, JarId, TabId, WindowId};
use url::Url;

use downloads::DownloadCenter;
pub use downloads::{DownloadKey, DownloadReport};
pub use network::UrlSessionHttp;
pub use page::FindStep;
pub use page::scripts::hints::{Followed, LinkTarget};
use page::{PageEvent, WebPage};
pub use reports::Navigated;
use reports::Reports;
use scheme::InternalPages;
pub use shields::{BlockerStatus, WebKitContentBlocker};

/// How wide, in points, the tab overview's thumbnails are drawn.
pub const THUMBNAIL_WIDTH: f64 = 480.0;

/// Thumbnails by tab, filled by snapshot callbacks.
type Thumbnails = Rc<RefCell<HashMap<TabId, Retained<NSImage>>>>;

/// The `WebKit` engine. Lives on the main thread, like every view it owns.
pub struct WebKitEngine {
    mtm: MainThreadMarker,
    pages: HashMap<TabId, WebPage>,
    hosts: HashMap<WindowId, Retained<NSView>>,
    shown: HashMap<WindowId, TabId>,
    jars: HashMap<JarId, Retained<WKWebsiteDataStore>>,
    internal: Retained<InternalPages>,
    /// Shared by every page: the content blocker installs its rule lists here.
    content: Retained<WKUserContentController>,
    downloads: Rc<DownloadCenter>,
    /// The jar of each page, for the downloads it starts.
    tab_jars: HashMap<TabId, Jar>,
    inbox: Rc<Inbox>,
    /// The last picture of each page, taken when it was hidden or asked for.
    thumbnails: Thumbnails,
}

impl std::fmt::Debug for WebKitEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebKitEngine")
            .field("pages", &self.pages.len())
            .field("shown", &self.shown)
            .field("jars", &self.jars.len())
            .finish_non_exhaustive()
    }
}

impl WebKitEngine {
    /// An engine with no pages. `internal` are the built-in pages, `(url, html)`, each under a
    /// custom scheme such as `quickie:`; they load from memory.
    #[must_use]
    pub fn new(internal: &[(Url, String)], mtm: MainThreadMarker) -> Self {
        let inbox: Rc<Inbox> = Rc::default();
        let waiting = Rc::downgrade(&inbox);
        let downloads = DownloadCenter::new(
            move || {
                if let Some(inbox) = waiting.upgrade() {
                    inbox.downloaded.set(true);
                    Inbox::schedule(&inbox);
                }
            },
            mtm,
        );
        Self {
            mtm,
            pages: HashMap::new(),
            hosts: HashMap::new(),
            shown: HashMap::new(),
            jars: HashMap::new(),
            internal: InternalPages::new(internal, mtm),
            // SAFETY: a fresh controller has no preconditions.
            content: unsafe { WKUserContentController::new(mtm) },
            downloads,
            tab_jars: HashMap::new(),
            inbox,
            thumbnails: Rc::default(),
        }
    }

    /// True when the tab's page says its focused element takes text: a key typed now is the
    /// page's, not a browser shortcut.
    #[must_use]
    pub fn input_focused(&self, tab: TabId) -> bool {
        self.pages.get(&tab).is_some_and(WebPage::input_focused)
    }

    /// The tab's last picture, `None` before its page was first drawn or for a tab not loaded.
    #[must_use]
    pub fn thumbnail(&self, tab: TabId) -> Option<Retained<NSImage>> {
        self.thumbnails.borrow().get(&tab).cloned()
    }

    /// Takes a new picture of the tab's page; `done` runs on the main thread once it is in
    /// [`Self::thumbnail`]. The page is drawn off the main thread.
    pub fn capture(&self, tab: TabId, done: impl FnOnce() + 'static) {
        let Some(page) = self.pages.get(&tab) else {
            return;
        };
        let thumbnails = Rc::downgrade(&self.thumbnails);
        page.snapshot(THUMBNAIL_WIDTH, move |image| {
            let (Some(image), Some(thumbnails)) = (image, thumbnails.upgrade()) else {
                return;
            };
            thumbnails.borrow_mut().insert(tab, image);
            done();
        });
    }

    /// Finds the links and controls on screen in the tab's page, for link hints. `done` gets
    /// them in document order, placed in the page view, which fills its window's host.
    pub fn link_targets(&self, tab: TabId, done: impl FnOnce(Vec<LinkTarget>) + 'static) {
        if let Some(page) = self.pages.get(&tab) {
            page.link_targets(done);
        }
    }

    /// Follows target `index` of the tab's last [`Self::link_targets`]: the page clicks or
    /// focuses it, or `done` hears of a link to open in another tab (in the background when
    /// `background`).
    pub fn follow_link(
        &self,
        tab: TabId,
        index: usize,
        background: bool,
        done: impl FnOnce(Followed) + 'static,
    ) {
        if let Some(page) = self.pages.get(&tab) {
            page.follow(index, background, done);
        }
    }

    /// Saves downloads into `directory` from now on; it is created when the first one comes.
    /// Until this is called, every download is refused.
    pub fn set_download_directory(&self, directory: PathBuf) {
        self.downloads.set_directory(directory);
    }

    /// What downloads did since the last call, oldest first.
    pub fn take_download_reports(&mut self) -> Vec<DownloadReport> {
        self.inbox.downloaded.set(false);
        self.downloads.take_reports()
    }

    /// Stops a running download and deletes the part of its file already written; nothing
    /// more is reported for it.
    pub fn cancel_download(&self, key: DownloadKey) {
        self.downloads.cancel(key);
    }

    /// Downloads `url` from a tab's page, in its jar, as a retry does; it is reported like any
    /// other download.
    ///
    /// # Errors
    ///
    /// The tab has no page.
    pub fn download(&self, tab: TabId, url: &Url) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        let jar = self.tab_jars.get(&tab).copied().unwrap_or(Jar::Main);
        DownloadCenter::start(&self.downloads, page.web_view(), url, tab, jar);
        Ok(())
    }

    /// The content blocker of every page, existing and future. It keeps compiled rule lists in
    /// `directory`, which it creates.
    ///
    /// # Errors
    ///
    /// The directory cannot be created or is no place `WebKit` can keep rule lists.
    pub fn content_blocker(&self, directory: &Path) -> Result<WebKitContentBlocker, EngineError> {
        std::fs::create_dir_all(directory)
            .map_err(|error| EngineError(format!("{}: {error}", directory.display())))?;
        let path = directory
            .to_str()
            .ok_or_else(|| EngineError(format!("{} is not UTF-8", directory.display())))?;
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
        // SAFETY: `url` is a file URL of an existing directory.
        let store = unsafe { WKContentRuleListStore::storeWithURL(Some(&url), self.mtm) }
            .ok_or_else(|| EngineError(format!("no rule list store in {path}")))?;
        Ok(WebKitContentBlocker::new(
            store,
            self.content.clone(),
            self.mtm,
        ))
    }

    /// Makes `host` the view a window's pages fill. The tab shown there, if any, moves in.
    pub fn attach_host(&mut self, window: WindowId, host: &NSView) {
        self.hosts.insert(window, host.retain());
        if let Some(page) = self.shown.get(&window).and_then(|tab| self.pages.get(tab)) {
            place(page.view(), host);
        }
    }

    /// Forgets a window's host view; its page leaves it.
    pub fn detach_host(&mut self, window: WindowId) {
        if let Some(page) = self.shown.get(&window).and_then(|tab| self.pages.get(tab)) {
            page.view().removeFromSuperview();
        }
        self.hosts.remove(&window);
    }

    /// The view of a tab's page, to give it keyboard focus.
    #[must_use]
    pub fn view(&self, tab: TabId) -> Option<&NSView> {
        self.pages.get(&tab).map(WebPage::view)
    }

    /// Sets what runs, later on the main run loop and never inside an engine call, when pages
    /// have reported navigations, loading progress, the matches of a find, or downloads. It
    /// should drain [`Self::take_navigations`], [`Self::take_progressed`],
    /// [`Self::take_found`], and [`Self::take_download_reports`].
    pub fn on_navigated(&mut self, wake: impl Fn() + 'static) {
        *self.inbox.wake.borrow_mut() = Some(Rc::new(wake));
        Inbox::schedule(&self.inbox);
    }

    /// Navigations committed and titles changed since the last call, one per tab.
    pub fn take_navigations(&mut self) -> Vec<Navigated> {
        self.inbox.reports.borrow_mut().take()
    }

    /// True when some page's loading progress changed since the last call; read it with
    /// [`Self::progress`].
    pub fn take_progressed(&mut self) -> bool {
        self.inbox.progressed.replace(false)
    }

    /// How much of a tab's page has loaded, `0.0 ..= 1.0`, while it loads; `None` when it is
    /// not loading or has no page.
    #[must_use]
    pub fn progress(&self, tab: TabId) -> Option<f64> {
        self.pages.get(&tab).and_then(WebPage::progress)
    }

    /// Selects a match of `query` in a tab's page as `step` says, marks every match, and
    /// reports them through [`Self::take_found`]. An empty query clears the marks.
    pub fn find(&self, tab: TabId, query: &str, step: FindStep) {
        let Some(page) = self.pages.get(&tab) else {
            return;
        };
        let inbox = Rc::downgrade(&self.inbox);
        let asked = query.to_owned();
        page.find(query, step, move |matches| {
            let Some(inbox) = inbox.upgrade() else {
                return;
            };
            inbox.found.borrow_mut().push(Found {
                tab,
                query: asked.clone(),
                matches,
            });
            Inbox::schedule(&inbox);
        });
    }

    /// The finds answered since the last call, oldest first.
    pub fn take_found(&mut self) -> Vec<Found> {
        std::mem::take(&mut *self.inbox.found.borrow_mut())
    }

    fn store(&mut self, jar: Jar) -> Retained<WKWebsiteDataStore> {
        let mtm = self.mtm;
        match jar {
            // SAFETY: the shared default store exists for the life of the process.
            Jar::Main => unsafe { WKWebsiteDataStore::defaultDataStore(mtm) },
            Jar::Throwaway(id) => self
                .jars
                .entry(id)
                // SAFETY: creating a non-persistent store has no preconditions.
                .or_insert_with(|| unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) })
                .clone(),
        }
    }

    fn page(&self, tab: TabId) -> Result<&WebPage, EngineError> {
        self.pages
            .get(&tab)
            .ok_or_else(|| EngineError(format!("no page for {tab}")))
    }
}

impl EnginePort for WebKitEngine {
    /// The development engine opens no pages by itself yet (`window.open` and ⌘-click become
    /// the `WKUIDelegate`'s job), so there is no page to adopt.
    type Page = std::convert::Infallible;

    fn adopt(&mut self, page: Self::Page, _tab: TabId) -> Result<(), EngineError> {
        match page {}
    }

    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        if self.pages.contains_key(&tab) {
            return Err(EngineError(format!("{tab} already has a page")));
        }
        let store = self.store(jar);
        let inbox = Rc::downgrade(&self.inbox);
        let downloads = Rc::downgrade(&self.downloads);
        let page = WebPage::new(
            &store,
            &self.content,
            &self.internal,
            move |event| match event {
                PageEvent::Download(download) => {
                    if let Some(downloads) = downloads.upgrade() {
                        downloads.adopt(tab, jar, &download);
                    }
                }
                event => Inbox::record(&inbox, tab, event),
            },
            self.mtm,
        );
        // Laid out at a window's size from the start, so a background page draws and has a
        // picture for the overview before it is ever shown.
        if let Some(host) = self.hosts.values().next() {
            page.view().setFrame(host.bounds());
        }
        let loaded = page.load(url);
        self.pages.insert(tab, page);
        self.tab_jars.insert(tab, jar);
        loaded
    }

    /// Records that `window` shows `tab` and swaps the page views in its host. A window whose
    /// host is not attached yet gets the page when [`WebKitEngine::attach_host`] runs.
    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError> {
        let view = self.page(tab)?.view().retain();
        let previous = self.shown.insert(window, tab);
        if let Some(previous) = previous.filter(|&previous| previous != tab)
            && let Some(page) = self.pages.get(&previous)
        {
            // The page as the user left it, for the tab overview; drawn while still laid out.
            self.capture(previous, || {});
            page.view().removeFromSuperview();
        }
        if let Some(host) = self.hosts.get(&window) {
            place(&view, host);
        }
        Ok(())
    }

    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        match navigation {
            Navigation::Load(url) => return page.load(url),
            Navigation::Back => page.go_back(),
            Navigation::Forward => page.go_forward(),
            Navigation::Reload => page.reload(),
            Navigation::Stop => page.stop(),
        }
        Ok(())
    }

    fn close(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.shown.retain(|_, shown| *shown != tab);
        self.inbox.reports.borrow_mut().forget(tab);
        self.thumbnails.borrow_mut().remove(&tab);
        self.tab_jars.remove(&tab);
        match self.pages.remove(&tab) {
            Some(_page) => Ok(()),
            None => Err(EngineError(format!("no page for {tab}"))),
        }
    }

    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError> {
        // Releasing the last reference to a non-persistent store discards its data.
        self.jars.remove(&jar);
        Ok(())
    }
}

/// Fills `host` with `view`, below anything else in it (the overlays).
fn place(view: &NSView, host: &NSView) {
    // SAFETY: a getter; the superview is retained for the duration of the comparison.
    let parent = unsafe { view.superview() };
    if parent.is_some_and(|parent| &*parent == host) {
        return;
    }
    view.removeFromSuperview();
    view.setFrame(host.bounds());
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    host.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, None);
}

/// The matches of a find in a tab's page, answering [`WebKitEngine::find`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub tab: TabId,
    /// The query they match, to tell an answer to an older query apart.
    pub query: String,
    pub matches: FindMatches,
}

/// Where pages record reports, and how the shell hears about them.
#[derive(Default)]
struct Inbox {
    reports: RefCell<Reports>,
    progressed: Cell<bool>,
    /// The download center has reports.
    downloaded: Cell<bool>,
    found: RefCell<Vec<Found>>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
    scheduled: Cell<bool>,
}

impl Inbox {
    fn record(inbox: &Weak<Self>, tab: TabId, event: PageEvent) {
        let Some(inbox) = inbox.upgrade() else {
            return;
        };
        {
            let mut reports = inbox.reports.borrow_mut();
            match event {
                PageEvent::Committed { url, title } => {
                    reports.committed(tab, url.as_deref(), title);
                }
                PageEvent::TitleChanged(title) => reports.titled(tab, title),
                PageEvent::Progress => inbox.progressed.set(true),
                // The page's handler gives downloads to the download center instead.
                PageEvent::Download(_) => {}
            }
        }
        Self::schedule(&inbox);
    }

    /// True when nothing waits for the shell.
    fn is_quiet(&self) -> bool {
        self.reports.borrow().is_empty()
            && !self.progressed.get()
            && !self.downloaded.get()
            && self.found.borrow().is_empty()
    }

    /// Wakes the shell on a later turn of the main run loop, once per batch of reports. A shell
    /// too busy to drain leaves them, and the next turn tries again.
    fn schedule(inbox: &Rc<Self>) {
        if inbox.scheduled.get() || inbox.is_quiet() {
            return;
        }
        let Some(wake) = inbox.wake.borrow().clone() else {
            return;
        };
        inbox.scheduled.set(true);
        let weak = Rc::downgrade(inbox);
        let block = RcBlock::new(move || {
            wake();
            if let Some(inbox) = weak.upgrade() {
                inbox.scheduled.set(false);
                Self::schedule(&inbox);
            }
        });
        // SAFETY: the block captures `Rc`s, which is sound because it runs on the main run
        // loop, the thread that scheduled it and owns them.
        unsafe { NSRunLoop::mainRunLoop().performBlock(&block) };
    }
}

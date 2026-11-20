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
//! Pages open pages of their own (`window.open`, `target=_blank`, `⌘`-click): the engine makes
//! them at once, from the configuration `WebKit` passes, so they share the opener's process and
//! jar, and the shell hands each one ([`WebKitEngine::take_opened`]) to `Browsing::adopt`. A
//! page's dialogs and permission requests wait as [`Ask`]s until the shell answers them.
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
              UI, and download delegates and the scheme handler need classes; content rule \
              lists cross threads"
)]

mod downloads;
mod inbox;
mod network;
mod page;
mod reports;
mod scheme;
mod shields;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSImage, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSString, NSURL};
use objc2_web_kit::{WKContentRuleListStore, WKUserContentController, WKWebsiteDataStore};
use quickie_application::{EngineError, EnginePort, Navigation};
use quickie_domain_browsing::{Jar, JarId, TabId, WindowId};
use url::Url;

use downloads::DownloadCenter;
pub use downloads::{DownloadKey, DownloadReport};
pub use inbox::{Ask, AskId, Found, Opened, OpenedPage};
use inbox::{Inbox, Owner};
pub use network::UrlSessionHttp;
pub use page::FindStep;
use page::WebPage;
pub use page::asks::{Answer, AskKind, Dialog};
pub use page::scripts::hints::{Followed, LinkTarget};
pub use reports::Navigated;
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
        // SAFETY: a fresh controller has no preconditions.
        let content = unsafe { WKUserContentController::new(mtm) };
        let inbox = Rc::<Inbox>::default();
        page::install_focus_script(&content, Rc::clone(&inbox.focus), mtm);
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
        *inbox.downloads.borrow_mut() = Rc::downgrade(&downloads);
        Self {
            mtm,
            pages: HashMap::new(),
            hosts: HashMap::new(),
            shown: HashMap::new(),
            jars: HashMap::new(),
            internal: InternalPages::new(internal, mtm),
            content,
            downloads,
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

    /// Reads the rendered text of the tab's page (its main frame), for the agent; `done` gets
    /// it on the main thread, or `None` at once when the tab has no page. The script runs in
    /// Quickie's isolated content world, never in the page's own.
    pub fn page_text(&self, tab: TabId, done: impl FnOnce(Option<String>) + 'static) {
        match self.pages.get(&tab) {
            Some(page) => page.text(move |text| done(Some(text))),
            None => done(None),
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
        let jar = self.inbox.jar_of(tab);
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

    /// The pages pages opened since the last call, oldest first, each to adopt into a tab.
    /// A link opened in another tab gets its page now, in its opener's data store; one whose
    /// opener has closed is dropped.
    pub fn take_opened(&mut self) -> Vec<Opened> {
        let links = std::mem::take(&mut *self.inbox.links.borrow_mut());
        for link in links {
            let (Some(opener), Ok(url)) = (self.pages.get(&link.opener), Url::parse(&link.url))
            else {
                continue;
            };
            let owner = Owner::default();
            let page = WebPage::with_configuration(
                &opener.configuration(),
                Inbox::hooks(&self.inbox, &owner, self.mtm),
                Rc::clone(&self.inbox.focus),
                self.mtm,
            );
            if page.load(&url).is_ok() {
                self.inbox.opened.borrow_mut().push(Opened {
                    opener: link.opener,
                    page: OpenedPage {
                        page,
                        owner,
                        opener: link.opener,
                    },
                    url,
                    focus: link.focus,
                });
            }
        }
        std::mem::take(&mut *self.inbox.opened.borrow_mut())
    }

    /// The tabs whose pages closed themselves (`window.close()`) since the last call.
    pub fn take_closed(&mut self) -> Vec<TabId> {
        std::mem::take(&mut *self.inbox.closed.borrow_mut())
    }

    /// The tabs whose pages entered (`true`) or left full screen since the last call, in order.
    pub fn take_fullscreen(&mut self) -> Vec<(TabId, bool)> {
        std::mem::take(&mut *self.inbox.fullscreen.borrow_mut())
    }

    /// The dialogs and permission requests pages made since the last call, oldest first. Each
    /// waits until [`Self::answer`]; closing its tab declines it.
    pub fn take_asks(&mut self) -> Vec<Ask> {
        self.inbox.take_asks()
    }

    /// Answers an ask; false when its page no longer waits.
    pub fn answer(&mut self, ask: AskId, answer: &Answer) -> bool {
        self.inbox.answer(ask, answer)
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

    /// Lays a new page out at a window's size, so a background page draws and has a picture
    /// for the overview before it is ever shown.
    fn lay_out(&self, page: &WebPage) {
        if let Some(host) = self.hosts.values().next() {
            page.view().setFrame(host.bounds());
        }
    }

    fn page(&self, tab: TabId) -> Result<&WebPage, EngineError> {
        self.pages
            .get(&tab)
            .ok_or_else(|| EngineError(format!("no page for {tab}")))
    }
}

impl EnginePort for WebKitEngine {
    /// A page a page opened, from [`WebKitEngine::take_opened`].
    type Page = OpenedPage;

    /// From now on the page reports for `tab`; what it committed before is reported now.
    fn adopt(&mut self, page: Self::Page, tab: TabId) -> Result<(), EngineError> {
        if self.pages.contains_key(&tab) {
            return Err(EngineError(format!("{tab} already has a page")));
        }
        let OpenedPage {
            page,
            owner,
            opener,
        } = page;
        let jar = self.inbox.jar_of(opener);
        self.inbox.jars.borrow_mut().insert(tab, jar);
        owner.set(Some(tab));
        self.lay_out(&page);
        if let Some((url, title)) = page.committed() {
            self.inbox
                .reports
                .borrow_mut()
                .committed(tab, Some(&url), title);
            Inbox::schedule(&self.inbox);
        }
        self.pages.insert(tab, page);
        Ok(())
    }

    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        if self.pages.contains_key(&tab) {
            return Err(EngineError(format!("{tab} already has a page")));
        }
        let store = self.store(jar);
        // Known before the page exists: a download it starts at once needs its jar.
        self.inbox.jars.borrow_mut().insert(tab, jar);
        let owner: Owner = Rc::new(Cell::new(Some(tab)));
        let page = WebPage::new(
            &store,
            &self.content,
            &self.internal,
            Inbox::hooks(&self.inbox, &owner, self.mtm),
            Rc::clone(&self.inbox.focus),
            self.mtm,
        );
        self.lay_out(&page);
        let loaded = page.load(url);
        self.pages.insert(tab, page);
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
        self.inbox.forget(tab);
        self.thumbnails.borrow_mut().remove(&tab);
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

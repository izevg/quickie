//! Development engine of Quickie: pages on `WKWebView`, so UI work never waits for a Chromium
//! build (`docs/architecture.md` §3, P0). It implements the application's [`EnginePort`].
//!
//! The engine keeps one page per tab, keyed by the domain's [`TabId`]. Main-jar pages share the
//! default data store; each throwaway jar gets one non-persistent store, shared by its tabs and
//! dropped with the jar. The shell registers each window's host view; [`EnginePort::show`]
//! puts a tab's page into it, under any overlays.
//!
//! Everything pages report flows back as one stream of [`EngineEvent`]s, without re-entering
//! the use cases: pages record what they did in one queue, the engine schedules the shell's
//! waker ([`WebKitEngine::set_waker`]) on the main run loop, and the shell drains the queue
//! ([`WebKitEngine::drain`]). Answers to requests (a find, link targets, page text, a picture)
//! come the same way, never through a callback.
//!
//! Pages open pages of their own (`window.open`, `target=_blank`, `⌘`-click): the engine makes
//! them at once, from the configuration `WebKit` passes, so they share the opener's process and
//! jar, and the shell hands each one ([`EngineEvent::Opened`]) to `Browsing::adopt`. A page's
//! dialogs and permission requests wait as asks ([`EngineEvent::Asked`]) until the shell
//! answers them. A page whose web content process died is reported ([`EngineEvent::Crashed`])
//! and stays blank until it is reloaded.
//!
//! Every page shares one user content controller, so the [`WebKitContentBlocker`] the engine
//! hands out ([`WebKitEngine::content_blocker`]) blocks on all of them. [`UrlSessionHttp`] is the
//! network for the browser's own requests, such as fetching filter lists.
//!
//! Navigations that become downloads go to one download center, which saves them into the
//! folder [`WebKitEngine::set_download_directory`] names and reports
//! [`EngineEvent::Download`]s; a page saved as a PDF ([`WebKitEngine::save_pdf`]) is written
//! there too, under a free name.
//!
//! Every page is inspectable (`WKWebView.isInspectable`), so Web Inspector attaches from the
//! page's context menu or Safari's Develop menu; `WebKit` has no public call that opens it.
//! Page zoom ([`WebKitEngine::set_zoom`]) and printing ([`WebKitEngine::print`]) are the
//! view's own.

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
mod scheme;
mod shields;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSImage, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSString, NSURL};
use objc2_web_kit::{WKContentRuleListStore, WKUserContentController, WKWebsiteDataStore};
use quickie_application::{
    Answer, DownloadToken, EngineError, EngineEvent, EnginePort, FindStep, Found, Navigation,
    ReplyToken, Ticket,
};
use quickie_domain_browsing::{Jar, JarId, TabId, WindowId};
use url::Url;

use downloads::DownloadCenter;
pub use inbox::OpenedPage;
use inbox::{Event, Inbox, Owner};
pub use network::UrlSessionHttp;
use page::WebPage;
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
            move |event| {
                if let Some(inbox) = waiting.upgrade() {
                    inbox.push(EngineEvent::Download(event));
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

    /// Takes a new picture of the tab's page, drawn off the main thread; once it is in
    /// [`Self::thumbnail`], [`EngineEvent::Pictured`] says so.
    pub fn capture(&self, tab: TabId) {
        let Some(page) = self.pages.get(&tab) else {
            return;
        };
        let thumbnails = Rc::downgrade(&self.thumbnails);
        let inbox = Rc::downgrade(&self.inbox);
        page.snapshot(THUMBNAIL_WIDTH, move |image| {
            let (Some(image), Some(thumbnails)) = (image, thumbnails.upgrade()) else {
                return;
            };
            thumbnails.borrow_mut().insert(tab, image);
            report(&inbox, EngineEvent::Pictured { tab });
        });
    }

    /// Finds the links and controls on screen in the tab's page, for link hints:
    /// [`EngineEvent::Targets`] with the ticket returned brings them in document order, placed
    /// in the page view, which fills its window's host. `None` when the tab has no page.
    #[must_use = "the ticket tells the answer apart"]
    pub fn link_targets(&self, tab: TabId) -> Option<Ticket> {
        let page = self.pages.get(&tab)?;
        let ticket = self.inbox.ticket();
        let inbox = Rc::downgrade(&self.inbox);
        page.link_targets(move |targets| {
            report(
                &inbox,
                EngineEvent::Targets {
                    tab,
                    ticket,
                    targets,
                },
            );
        });
        Some(ticket)
    }

    /// Follows target `index` of the tab's last [`Self::link_targets`]: the page clicks or
    /// focuses it, or [`EngineEvent::Followed`] with the ticket returned brings a link to open
    /// in another tab (in the background when `background`). `None` when the tab has no page.
    #[must_use = "the ticket tells the answer apart"]
    pub fn follow_link(&self, tab: TabId, index: usize, background: bool) -> Option<Ticket> {
        let page = self.pages.get(&tab)?;
        let ticket = self.inbox.ticket();
        let inbox = Rc::downgrade(&self.inbox);
        page.follow(index, background, move |followed| {
            report(
                &inbox,
                EngineEvent::Followed {
                    tab,
                    ticket,
                    followed,
                },
            );
        });
        Some(ticket)
    }

    /// Reads the rendered text of the tab's page (its main frame), for the agent:
    /// [`EngineEvent::Text`] with the ticket returned brings it. `None` when the tab has no
    /// page. The script runs in Quickie's isolated content world, never in the page's own.
    #[must_use = "the ticket tells the answer apart"]
    pub fn page_text(&self, tab: TabId) -> Option<Ticket> {
        let page = self.pages.get(&tab)?;
        let ticket = self.inbox.ticket();
        let inbox = Rc::downgrade(&self.inbox);
        page.text(move |text| report(&inbox, EngineEvent::Text { tab, ticket, text }));
        Some(ticket)
    }

    /// Saves downloads into `directory` from now on; it is created when the first one comes.
    /// Until this is called, every download is refused.
    pub fn set_download_directory(&self, directory: PathBuf) {
        self.downloads.set_directory(directory);
    }

    /// Stops a running download and deletes the part of its file already written; nothing
    /// more is reported for it.
    pub fn cancel_download(&self, token: DownloadToken) {
        self.downloads.cancel(token);
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

    /// Sets what runs, later on the main run loop and never inside an engine call, when events
    /// wait: it should [`Self::drain`] them. Events already waiting wake it at once.
    pub fn set_waker(&mut self, wake: impl Fn() + 'static) {
        *self.inbox.wake.borrow_mut() = Some(Rc::new(wake));
        Inbox::schedule(&self.inbox);
    }

    /// Moves what pages did since the last call to the end of `events`, oldest first. A link a
    /// page opened in another tab gets its page now, in its opener's data store; one whose
    /// opener has closed is dropped.
    pub fn drain(&mut self, events: &mut Vec<EngineEvent<OpenedPage>>) {
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
                self.inbox.push(EngineEvent::Opened {
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
        self.inbox.drain(events);
    }

    /// How much of a tab's page has loaded, `0.0 ..= 1.0`, while it loads; `None` when it is
    /// not loading or has no page. [`EngineEvent::Loading`] tells when it moves.
    #[must_use]
    pub fn progress(&self, tab: TabId) -> Option<f64> {
        self.pages.get(&tab).and_then(WebPage::progress)
    }

    /// Selects a match of `query` in a tab's page as `step` says, marks every match, and
    /// reports them as [`EngineEvent::Found`]. An empty query clears the marks.
    pub fn find(&self, tab: TabId, query: &str, step: FindStep) {
        let Some(page) = self.pages.get(&tab) else {
            return;
        };
        let inbox = Rc::downgrade(&self.inbox);
        let asked = query.to_owned();
        page.find(query, step, move |matches| {
            report(
                &inbox,
                EngineEvent::Found(Found {
                    tab,
                    query: asked.clone(),
                    matches,
                }),
            );
        });
    }

    /// A tab's page zoom, `1.0` for actual size; `None` for a tab without a page.
    #[must_use]
    pub fn zoom(&self, tab: TabId) -> Option<f64> {
        self.pages.get(&tab).map(WebPage::zoom)
    }

    /// Zooms a tab's page by `factor` (`WKWebView.pageZoom`): text and layout scale, as
    /// Safari's zoom does. The page keeps it across navigations until it is set again.
    pub fn set_zoom(&self, tab: TabId, factor: f64) {
        if let Some(page) = self.pages.get(&tab) {
            page.set_zoom(factor);
        }
    }

    /// Whether Web Inspector may attach to a tab's page: every page, popups included, is
    /// inspectable on macOS 13.3 and later.
    #[must_use]
    pub fn inspectable(&self, tab: TabId) -> bool {
        self.pages.get(&tab).is_some_and(WebPage::inspectable)
    }

    /// Opens the print panel for a tab's page, as a sheet on its window.
    ///
    /// # Errors
    ///
    /// The tab has no page, or its page is not in a window.
    pub fn print(&self, tab: TabId) -> Result<(), EngineError> {
        self.page(tab)?.print()
    }

    /// Saves a tab's page as a PDF in the downloads folder, under its title and a free name;
    /// it is reported as [`EngineEvent::Download`] of [`DownloadEvent::Saved`], or of
    /// [`DownloadEvent::Refused`] when it could not be made or written.
    ///
    /// [`DownloadEvent::Saved`]: quickie_application::DownloadEvent::Saved
    /// [`DownloadEvent::Refused`]: quickie_application::DownloadEvent::Refused
    ///
    /// # Errors
    ///
    /// The tab has no page, or its page has not committed a URL yet.
    pub fn save_pdf(&self, tab: TabId) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        let (url, title) = page
            .committed()
            .ok_or_else(|| EngineError(format!("{tab} has no page to save yet")))?;
        let url = Url::parse(&url).map_err(|error| EngineError(format!("{url}: {error}")))?;
        let name = downloads::pdf_name(title.as_deref(), &url);
        let jar = self.inbox.jar_of(tab);
        let center = Rc::downgrade(&self.downloads);
        page.pdf(move |made| {
            if let Some(center) = center.upgrade() {
                DownloadCenter::save(&center, tab, jar, url, &name, made);
            }
        });
        Ok(())
    }

    /// Answers an ask ([`EngineEvent::Asked`]); false when its page no longer waits.
    pub fn answer(&mut self, token: ReplyToken, answer: &Answer) -> bool {
        self.inbox.answer(token, answer)
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
    /// A page a page opened, from [`EngineEvent::Opened`].
    type Page = OpenedPage;

    /// From now on the page reports for `tab`; what it committed and asked before is reported
    /// now.
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
            self.inbox.committed(tab, Some(&url), title);
        }
        self.inbox.release_asks();
        Inbox::schedule(&self.inbox);
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
            self.capture(previous);
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
            Navigation::HardReload => page.reload_from_origin(),
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

/// Queues an answer that arrived from `WebKit` and wakes the shell for it; nothing when the
/// engine is gone.
fn report(inbox: &Weak<Inbox>, event: Event) {
    if let Some(inbox) = inbox.upgrade() {
        inbox.push(event);
        Inbox::schedule(&inbox);
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

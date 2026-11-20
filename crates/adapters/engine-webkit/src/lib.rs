//! Development engine of Quickie: pages on `WKWebView`, so UI work never waits for a Chromium
//! build (`docs/architecture.md` §3, P0). It implements the application's [`EnginePort`] and
//! [`PageContent`].
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
//! Main-jar pages share one user content controller and throwaway-jar pages another, so the
//! [`WebKitContentBlocker`] the engine hands out ([`WebKitEngine::content_blocker`]) installs
//! the same rules in both and blocks on every page, popups included, and on pages made before
//! its rules were compiled. Its cosmetic filters and scriptlets ride in the same controllers; a
//! frame's document waits, briefly, for its host's. Extensions' content scripts land only in
//! the main jar's controller, so they never reach a throwaway page. [`UrlSessionHttp`] is the
//! network for the browser's own requests, such as fetching filter lists.
//!
//! Navigations that become downloads go to one download center, which saves them into the
//! folder [`Downloads::set_directory`] names and reports [`EngineEvent::Download`]s; a page
//! saved as a PDF ([`Downloads::save_pdf`]) is written there too, under a free name.
//!
//! Every page is inspectable (`WKWebView.isInspectable`), so Web Inspector attaches from the
//! page's context menu or Safari's Develop menu; `WebKit` has no public call that opens it.
//! Page zoom ([`PageZoom`]) and printing ([`WebKitEngine::print`]) are the view's own.
//!
//! Web extensions ([`WebKitExtensions`], from [`WebKitEngine::web_extensions`]) run in the
//! main jar's pages through one `WKWebExtensionController`, on macOS 15.4 and later; never in
//! a throwaway jar's.

#![expect(
    unsafe_code,
    reason = "WebKit through objc2: its bindings mark most methods unsafe; KVO, the navigation \
              UI, download delegates, the scheme handler, and the extension controller's \
              delegate, tabs and windows need classes; content rule lists cross threads"
)]

mod downloads;
mod extensions;
mod inbox;
mod network;
mod page;
mod scheme;
mod shields;
mod thumbnails;
mod user_agent;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSImage, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSSize, NSString, NSURL};
use objc2_web_kit::{WKContentRuleListStore, WKUserContentController, WKWebsiteDataStore};
use quickie_application::{
    Answer, Capabilities, DownloadToken, Downloads, EngineError, EngineEvent, EnginePort, FindStep,
    Found, Navigation, PageAsks, PageContent, PageZoom, ReplyToken, Ticket,
};
use quickie_domain_browsing::{Jar, JarId, TabId, WindowId};
use quickie_domain_settings::ZoomLevel;
use url::Url;

use downloads::DownloadCenter;
pub use extensions::WebKitExtensions;
pub use inbox::OpenedPage;
use inbox::{Event, Inbox, Owner};
pub use network::UrlSessionHttp;
use page::WebPage;
use scheme::InternalPages;
use shields::cosmetic::Cosmetics;
pub use shields::{BlockerStatus, Installed, WebKitContentBlocker};

/// The size, in points, of the tab overview's pictures until the shell sets its own
/// ([`WebKitEngine::set_thumbnail_size`]): the overview's widest card, 16:10.
pub const THUMBNAIL_SIZE: NSSize = NSSize::new(320.0, 200.0);

/// The pictures of live pages, filled by snapshot callbacks.
type Thumbnails = Rc<RefCell<thumbnails::Thumbnails<Retained<NSImage>>>>;

/// The `WebKit` engine. Lives on the main thread, like every view it owns.
pub struct WebKitEngine {
    mtm: MainThreadMarker,
    pages: HashMap<TabId, WebPage>,
    hosts: HashMap<WindowId, Retained<NSView>>,
    shown: HashMap<WindowId, TabId>,
    jars: HashMap<JarId, Retained<WKWebsiteDataStore>>,
    internal: Retained<InternalPages>,
    /// Shared by every main-jar page: the content blocker installs its rule lists and shields
    /// scripts here, and the extension controller its content scripts.
    content: Retained<WKUserContentController>,
    /// Shared by every throwaway-jar page: the same rule lists and shields scripts, and no
    /// extension's content scripts.
    throwaway_content: Retained<WKUserContentController>,
    /// The cosmetic filters and scriptlets of every page, in `content`; the content blocker
    /// gives them their lists.
    cosmetics: Rc<Cosmetics>,
    downloads: Rc<DownloadCenter>,
    inbox: Rc<Inbox>,
    /// The last picture of each page, taken when it was hidden or asked for.
    thumbnails: Thumbnails,
    /// The size pictures are drawn at, in points; the screen's scale multiplies it.
    thumbnail_size: NSSize,
    /// The main jar's extension controller and every page's view, once extensions are on.
    extensions: Option<Rc<extensions::Shared>>,
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
        // SAFETY: as above.
        let throwaway_content = unsafe { WKUserContentController::new(mtm) };
        let inbox = Rc::<Inbox>::default();
        page::install_focus_script(&content, Rc::clone(&inbox.focus), mtm);
        page::install_focus_script(&throwaway_content, Rc::clone(&inbox.focus), mtm);
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
        let cosmetics = Cosmetics::new(vec![content.clone(), throwaway_content.clone()], mtm);
        *inbox.cosmetics.borrow_mut() = Rc::downgrade(&cosmetics);
        Self {
            mtm,
            pages: HashMap::new(),
            hosts: HashMap::new(),
            shown: HashMap::new(),
            jars: HashMap::new(),
            internal: InternalPages::new(internal, mtm),
            content,
            throwaway_content,
            cosmetics,
            downloads,
            inbox,
            thumbnails: Rc::default(),
            thumbnail_size: THUMBNAIL_SIZE,
            extensions: None,
        }
    }

    /// What this engine offers the UI beyond its ports.
    pub const CAPS: Capabilities = Capabilities {
        web_extensions: true,
    };

    /// The main jar's web extensions, made once: pages created from now on run them. Call it
    /// before the first page exists. `None` on a macOS without `WKWebExtension` (before 15.4)
    /// and on a second call.
    pub fn web_extensions(&mut self) -> Option<WebKitExtensions> {
        if self.extensions.is_some() {
            return None;
        }
        let shared = extensions::Shared::new(&user_agent::safari_tail(), self.mtm)?;
        for (&tab, page) in &self.pages {
            if self.inbox.jar_of(tab) == Jar::Main {
                shared
                    .views
                    .borrow_mut()
                    .insert(tab, page.web_view().retain());
            }
        }
        self.extensions = Some(Rc::clone(&shared));
        Some(WebKitExtensions::new(shared, self.mtm))
    }

    /// Lets extensions reach a tab's page, when it is in the main jar.
    fn share_view(&self, tab: TabId) {
        if let (Some(shared), Some(page)) = (&self.extensions, self.pages.get(&tab))
            && self.inbox.jar_of(tab) == Jar::Main
        {
            shared
                .views
                .borrow_mut()
                .insert(tab, page.web_view().retain());
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
        self.thumbnails.borrow().get(tab)
    }

    /// Sets the size, in points, pictures are drawn at from now on: the largest the overview
    /// shows one. A picture is cropped to its shape and is never bigger, so each costs at most
    /// `size` at the screen's scale.
    pub fn set_thumbnail_size(&mut self, size: NSSize) {
        self.thumbnail_size = size;
    }

    /// Takes a new picture of the tab's page, painted by its web content process; once it is
    /// in [`Self::thumbnail`], [`EngineEvent::Pictured`] says so. Nothing for a tab without a
    /// page or a page not laid out yet. A picture of a page that closes meanwhile is dropped.
    pub fn capture(&self, tab: TabId) {
        let Some(page) = self.pages.get(&tab) else {
            return;
        };
        let size = self.thumbnail_size;
        let Some(rect) = thumbnails::crop(page.size(), size) else {
            return;
        };
        let thumbnails = Rc::downgrade(&self.thumbnails);
        let inbox = Rc::downgrade(&self.inbox);
        page.snapshot(rect, size.width, move |image| {
            let image = image.filter(|image| {
                let size = image.size();
                size.width > 0.0 && size.height > 0.0
            });
            let (Some(image), Some(thumbnails)) = (image, thumbnails.upgrade()) else {
                return;
            };
            if thumbnails.borrow_mut().store(tab, image) {
                report(&inbox, EngineEvent::Pictured { tab });
            }
        });
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
            vec![self.content.clone(), self.throwaway_content.clone()],
            Rc::clone(&self.cosmetics),
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

    /// A tab's page zoom, `1.0` for actual size; `None` for a tab without a page.
    #[must_use]
    pub fn zoom(&self, tab: TabId) -> Option<f64> {
        self.pages.get(&tab).map(WebPage::zoom)
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
        self.thumbnails.borrow_mut().open(tab);
        if let Some((url, title)) = page.committed() {
            self.inbox.committed(tab, Some(&url), title);
        }
        self.inbox.release_asks();
        Inbox::schedule(&self.inbox);
        self.pages.insert(tab, page);
        self.share_view(tab);
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
        let (content, extensions) = match jar {
            Jar::Main => (
                &self.content,
                self.extensions.as_ref().map(|shared| &*shared.controller),
            ),
            Jar::Throwaway(_) => (&self.throwaway_content, None),
        };
        let page = WebPage::new(
            &store,
            content,
            &self.internal,
            extensions,
            Inbox::hooks(&self.inbox, &owner, self.mtm),
            Rc::clone(&self.inbox.focus),
            self.mtm,
        );
        self.lay_out(&page);
        self.thumbnails.borrow_mut().open(tab);
        let loaded = page.load(url);
        self.pages.insert(tab, page);
        self.share_view(tab);
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
        self.thumbnails.borrow_mut().close(tab);
        if let Some(shared) = &self.extensions {
            shared.views.borrow_mut().remove(&tab);
        }
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

/// Find is `WKWebView`'s own; text, link targets and following one are scripts in Quickie's
/// isolated content world, never in the page's own. Every answer is queued when `WebKit` hands
/// it over and reaches the shell through the waker.
impl PageContent for WebKitEngine {
    fn find(&mut self, tab: TabId, query: &str, step: FindStep) -> Result<(), EngineError> {
        let page = self.page(tab)?;
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
        Ok(())
    }

    fn read_text(&mut self, tab: TabId, max_chars: usize) -> Result<Ticket, EngineError> {
        let page = self.page(tab)?;
        let ticket = self.inbox.ticket();
        let inbox = Rc::downgrade(&self.inbox);
        page.text(max_chars, move |text| {
            report(&inbox, EngineEvent::Text { tab, ticket, text });
        });
        Ok(ticket)
    }

    /// The targets are placed in the page view, which fills its window's host.
    fn link_targets(&mut self, tab: TabId) -> Result<Ticket, EngineError> {
        let page = self.page(tab)?;
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
        Ok(ticket)
    }

    fn follow_link(
        &mut self,
        tab: TabId,
        index: usize,
        background: bool,
    ) -> Result<Ticket, EngineError> {
        let page = self.page(tab)?;
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
        Ok(ticket)
    }
}

/// Navigations that become downloads, and downloads asked for, go to one download center,
/// which saves them into the folder set here; a page saved as a PDF is written there too,
/// under a free name. A download is fetched in its page's jar.
impl Downloads for WebKitEngine {
    fn set_directory(&mut self, directory: PathBuf) {
        self.downloads.set_directory(directory);
    }

    fn download(&mut self, tab: TabId, url: &Url) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        let jar = self.inbox.jar_of(tab);
        DownloadCenter::start(&self.downloads, page.web_view(), url, tab, jar);
        Ok(())
    }

    /// Stops a running download and deletes the part of its file already written.
    fn cancel(&mut self, token: DownloadToken) {
        self.downloads.cancel(token);
    }

    /// The page as `WKWebView` renders it into a PDF, named after its title.
    fn save_pdf(&mut self, tab: TabId) -> Result<(), EngineError> {
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
}

/// The inbox holds what each waiting page's UI delegate handed `WebKit` to call back with; an
/// answer calls it, and the page goes on.
impl PageAsks for WebKitEngine {
    fn answer(&mut self, token: ReplyToken, answer: &Answer) -> bool {
        self.inbox.answer(token, answer)
    }
}

/// A level is `WKWebView.pageZoom` at its factor: text and layout scale, as Safari's zoom does,
/// and the view keeps it across navigations.
impl PageZoom for WebKitEngine {
    fn set_zoom(&mut self, tab: TabId, level: ZoomLevel) {
        if let Some(page) = self.pages.get(&tab) {
            page.set_zoom(level.factor());
        }
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

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

#![expect(
    unsafe_code,
    reason = "WebKit through objc2: its bindings mark most methods unsafe; KVO, the navigation \
              delegate and the scheme handler need classes"
)]

mod page;
mod reports;
mod scheme;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
use objc2_foundation::NSRunLoop;
use objc2_web_kit::WKWebsiteDataStore;
use quickie_application::{EngineError, EnginePort, Navigation};
use quickie_domain_browsing::{FindMatches, Jar, JarId, TabId, WindowId};
use url::Url;

pub use page::FindStep;
use page::{PageEvent, WebPage};
pub use reports::Navigated;
use reports::Reports;
use scheme::InternalPages;

/// The `WebKit` engine. Lives on the main thread, like every view it owns.
pub struct WebKitEngine {
    mtm: MainThreadMarker,
    pages: HashMap<TabId, WebPage>,
    hosts: HashMap<WindowId, Retained<NSView>>,
    shown: HashMap<WindowId, TabId>,
    jars: HashMap<JarId, Retained<WKWebsiteDataStore>>,
    internal: Retained<InternalPages>,
    inbox: Rc<Inbox>,
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
        Self {
            mtm,
            pages: HashMap::new(),
            hosts: HashMap::new(),
            shown: HashMap::new(),
            jars: HashMap::new(),
            internal: InternalPages::new(internal, mtm),
            inbox: Rc::default(),
        }
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
    /// have reported navigations, loading progress, or the matches of a find. It should drain
    /// [`Self::take_navigations`], [`Self::take_progressed`], and [`Self::take_found`].
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
    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        if self.pages.contains_key(&tab) {
            return Err(EngineError(format!("{tab} already has a page")));
        }
        let store = self.store(jar);
        let inbox = Rc::downgrade(&self.inbox);
        let page = WebPage::new(
            &store,
            &self.internal,
            move |event| Inbox::record(&inbox, tab, event),
            self.mtm,
        );
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
            }
        }
        Self::schedule(&inbox);
    }

    /// True when nothing waits for the shell.
    fn is_quiet(&self) -> bool {
        self.reports.borrow().is_empty() && !self.progressed.get() && self.found.borrow().is_empty()
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

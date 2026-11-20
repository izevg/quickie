//! [`ChromiumEngine`]: the application's [`EnginePort`] on the bridge to `//quickie`, with the
//! bookkeeping the C++ side leaves to Rust: which tabs have a page, which window shows what
//! before its host view exists, and which throwaway jars ever got a profile.

mod inbox;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use quickie_application::{
    EngineError, EngineEvent, EnginePort, FindStep, Navigation, PageContent, PageZoom, Ticket,
};
use quickie_domain_browsing::{Jar, JarId, TabId, WindowId};
use quickie_domain_settings::ZoomLevel;
use url::Url;

use inbox::Inbox;
pub use inbox::PageHandle;

use crate::bridge::{Bridge, BridgeError, Ffi, KeyEquivalent, NavigationKind, dispatch};

/// The Chromium engine, on the main thread (Chromium's UI thread).
///
/// Pages are keyed by the domain's [`TabId`]: the C++ side uses its raw value. Main-jar pages
/// live in the main profile; each throwaway jar is one off-the-record profile, made with its
/// first page and destroyed by [`EnginePort::destroy_jar`]. A window shows pages once the shell
/// has attached its host view; a show that comes earlier waits for it.
///
/// One engine per process: creating it installs the handlers the C++ side reports into, and
/// dropping it removes them.
pub struct ChromiumEngine {
    bridge: Rc<dyn Bridge>,
    inbox: Rc<Inbox>,
    /// Tabs whose page exists in Chromium.
    pages: HashSet<TabId>,
    /// Each window's `NSView*`, bound for good by the first page Chromium shows there.
    hosts: HashMap<WindowId, NonNull<c_void>>,
    /// The tab each window without a host view yet is to show.
    deferred: HashMap<WindowId, TabId>,
    /// Throwaway jars that have an off-the-record profile.
    jars: HashSet<JarId>,
}

impl fmt::Debug for ChromiumEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromiumEngine")
            .field("pages", &self.pages.len())
            .field("hosts", &self.hosts.len())
            .field("deferred", &self.deferred)
            .field("jars", &self.jars.len())
            .finish_non_exhaustive()
    }
}

impl ChromiumEngine {
    /// Connects to Chromium. `keys` decides, for every key-down the engine offers, whether it is
    /// the shell's (`true`) or the page's.
    #[must_use]
    pub fn new(keys: impl FnMut(KeyEquivalent) -> bool + 'static) -> Self {
        Self::with_bridge(Rc::new(Ffi), keys)
    }

    pub(crate) fn with_bridge(
        bridge: Rc<dyn Bridge>,
        keys: impl FnMut(KeyEquivalent) -> bool + 'static,
    ) -> Self {
        let inbox = Rc::new(Inbox::new(Rc::clone(&bridge)));
        let events = Rc::clone(&inbox);
        let wake = Rc::clone(&inbox);
        dispatch::install(
            Box::new(move |event| events.record(event)),
            Box::new(keys),
            Box::new(move || wake.wake()),
        );
        Self {
            bridge,
            inbox,
            pages: HashSet::new(),
            hosts: HashMap::new(),
            deferred: HashMap::new(),
            jars: HashSet::new(),
        }
    }

    /// Makes `host` the view a window's pages fill. The tab shown there before, if any, moves in.
    ///
    /// # Errors
    ///
    /// The window already has another host view (Chromium keeps the first one for good), or
    /// the engine failed to show the tab that waited for it.
    ///
    /// # Safety
    ///
    /// `host` is the `NSView*` of `window` and stays alive while the engine shows pages in it,
    /// that is until the window's tabs are closed or shown in another window.
    pub unsafe fn attach_host(
        &mut self,
        window: WindowId,
        host: NonNull<c_void>,
    ) -> Result<(), EngineError> {
        if self.hosts.get(&window).is_some_and(|&bound| bound != host) {
            return Err(EngineError(format!(
                "{window} already has another host view"
            )));
        }
        self.hosts.insert(window, host);
        match self.deferred.remove(&window) {
            Some(tab) => self.show_in(window, host, tab),
            None => Ok(()),
        }
    }

    /// Sets what runs, later on the main loop and never inside an engine call, when events
    /// wait: it should [`Self::drain`] them. Events already waiting wake it at once.
    pub fn set_waker(&mut self, wake: impl Fn() + 'static) {
        self.inbox.set_waker(Rc::new(wake));
    }

    /// Moves what pages did since the last call to the end of `events`, oldest first.
    pub fn drain(&mut self, events: &mut Vec<EngineEvent<PageHandle>>) {
        self.inbox.drain(events);
    }

    /// Gives a page Chromium opened its tab id; from now on it is that tab's page.
    fn bind_opened(&mut self, page: PageHandle, tab: TabId) -> Result<(), EngineError> {
        self.vacant(tab)?;
        self.bridge.adopt_tab(page.0, tab.get()).map_err(|cause| {
            failed(format_args!("adopt opened page {} as {tab}", page.0), cause)
        })?;
        self.pages.insert(tab);
        Ok(())
    }

    fn vacant(&self, tab: TabId) -> Result<(), EngineError> {
        if self.pages.contains(&tab) {
            return Err(EngineError(format!("{tab} already has a page")));
        }
        Ok(())
    }

    fn live(&self, tab: TabId) -> Result<(), EngineError> {
        if self.pages.contains(&tab) {
            Ok(())
        } else {
            Err(EngineError(format!("no page for {tab}")))
        }
    }

    /// Fails a page-content request the bridge cannot carry yet: `action` names it.
    fn unsupported<T>(&self, tab: TabId, action: &str) -> Result<T, EngineError> {
        self.live(tab)?;
        Err(EngineError(format!(
            "{action} the page of {tab}: the Chromium bridge cannot yet"
        )))
    }

    fn show_in(
        &self,
        window: WindowId,
        host: NonNull<c_void>,
        tab: TabId,
    ) -> Result<(), EngineError> {
        // SAFETY: `host` was attached for `window` through `attach_host`, whose caller keeps it
        // alive while the engine shows pages in it.
        unsafe { self.bridge.show_tab(window.get(), host, tab.get()) }
            .map_err(|cause| failed(format_args!("show {tab} in {window}"), cause))
    }
}

impl Drop for ChromiumEngine {
    fn drop(&mut self) {
        dispatch::uninstall();
    }
}

impl EnginePort for ChromiumEngine {
    type Page = PageHandle;

    fn adopt(&mut self, page: PageHandle, tab: TabId) -> Result<(), EngineError> {
        self.bind_opened(page, tab)
    }

    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        self.vacant(tab)?;
        let (throwaway, raw_jar) = match jar {
            Jar::Main => (false, 0),
            Jar::Throwaway(id) => (true, id.get()),
        };
        self.bridge
            .create_tab(tab.get(), url.as_str(), throwaway, raw_jar)
            .map_err(|cause| failed(format_args!("create the page for {tab}"), cause))?;
        self.pages.insert(tab);
        if let Jar::Throwaway(id) = jar {
            self.jars.insert(id);
        }
        Ok(())
    }

    /// Shows `tab` in `window`. A window whose host view is not attached yet shows it once
    /// [`ChromiumEngine::attach_host`] runs; a tab shown somewhere no longer waits elsewhere.
    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError> {
        self.live(tab)?;
        self.deferred.retain(|_, waiting| *waiting != tab);
        if let Some(&host) = self.hosts.get(&window) {
            self.show_in(window, host, tab)
        } else {
            self.deferred.insert(window, tab);
            Ok(())
        }
    }

    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError> {
        self.live(tab)?;
        let (kind, url, action) = match navigation {
            Navigation::Load(url) => (NavigationKind::Load, url.as_str(), "load "),
            Navigation::Back => (NavigationKind::Back, "", "go back"),
            Navigation::Forward => (NavigationKind::Forward, "", "go forward"),
            Navigation::Reload => (NavigationKind::Reload, "", "reload"),
            Navigation::HardReload => (NavigationKind::HardReload, "", "reload past the cache"),
            Navigation::Stop => (NavigationKind::Stop, "", "stop loading"),
        };
        self.bridge
            .navigate_tab(tab.get(), kind, url)
            .map_err(|cause| failed(format_args!("{action}{url} in {tab}"), cause))
    }

    /// Destroys a tab's page. Rust forgets it even if Chromium fails, as Chromium does: its
    /// registry drops the page before it looks for the tab strip that holds it.
    fn close(&mut self, tab: TabId) -> Result<(), EngineError> {
        if !self.pages.remove(&tab) {
            return Err(EngineError(format!("no page for {tab}")));
        }
        self.deferred.retain(|_, waiting| *waiting != tab);
        self.inbox.forget(tab);
        self.bridge
            .close_tab(tab.get())
            .map_err(|cause| failed(format_args!("close the page of {tab}"), cause))
    }

    /// Destroys a throwaway jar's profile. A jar none of whose tabs ever got a page has no
    /// profile, and so nothing to destroy.
    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError> {
        if !self.jars.contains(&jar) {
            return Ok(());
        }
        self.bridge.destroy_jar(jar.get()).map_err(|cause| {
            failed(
                format_args!(
                    "destroy throwaway jar {} (a page may still use it)",
                    jar.get()
                ),
                cause,
            )
        })?;
        self.jars.remove(&jar);
        Ok(())
    }
}

/// The bridge has no page content yet: every request fails, without calling Chromium, and says
/// so; a tab without a page fails as it does everywhere else.
impl PageContent for ChromiumEngine {
    fn find(&mut self, tab: TabId, _query: &str, _step: FindStep) -> Result<(), EngineError> {
        self.unsupported(tab, "find in")
    }

    fn read_text(&mut self, tab: TabId, _max_chars: usize) -> Result<Ticket, EngineError> {
        self.unsupported(tab, "read the text of")
    }

    fn link_targets(&mut self, tab: TabId) -> Result<Ticket, EngineError> {
        self.unsupported(tab, "find link targets in")
    }

    fn follow_link(&mut self, tab: TabId, _: usize, _: bool) -> Result<Ticket, EngineError> {
        self.unsupported(tab, "follow a link target in")
    }
}

/// The bridge cannot zoom pages yet: every page stays at actual size.
impl PageZoom for ChromiumEngine {
    fn set_zoom(&mut self, _tab: TabId, _level: ZoomLevel) {}
}

/// An engine error naming what failed and why.
fn failed(action: fmt::Arguments<'_>, cause: BridgeError) -> EngineError {
    EngineError(format!("{action}: {cause}"))
}

#[cfg(test)]
mod tests;

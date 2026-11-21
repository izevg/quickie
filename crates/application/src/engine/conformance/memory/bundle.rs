//! The memory engine as a whole engine: its pages on a surface that draws nothing, a network
//! that reaches nothing, and a content blocker whose rules its pages read.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use quickie_domain_browsing::{TabId, Timestamp, WindowId};
use url::Url;

use super::{MemoryEngine, MemoryPage, rules_in};
use crate::{
    ActiveRules, BlockerError, BlockerRules, Capabilities, ContentBlockerPort, DevTools, Ended,
    Engine, EngineError, EngineEvent, Flow, HttpError, HttpEvent, HttpPort, HttpRequest, Inspector,
    NoExtensions, PageSurface, RulesStatus, StopSignal,
};

/// What a page of the memory engine is drawn in: nothing to look at, only there or not.
#[derive(Debug, PartialEq, Eq)]
pub struct MemoryView;

/// The view every page of the memory engine has.
static VIEW: MemoryView = MemoryView;

/// The memory engine's network: its pages are in memory, so no request of the browser's own
/// goes anywhere, and each fails.
#[derive(Clone, Copy, Debug)]
pub struct MemoryHttp;

/// Why every request of [`MemoryHttp`] fails.
const OFFLINE: &str = "the memory engine has no network";

impl HttpPort for MemoryHttp {
    fn get(&self, _url: &Url) -> Result<Vec<u8>, HttpError> {
        Err(HttpError(OFFLINE.to_owned()))
    }

    fn stream(
        &self,
        _request: &HttpRequest,
        _stop: &StopSignal,
        _on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        Err(HttpError(OFFLINE.to_owned()))
    }
}

/// The memory engine's content blocker: the rules it installs are the ones its pages load with
/// from their next load on. Nothing compiles, so rules are active, cosmetic filters included,
/// as soon as they are installed, and they never settle later.
#[derive(Debug)]
pub struct MemoryBlocker {
    rules: Rc<RefCell<BlockerRules>>,
    installed: bool,
}

impl MemoryBlocker {
    pub(super) fn new(rules: Rc<RefCell<BlockerRules>>) -> Self {
        Self {
            rules,
            installed: false,
        }
    }
}

impl ContentBlockerPort for MemoryBlocker {
    fn install(&mut self, rules: &BlockerRules) -> Result<(), BlockerError> {
        rules.clone_into(&mut self.rules.borrow_mut());
        self.installed = true;
        Ok(())
    }

    /// Active once installed, compiled at the epoch: the memory engine keeps no time.
    fn status(&self) -> RulesStatus {
        if !self.installed {
            return RulesStatus::Idle;
        }
        let rules = self.rules.borrow();
        RulesStatus::Active(ActiveRules {
            rules: rules
                .lists
                .iter()
                .map(|list| rules_in(&list.body).len())
                .sum(),
            lists: rules.lists.len(),
            compiled: Timestamp::from_millis(0),
        })
    }

    fn cosmetics_ready(&self) -> bool {
        true
    }

    fn on_settled(&mut self, _settled: Box<dyn Fn()>) {}
}

/// Nothing is drawn: a page has a view while it exists, is loading until its turn loads it,
/// and its picture is where it was when captured.
impl PageSurface for MemoryEngine {
    type View = MemoryView;
    type Picture = Url;

    fn attach_host(&mut self, _window: WindowId, _host: &MemoryView) -> Result<(), EngineError> {
        Ok(())
    }

    fn detach_host(&mut self, _window: WindowId) {}

    fn view(&self, tab: TabId) -> Option<&MemoryView> {
        self.pages.get(&tab).map(|_| &VIEW)
    }

    fn input_focused(&self, tab: TabId) -> bool {
        self.pages.get(&tab).is_some_and(|page| page.field_focused)
    }

    fn loading(&self, tab: TabId) -> Option<f64> {
        self.pages
            .get(&tab)
            .and_then(|page| page.loading.as_ref())
            .map(|_| 0.0)
    }

    fn capture(&mut self, tab: TabId) {
        if let Some(url) = self.pages.get(&tab).and_then(MemoryPage::url) {
            self.pictures.insert(tab, url.clone());
            self.events.push(EngineEvent::Pictured { tab });
        }
    }

    fn picture(&self, tab: TabId) -> Option<Url> {
        self.pictures.get(&tab).cloned()
    }

    fn print(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.page(tab).map(drop)
    }

    fn inspect(&mut self, tab: TabId, _asked: DevTools) -> Result<(), EngineError> {
        self.page(tab).map(drop)
    }
}

impl Engine for MemoryEngine {
    type Http = MemoryHttp;
    type Blocker = MemoryBlocker;
    type Extensions = NoExtensions<MemoryView>;

    const CAPS: Capabilities = Capabilities {
        web_extensions: false,
        inspector: Inspector::Programmatic,
        blocked_counts: false,
    };

    fn http(&self) -> MemoryHttp {
        MemoryHttp
    }

    fn web_extensions(&mut self) -> Option<NoExtensions<MemoryView>> {
        None
    }

    fn content_blocker(&mut self, _directory: &Path) -> Result<MemoryBlocker, EngineError> {
        Ok(MemoryBlocker::new(Rc::clone(&self.rules)))
    }

    fn set_waker(&mut self, wake: Box<dyn Fn()>) {
        self.wake = Some(wake);
    }

    fn drain(&mut self, into: &mut Vec<EngineEvent<MemoryPage>>) {
        self.events.drain_into(into);
    }
}

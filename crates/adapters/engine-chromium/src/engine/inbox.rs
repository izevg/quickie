//! Where engine events wait for the shell, and how the shell hears about them.
//!
//! The C++ side reports events from inside bridge calls (adopting a page reports its state at
//! once) and from Chromium's own tasks. Either way the inbox only records them and asks the
//! bridge for a wake; the shell's waker runs from that posted task, so it never re-enters the use
//! cases from inside an engine call.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use quickie_application::Navigated;
use quickie_domain_browsing::TabId;
use url::Url;

use super::reports::Reports;
use crate::bridge::{Bridge, Event};

/// The engine's handle for a page Chromium opened, until it is adopted as a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageHandle(pub(crate) u64);

/// A page opened a tab or a popup (`window.open`, a link with a target). It has no tab id yet:
/// the shell adopts it with [`crate::ChromiumEngine::adopt`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedPage {
    /// The engine's handle for the new page.
    pub page: PageHandle,
    /// The tab that opened it, if the engine knows.
    pub opener: Option<TabId>,
    /// The URL it is opening; `None` for a blank page or one Chromium could not name.
    pub url: Option<Url>,
}

/// Events waiting for the shell.
pub(crate) struct Inbox {
    bridge: Rc<dyn Bridge>,
    reports: RefCell<Reports>,
    opened: RefCell<Vec<OpenedPage>>,
    waker: RefCell<Option<Rc<dyn Fn()>>>,
    /// A wake is posted and has not run yet.
    scheduled: Cell<bool>,
}

impl Inbox {
    pub(crate) fn new(bridge: Rc<dyn Bridge>) -> Self {
        Self {
            bridge,
            reports: RefCell::default(),
            opened: RefCell::default(),
            waker: RefCell::default(),
            scheduled: Cell::new(false),
        }
    }

    /// Records an event from the C++ side, translated into the domain's ids.
    pub(crate) fn record(&self, event: Event) {
        match event {
            Event::NavigationCommitted { tab, url } => {
                self.reports.borrow_mut().committed(TabId::new(tab), &url);
            }
            Event::TitleChanged { tab, title } => {
                self.reports.borrow_mut().titled(TabId::new(tab), title);
            }
            Event::PageOpenedTab { page, opener, url } => {
                self.opened.borrow_mut().push(OpenedPage {
                    page: PageHandle(page),
                    opener: opener.map(TabId::new),
                    url: Url::parse(&url).ok(),
                });
            }
            Event::Unknown => return,
        }
        self.schedule();
    }

    /// Sets what runs when events are waiting, and wakes it for those already here.
    pub(crate) fn set_waker(&self, waker: Rc<dyn Fn()>) {
        *self.waker.borrow_mut() = Some(waker);
        self.schedule();
    }

    /// The posted wake runs: the shell drains. Events it leaves, or that arrive while it runs,
    /// get the next wake.
    pub(crate) fn wake(&self) {
        let waker = self.waker.borrow().clone();
        if let Some(waker) = waker {
            waker();
        }
        self.scheduled.set(false);
        self.schedule();
    }

    pub(crate) fn take_navigations(&self) -> Vec<Navigated> {
        self.reports.borrow_mut().take()
    }

    pub(crate) fn take_opened(&self) -> Vec<OpenedPage> {
        std::mem::take(&mut *self.opened.borrow_mut())
    }

    /// The tab's page is gone; what it reported is dropped.
    pub(crate) fn forget(&self, tab: TabId) {
        self.reports.borrow_mut().forget(tab);
    }

    /// Posts one wake per batch of events, once there is a waker to run.
    fn schedule(&self) {
        let waiting = !self.reports.borrow().is_empty() || !self.opened.borrow().is_empty();
        if self.scheduled.get() || !waiting || self.waker.borrow().is_none() {
            return;
        }
        self.scheduled.set(true);
        self.bridge.schedule_wake();
    }
}

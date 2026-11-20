//! Where engine events wait for the shell, and how the shell hears about them.
//!
//! The C++ side reports events from inside bridge calls (adopting a page reports its state at
//! once) and from Chromium's own tasks. Either way the inbox only records them, as
//! [`EngineEvent`]s in one queue, and asks the bridge for a wake; the shell's waker runs from
//! that posted task, so it never re-enters the use cases from inside an engine call.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use quickie_application::{EngineEvent, EventQueue};
use quickie_domain_browsing::{Focus, Profile, TabId};
use url::Url;

use crate::bridge::{Bridge, Event};

/// Where a page opened without a URL Chromium can name is.
const BLANK: &str = "about:blank";

/// The tab a serial from the C++ side names. The fork hosts only the main profile for now, so
/// its serials are the main profile's.
fn main_tab(serial: u64) -> TabId {
    TabId::new(Profile::Main, serial)
}

/// The engine's handle for a page Chromium opened, until it is adopted as a tab: what
/// [`EngineEvent::Opened`] carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageHandle(pub(crate) u64);

/// Events waiting for the shell.
pub(crate) struct Inbox {
    bridge: Rc<dyn Bridge>,
    events: RefCell<EventQueue<PageHandle>>,
    waker: RefCell<Option<Rc<dyn Fn()>>>,
    /// A wake is posted and has not run yet.
    scheduled: Cell<bool>,
}

impl Inbox {
    pub(crate) fn new(bridge: Rc<dyn Bridge>) -> Self {
        Self {
            bridge,
            events: RefCell::default(),
            waker: RefCell::default(),
            scheduled: Cell::new(false),
        }
    }

    /// Records an event from the C++ side, translated into the domain's ids. A commit whose URL
    /// does not parse is not one the session can keep, and a page opened by no tab Chromium
    /// names has no window to join, so both are dropped.
    pub(crate) fn record(&self, event: Event) {
        let mut events = self.events.borrow_mut();
        match event {
            Event::NavigationCommitted { tab, url } => {
                let Ok(url) = Url::parse(&url) else {
                    return;
                };
                // The title stays until the page reports a new one, because a same-document
                // navigation (a fragment, `history.pushState`) keeps it.
                //
                // TODO(verify): that Chromium reports `TitleWasSet` after every cross-document
                // commit, even for a page without a `<title>`
                // (`content/browser/web_contents/web_contents_impl.cc`); otherwise the previous
                // page's title lingers until the next one.
                events.committed(main_tab(tab), url, None);
            }
            Event::TitleChanged { tab, title } => events.titled(main_tab(tab), title),
            Event::PageOpenedTab { page, opener, url } => {
                let (Some(opener), Ok(url)) =
                    (opener, Url::parse(&url).or_else(|_| Url::parse(BLANK)))
                else {
                    return;
                };
                events.push(EngineEvent::Opened {
                    opener: main_tab(opener),
                    page: PageHandle(page),
                    url,
                    focus: Focus::Foreground,
                });
            }
            Event::Unknown => return,
        }
        drop(events);
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

    /// Moves every waiting event to the end of `into`, oldest first.
    pub(crate) fn drain(&self, into: &mut Vec<EngineEvent<PageHandle>>) {
        self.events.borrow_mut().drain_into(into);
    }

    /// The tab's page is gone; what it reported is dropped.
    pub(crate) fn forget(&self, tab: TabId) {
        self.events.borrow_mut().forget(tab);
    }

    /// Posts one wake per batch of events, once there is a waker to run.
    fn schedule(&self) {
        let waiting = !self.events.borrow().is_empty();
        if self.scheduled.get() || !waiting || self.waker.borrow().is_none() {
            return;
        }
        self.scheduled.set(true);
        self.bridge.schedule_wake();
    }
}

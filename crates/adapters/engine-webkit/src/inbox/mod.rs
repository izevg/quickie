//! Where pages record what they report and ask, and how the shell hears about it: the engine
//! schedules the shell's waker on the main run loop, never inside a call into a view, and the
//! shell takes each queue.
//!
//! A page reports for its [`Owner`], the tab it belongs to. A page `WebKit` opened by itself has
//! none until the session adopts it; what it reports meanwhile waits (asks) or is read again at
//! adoption (its URL).

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_foundation::NSRunLoop;
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
use quickie_domain_browsing::{FindMatches, Focus, Jar, TabId};
use url::Url;

use crate::downloads::DownloadCenter;
use crate::page::asks::{Answer, AskKind, Reply};
use crate::page::{FocusReports, Hooks, PageEvent, WebPage};
use crate::reports::Reports;

/// The tab a page reports for; `None` while a page the engine opened waits to be adopted.
pub(crate) type Owner = Rc<Cell<Option<TabId>>>;

/// The matches of a find in a tab's page, answering `WebKitEngine::find`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub tab: TabId,
    /// The query they match, to tell an answer to an older query apart.
    pub query: String,
    pub matches: FindMatches,
}

/// A page the engine opened by itself, until the session adopts it: the `WebKit` engine's
/// `EnginePort::Page`.
pub struct OpenedPage {
    pub(crate) page: WebPage,
    pub(crate) owner: Owner,
    /// The tab whose page opened it: the new tab shares its jar.
    pub(crate) opener: TabId,
}

impl std::fmt::Debug for OpenedPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedPage").finish_non_exhaustive()
    }
}

/// A page a page opened: a popup, a `target=_blank` link, a `⌘`-click. The shell hands
/// [`Opened::page`] to `Browsing::adopt`.
#[derive(Debug)]
pub struct Opened {
    pub opener: TabId,
    pub page: OpenedPage,
    /// Where it goes; `about:blank` for a page opened without a URL.
    pub url: Url,
    pub focus: Focus,
}

/// Names an ask until it is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AskId(u64);

/// A question a page waits on: answer it with `WebKitEngine::answer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub id: AskId,
    pub tab: TabId,
    /// The origin that asks, as a URL (`https://meet.example/`); `about:blank` for a frame
    /// without one.
    pub origin: Url,
    pub kind: AskKind,
}

/// An ask whose page may not have a tab yet.
struct Waiting {
    id: AskId,
    owner: Owner,
    origin: Url,
    kind: AskKind,
}

/// A link to open in another tab, as its page asked.
pub(crate) struct Link {
    pub(crate) opener: TabId,
    pub(crate) url: String,
    pub(crate) focus: Focus,
}

/// Every queue the shell drains.
#[derive(Default)]
pub(crate) struct Inbox {
    pub(crate) reports: RefCell<Reports>,
    /// Whether each page's focused element takes text, as its focus script last said.
    pub(crate) focus: FocusReports,
    pub(crate) progressed: Cell<bool>,
    /// The download center has reports.
    pub(crate) downloaded: Cell<bool>,
    /// Where a page's downloads go; set once the center exists.
    pub(crate) downloads: RefCell<Weak<DownloadCenter>>,
    /// The jar of each page, for the downloads it starts.
    pub(crate) jars: RefCell<HashMap<TabId, Jar>>,
    pub(crate) found: RefCell<Vec<Found>>,
    pub(crate) opened: RefCell<Vec<Opened>>,
    pub(crate) links: RefCell<Vec<Link>>,
    pub(crate) closed: RefCell<Vec<TabId>>,
    pub(crate) fullscreen: RefCell<Vec<(TabId, bool)>>,
    asks: RefCell<Vec<Waiting>>,
    replies: RefCell<HashMap<AskId, (Owner, Reply)>>,
    next_ask: Cell<u64>,
    pub(crate) wake: RefCell<Option<Rc<dyn Fn()>>>,
    scheduled: Cell<bool>,
}

impl Inbox {
    /// What a page owned by `owner` needs to report and ask through this inbox.
    pub(crate) fn hooks(inbox: &Rc<Self>, owner: &Owner, mtm: MainThreadMarker) -> Hooks {
        let (events, asks, opens) = (
            Rc::downgrade(inbox),
            Rc::downgrade(inbox),
            Rc::downgrade(inbox),
        );
        let (event_owner, ask_owner, opener) =
            (Rc::clone(owner), Rc::clone(owner), Rc::clone(owner));
        Hooks {
            on_event: Box::new(move |event| Self::record(&events, &event_owner, event)),
            on_ask: Box::new(move |origin, kind, reply| {
                Self::ask(&asks, &ask_owner, origin, kind, reply);
            }),
            open: Box::new(move |configuration, url, focus| {
                Self::open(&opens, &opener, configuration, url, focus, mtm)
            }),
        }
    }

    fn record(inbox: &Weak<Self>, owner: &Owner, event: PageEvent) {
        let (Some(inbox), Some(tab)) = (inbox.upgrade(), owner.get()) else {
            return;
        };
        match event {
            PageEvent::Committed { url, title } => {
                inbox
                    .reports
                    .borrow_mut()
                    .committed(tab, url.as_deref(), title);
            }
            PageEvent::TitleChanged(title) => inbox.reports.borrow_mut().titled(tab, title),
            PageEvent::Progress => inbox.progressed.set(true),
            PageEvent::OpenLink { url, focus } => inbox.links.borrow_mut().push(Link {
                opener: tab,
                url,
                focus,
            }),
            PageEvent::Closed => inbox.closed.borrow_mut().push(tab),
            PageEvent::Fullscreen(on) => inbox.fullscreen.borrow_mut().push((tab, on)),
            PageEvent::Download(download) => {
                let center = inbox.downloads.borrow().upgrade();
                if let Some(center) = center {
                    center.adopt(tab, inbox.jar_of(tab), &download);
                }
            }
        }
        Self::schedule(&inbox);
    }

    /// The jar of a tab's page; the main one for a tab the engine never saw.
    pub(crate) fn jar_of(&self, tab: TabId) -> Jar {
        self.jars.borrow().get(&tab).copied().unwrap_or(Jar::Main)
    }

    fn ask(inbox: &Weak<Self>, owner: &Owner, origin: Url, kind: AskKind, reply: Reply) {
        let Some(inbox) = inbox.upgrade() else {
            return;
        };
        let id = AskId(inbox.next_ask.get());
        inbox.next_ask.set(id.0 + 1);
        inbox
            .replies
            .borrow_mut()
            .insert(id, (Rc::clone(owner), reply));
        inbox.asks.borrow_mut().push(Waiting {
            id,
            owner: Rc::clone(owner),
            origin,
            kind,
        });
        Self::schedule(&inbox);
    }

    /// Makes the page `WebKit` asked for, from the configuration it passed, and queues it for
    /// adoption. A page whose own page was not adopted yet opens nothing.
    fn open(
        inbox: &Weak<Self>,
        opener: &Owner,
        configuration: &WKWebViewConfiguration,
        url: Url,
        focus: Focus,
        mtm: MainThreadMarker,
    ) -> Option<Retained<WKWebView>> {
        let inbox = inbox.upgrade()?;
        let opener = opener.get()?;
        let owner = Owner::default();
        let page = WebPage::with_configuration(
            configuration,
            Self::hooks(&inbox, &owner, mtm),
            Rc::clone(&inbox.focus),
            mtm,
        );
        let view = page.web_view().retain();
        inbox.opened.borrow_mut().push(Opened {
            opener,
            page: OpenedPage {
                page,
                owner,
                opener,
            },
            url,
            focus,
        });
        Self::schedule(&inbox);
        Some(view)
    }

    /// The asks of pages that have a tab, oldest first; those of pages still waiting to be
    /// adopted stay queued.
    pub(crate) fn take_asks(&self) -> Vec<Ask> {
        let mut taken = Vec::new();
        self.asks
            .borrow_mut()
            .retain(|waiting| match waiting.owner.get() {
                Some(tab) => {
                    taken.push(Ask {
                        id: waiting.id,
                        tab,
                        origin: waiting.origin.clone(),
                        kind: waiting.kind.clone(),
                    });
                    false
                }
                None => true,
            });
        taken
    }

    /// Sends the answer to the page that waits on `id`; false when it no longer waits.
    pub(crate) fn answer(&self, id: AskId, answer: &Answer) -> bool {
        let reply = self.replies.borrow_mut().remove(&id);
        reply.map(|(_, reply)| reply.answer(answer)).is_some()
    }

    /// The tab's page is gone: its asks are declined, its reports dropped.
    pub(crate) fn forget(&self, tab: TabId) {
        let declined: Vec<(Owner, Reply)> = {
            let mut replies = self.replies.borrow_mut();
            let ids: Vec<AskId> = replies
                .iter()
                .filter(|(_, (owner, _))| owner.get() == Some(tab))
                .map(|(&id, _)| id)
                .collect();
            ids.iter().filter_map(|id| replies.remove(id)).collect()
        };
        self.asks
            .borrow_mut()
            .retain(|waiting| waiting.owner.get() != Some(tab));
        self.reports.borrow_mut().forget(tab);
        self.jars.borrow_mut().remove(&tab);
        // Dropped outside the borrows: each reply calls into WebKit.
        drop(declined);
    }

    /// True when nothing waits for the shell.
    fn is_quiet(&self) -> bool {
        self.reports.borrow().is_empty()
            && !self.progressed.get()
            && !self.downloaded.get()
            && self.found.borrow().is_empty()
            && self.opened.borrow().is_empty()
            && self.links.borrow().is_empty()
            && self.closed.borrow().is_empty()
            && self.fullscreen.borrow().is_empty()
            && self
                .asks
                .borrow()
                .iter()
                .all(|waiting| waiting.owner.get().is_none())
    }

    /// Wakes the shell on a later turn of the main run loop, once per batch of reports. A shell
    /// too busy to drain leaves them, and the next turn tries again.
    pub(crate) fn schedule(inbox: &Rc<Self>) {
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

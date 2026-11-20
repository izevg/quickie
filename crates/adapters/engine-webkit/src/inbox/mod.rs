//! Where pages record what they report and ask, and how the shell hears about it: every report
//! becomes an [`EngineEvent`] in one [`EventQueue`], the engine schedules the shell's waker on
//! the main run loop, never inside a call into a view, and the shell drains the queue.
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
use quickie_application::{Answer, Ask, AskKind, EngineEvent, EventQueue, ReplyToken, Ticket};
use quickie_domain_browsing::{Focus, Jar, TabId};
use url::Url;

use crate::downloads::DownloadCenter;
use crate::page::asks::Reply;
use crate::page::{FocusReports, Hooks, PageEvent, WebPage};
use crate::shields::cosmetic::Cosmetics;

/// The tab a page reports for; `None` while a page the engine opened waits to be adopted.
pub(crate) type Owner = Rc<Cell<Option<TabId>>>;

/// What the `WebKit` engine reports.
pub(crate) type Event = EngineEvent<OpenedPage>;

/// A page the engine opened by itself, until the session adopts it: the `WebKit` engine's
/// `EnginePort::Page`, carried by [`EngineEvent::Opened`].
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

/// An ask of a page that has no tab yet.
struct Waiting {
    token: ReplyToken,
    owner: Owner,
    origin: Url,
    kind: AskKind,
}

/// A link to open in another tab, as its page asked. Its page is made when the shell drains,
/// from its opener's configuration.
pub(crate) struct Link {
    pub(crate) opener: TabId,
    pub(crate) url: String,
    pub(crate) focus: Focus,
}

/// The engine's queue and what it needs to fill it.
#[derive(Default)]
pub(crate) struct Inbox {
    pub(crate) events: RefCell<EventQueue<OpenedPage>>,
    /// Whether each page's focused element takes text, as its focus script last said.
    pub(crate) focus: FocusReports,
    /// Where a page's downloads go; set once the center exists.
    pub(crate) downloads: RefCell<Weak<DownloadCenter>>,
    /// The cosmetic shields a frame's document waits for; set once they exist.
    pub(crate) cosmetics: RefCell<Weak<Cosmetics>>,
    /// The jar of each page, for the downloads it starts.
    pub(crate) jars: RefCell<HashMap<TabId, Jar>>,
    pub(crate) links: RefCell<Vec<Link>>,
    /// Asks of pages that wait to be adopted; they join the queue with their tab.
    waiting: RefCell<Vec<Waiting>>,
    replies: RefCell<HashMap<ReplyToken, (Owner, Reply)>>,
    next_ask: Cell<u64>,
    next_ticket: Cell<u64>,
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
        let (starting, arriving) = (
            inbox.cosmetics.borrow().clone(),
            inbox.cosmetics.borrow().clone(),
        );
        Hooks {
            on_navigation: Box::new(move |url| {
                if let Some(cosmetics) = starting.upgrade() {
                    Cosmetics::expect(&cosmetics, url);
                }
            }),
            before_document: Box::new(move |url, go| match arriving.upgrade() {
                Some(cosmetics) => Cosmetics::prepare(&cosmetics, url, go),
                None => go(),
            }),
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
            PageEvent::Committed { url, title } => inbox.committed(tab, url.as_deref(), title),
            PageEvent::TitleChanged(title) => inbox
                .events
                .borrow_mut()
                .titled(tab, title.unwrap_or_default()),
            PageEvent::Progress(progress) => inbox.events.borrow_mut().loading(tab, progress),
            PageEvent::OpenLink { url, focus } => inbox.links.borrow_mut().push(Link {
                opener: tab,
                url,
                focus,
            }),
            PageEvent::Closed => inbox.push(EngineEvent::ClosedByPage { tab }),
            PageEvent::Fullscreen(entered) => {
                inbox.push(EngineEvent::Fullscreen { tab, entered });
            }
            PageEvent::Crashed => inbox.push(EngineEvent::Crashed { tab }),
            PageEvent::Blocked => inbox.events.borrow_mut().blocked(tab),
            PageEvent::Download(download) => {
                let center = inbox.downloads.borrow().upgrade();
                if let Some(center) = center {
                    center.adopt(tab, inbox.jar_of(tab), &download);
                }
            }
        }
        Self::schedule(&inbox);
    }

    /// A tab's page committed `url`, titled `title` (none when it has none yet). A URL that
    /// does not parse is not one the session can keep, so it is dropped.
    pub(crate) fn committed(&self, tab: TabId, url: Option<&str>, title: Option<String>) {
        if let Some(url) = url.and_then(|url| Url::parse(url).ok()) {
            self.events
                .borrow_mut()
                .committed(tab, url, Some(title.unwrap_or_default()));
        }
    }

    /// Queues an event as it is; the caller schedules the wake.
    pub(crate) fn push(&self, event: Event) {
        self.events.borrow_mut().push(event);
    }

    /// The next request's ticket.
    pub(crate) fn ticket(&self) -> Ticket {
        let raw = self.next_ticket.get();
        self.next_ticket.set(raw + 1);
        Ticket::new(raw)
    }

    /// The jar of a tab's page; the main one for a tab the engine never saw.
    pub(crate) fn jar_of(&self, tab: TabId) -> Jar {
        self.jars.borrow().get(&tab).copied().unwrap_or(Jar::Main)
    }

    fn ask(inbox: &Weak<Self>, owner: &Owner, origin: Url, kind: AskKind, reply: Reply) {
        let Some(inbox) = inbox.upgrade() else {
            return;
        };
        let raw = inbox.next_ask.get();
        inbox.next_ask.set(raw + 1);
        let token = ReplyToken::new(raw);
        inbox
            .replies
            .borrow_mut()
            .insert(token, (Rc::clone(owner), reply));
        match owner.get() {
            Some(tab) => inbox.push(EngineEvent::Asked(Ask {
                token,
                tab,
                origin,
                kind,
            })),
            None => inbox.waiting.borrow_mut().push(Waiting {
                token,
                owner: Rc::clone(owner),
                origin,
                kind,
            }),
        }
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
        inbox.push(EngineEvent::Opened {
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

    /// The asks of pages that got their tab since they asked join the queue, oldest first;
    /// those of pages still waiting to be adopted keep waiting.
    pub(crate) fn release_asks(&self) {
        let mut released = Vec::new();
        self.waiting
            .borrow_mut()
            .retain(|waiting| match waiting.owner.get() {
                Some(tab) => {
                    released.push(EngineEvent::Asked(Ask {
                        token: waiting.token,
                        tab,
                        origin: waiting.origin.clone(),
                        kind: waiting.kind.clone(),
                    }));
                    false
                }
                None => true,
            });
        for event in released {
            self.push(event);
        }
    }

    /// Sends the answer to the page that waits on `token`; false when it no longer waits.
    pub(crate) fn answer(&self, token: ReplyToken, answer: &Answer) -> bool {
        let reply = self.replies.borrow_mut().remove(&token);
        reply.map(|(_, reply)| reply.answer(answer)).is_some()
    }

    /// The tab's page is gone: its asks are declined, its reports dropped.
    pub(crate) fn forget(&self, tab: TabId) {
        let declined: Vec<(Owner, Reply)> = {
            let mut replies = self.replies.borrow_mut();
            let tokens: Vec<ReplyToken> = replies
                .iter()
                .filter(|(_, (owner, _))| owner.get() == Some(tab))
                .map(|(&token, _)| token)
                .collect();
            tokens
                .iter()
                .filter_map(|token| replies.remove(token))
                .collect()
        };
        self.events.borrow_mut().forget(tab);
        self.jars.borrow_mut().remove(&tab);
        // Dropped outside the borrows: each reply calls into WebKit.
        drop(declined);
    }

    /// Moves every waiting event to the end of `into`, oldest first.
    pub(crate) fn drain(&self, into: &mut Vec<Event>) {
        self.events.borrow_mut().drain_into(into);
    }

    /// True when nothing waits for the shell.
    fn is_quiet(&self) -> bool {
        self.events.borrow().is_empty() && self.links.borrow().is_empty()
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

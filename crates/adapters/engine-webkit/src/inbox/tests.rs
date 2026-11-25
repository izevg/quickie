use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2::runtime::Bool;
use quickie_application::{Answer, Ask, AskKind, Dialog, EngineEvent, Fullscreen, ReplyToken};
use quickie_domain_browsing::{Focus, Profile, TabId};
use url::Url;

use super::{Event, Inbox, OpenedPage, Owner};
use crate::page::PageEvent;
use crate::page::asks::Reply;

/// Two opened pages are the same when they are the same view.
impl PartialEq for OpenedPage {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.page.web_view(), other.page.web_view())
    }
}

fn owner(tab: Option<u64>) -> Owner {
    Rc::new(Cell::new(
        tab.map(|serial| TabId::new(Profile::Main, serial)),
    ))
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn alert(message: &str) -> AskKind {
    AskKind::Dialog(Dialog::Alert {
        message: message.to_owned(),
    })
}

/// A confirm reply that writes what it heard into `heard`.
fn confirm(heard: &Rc<RefCell<Vec<bool>>>) -> Reply {
    let heard = Rc::clone(heard);
    Reply::confirm(RcBlock::new(move |ok: Bool| {
        heard.borrow_mut().push(ok.as_bool());
    }))
}

fn drained(inbox: &Inbox) -> Vec<Event> {
    let mut events = Vec::new();
    inbox.drain(&mut events);
    events
}

/// The asks among what the inbox holds now.
fn asks(inbox: &Inbox) -> Vec<Ask> {
    drained(inbox)
        .into_iter()
        .filter_map(|event| match event {
            EngineEvent::Asked(ask) => Some(ask),
            _ => None,
        })
        .collect()
}

#[test]
fn asks_wait_until_their_page_has_a_tab() {
    let inbox = Rc::new(Inbox::default());
    let heard = Rc::default();
    let popup = owner(None);
    Inbox::ask(
        &Rc::downgrade(&inbox),
        &popup,
        url("https://a.example/"),
        alert("early"),
        confirm(&heard),
    );
    Inbox::ask(
        &Rc::downgrade(&inbox),
        &owner(Some(1)),
        url("https://b.example/"),
        alert("now"),
        confirm(&heard),
    );

    let first: Vec<(TabId, AskKind)> = asks(&inbox)
        .into_iter()
        .map(|ask| (ask.tab, ask.kind))
        .collect();
    inbox.release_asks();
    let unreleased = asks(&inbox);
    popup.set(Some(TabId::new(Profile::Main, 7)));
    inbox.release_asks();
    let second: Vec<(TabId, Url)> = asks(&inbox)
        .into_iter()
        .map(|ask| (ask.tab, ask.origin))
        .collect();

    assert_eq!(first, [(TabId::new(Profile::Main, 1), alert("now"))]);
    assert_eq!(unreleased, []);
    assert_eq!(
        second,
        [(TabId::new(Profile::Main, 7), url("https://a.example/"))]
    );
    inbox.release_asks();
    assert_eq!(asks(&inbox), []);
}

#[test]
fn an_answer_reaches_its_page_once() {
    let inbox = Rc::new(Inbox::default());
    let heard = Rc::default();
    Inbox::ask(
        &Rc::downgrade(&inbox),
        &owner(Some(1)),
        url("https://a.example/"),
        alert("one"),
        confirm(&heard),
    );
    let token = asks(&inbox)[0].token;

    assert!(inbox.answer(token, &Answer::Accept(None)));
    assert!(!inbox.answer(token, &Answer::Decline));
    assert_eq!(*heard.borrow(), [true]);
}

#[test]
fn a_closed_tab_declines_its_asks_and_keeps_the_others() {
    let inbox = Rc::new(Inbox::default());
    let heard = Rc::default();
    let other = Rc::default();
    for tab in [1, 1] {
        Inbox::ask(
            &Rc::downgrade(&inbox),
            &owner(Some(tab)),
            url("https://a.example/"),
            alert("closing"),
            confirm(&heard),
        );
    }
    Inbox::ask(
        &Rc::downgrade(&inbox),
        &owner(Some(2)),
        url("https://b.example/"),
        alert("staying"),
        confirm(&other),
    );

    inbox.forget(TabId::new(Profile::Main, 1));
    let kept: Vec<ReplyToken> = asks(&inbox).into_iter().map(|ask| ask.token).collect();

    assert_eq!(*heard.borrow(), [false, false]);
    assert!(other.borrow().is_empty());
    assert_eq!(kept.len(), 1);
    assert!(inbox.answer(kept[0], &Answer::Accept(None)));
    assert_eq!(*other.borrow(), [true]);
}

#[test]
fn page_events_become_engine_events_for_their_tab_in_order() {
    let inbox = Rc::new(Inbox::default());
    let weak = Rc::downgrade(&inbox);
    let page = owner(Some(3));
    let tab = TabId::new(Profile::Main, 3);

    Inbox::record(
        &weak,
        &page,
        PageEvent::Committed {
            url: Some("https://a.example/".to_owned()),
            title: None,
        },
    );
    Inbox::record(&weak, &page, PageEvent::Fullscreen(true));
    Inbox::record(&weak, &page, PageEvent::Progress(Some(0.5)));
    Inbox::record(&weak, &page, PageEvent::TitleChanged(Some("A".to_owned())));
    Inbox::record(&weak, &page, PageEvent::Fullscreen(false));
    Inbox::record(&weak, &page, PageEvent::FullscreenInWindow(true));
    Inbox::record(&weak, &page, PageEvent::FullscreenInWindow(false));
    Inbox::record(&weak, &page, PageEvent::Crashed);
    Inbox::record(&weak, &page, PageEvent::Progress(None));
    Inbox::record(&weak, &page, PageEvent::Closed);
    Inbox::record(&weak, &owner(None), PageEvent::Closed);

    assert_eq!(
        drained(&inbox),
        [
            EngineEvent::Committed {
                tab,
                url: url("https://a.example/"),
                title: "A".to_owned()
            },
            EngineEvent::Fullscreen {
                tab,
                state: Fullscreen::OwnWindow
            },
            EngineEvent::Loading {
                tab,
                progress: None
            },
            EngineEvent::Fullscreen {
                tab,
                state: Fullscreen::Left
            },
            EngineEvent::Fullscreen {
                tab,
                state: Fullscreen::InWindow
            },
            EngineEvent::Fullscreen {
                tab,
                state: Fullscreen::Left
            },
            EngineEvent::Crashed { tab },
            EngineEvent::ClosedByPage { tab },
        ]
    );
}

#[test]
fn a_link_to_open_elsewhere_waits_for_the_engine_to_make_its_page() {
    let inbox = Rc::new(Inbox::default());

    Inbox::record(
        &Rc::downgrade(&inbox),
        &owner(Some(3)),
        PageEvent::OpenLink {
            url: "https://a.example/next".to_owned(),
            focus: Focus::Background,
        },
    );

    let links: Vec<(TabId, String, Focus)> = inbox
        .links
        .borrow()
        .iter()
        .map(|link| (link.opener, link.url.clone(), link.focus))
        .collect();
    assert_eq!(
        links,
        [(
            TabId::new(Profile::Main, 3),
            "https://a.example/next".to_owned(),
            Focus::Background
        )]
    );
    assert!(!inbox.is_quiet());
    assert_eq!(drained(&inbox), []);
}

#[test]
fn a_commit_whose_url_does_not_parse_is_dropped() {
    let inbox = Inbox::default();

    inbox.committed(TabId::new(Profile::Main, 1), Some("not a url"), None);
    inbox.committed(TabId::new(Profile::Main, 1), None, Some("A".to_owned()));

    assert!(inbox.is_quiet());
}

#[test]
fn tickets_differ() {
    let inbox = Inbox::default();

    assert_ne!(inbox.ticket(), inbox.ticket());
}

#[test]
fn picture_in_picture_events_keep_their_tab_and_order() {
    let inbox = Rc::new(Inbox::default());
    let weak = Rc::downgrade(&inbox);
    let page = owner(Some(3));
    let tab = TabId::new(Profile::Main, 3);

    Inbox::record(&weak, &page, PageEvent::PictureInPicture(true));
    Inbox::record(&weak, &page, PageEvent::PictureInPictureReturnRequested);
    Inbox::record(&weak, &page, PageEvent::PictureInPicture(false));

    assert_eq!(
        drained(&inbox),
        [
            EngineEvent::PictureInPicture { tab, active: true },
            EngineEvent::PictureInPictureReturnRequested { tab },
            EngineEvent::PictureInPicture { tab, active: false },
        ]
    );
}

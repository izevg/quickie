use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2::runtime::Bool;
use quickie_application::{Answer, AskKind, Dialog};
use quickie_domain_browsing::{Focus, Profile, TabId};
use url::Url;

use super::{Inbox, Owner};
use crate::page::PageEvent;
use crate::page::asks::Reply;

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

    let first: Vec<(TabId, AskKind)> = inbox
        .take_asks()
        .into_iter()
        .map(|ask| (ask.tab, ask.kind))
        .collect();
    popup.set(Some(TabId::new(Profile::Main, 7)));
    let second: Vec<(TabId, Url)> = inbox
        .take_asks()
        .into_iter()
        .map(|ask| (ask.tab, ask.origin))
        .collect();

    assert_eq!(first, [(TabId::new(Profile::Main, 1), alert("now"))]);
    assert_eq!(
        second,
        [(TabId::new(Profile::Main, 7), url("https://a.example/"))]
    );
    assert_eq!(inbox.take_asks(), Vec::new());
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
    let token = inbox.take_asks()[0].token;

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
    let kept = inbox.take_asks().pop().map(|ask| ask.token);

    inbox.forget(TabId::new(Profile::Main, 1));

    assert_eq!(*heard.borrow(), [false, false]);
    assert!(other.borrow().is_empty());
    assert!(kept.is_some_and(|token| inbox.answer(token, &Answer::Accept(None))));
    assert_eq!(*other.borrow(), [true]);
}

#[test]
fn page_events_land_in_their_queues_for_their_tab() {
    let inbox = Rc::new(Inbox::default());
    let weak = Rc::downgrade(&inbox);
    let page = owner(Some(3));

    Inbox::record(
        &weak,
        &page,
        PageEvent::OpenLink {
            url: "https://a.example/next".to_owned(),
            focus: Focus::Background,
        },
    );
    Inbox::record(&weak, &page, PageEvent::Fullscreen(true));
    Inbox::record(&weak, &page, PageEvent::Fullscreen(false));
    Inbox::record(&weak, &page, PageEvent::Closed);
    Inbox::record(&weak, &owner(None), PageEvent::Closed);

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
    assert_eq!(
        *inbox.fullscreen.borrow(),
        [
            (TabId::new(Profile::Main, 3), true),
            (TabId::new(Profile::Main, 3), false)
        ]
    );
    assert_eq!(*inbox.closed.borrow(), [TabId::new(Profile::Main, 3)]);
}

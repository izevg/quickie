//! Tests of the bridge's Rust side: event conversion and delivery. Nothing here calls into C++.

use std::cell::RefCell;
use std::rc::Rc;

use super::{Event, KeyEquivalent, dispatch};
use crate::ffi;
use crate::ffi::bridge::{EngineEvent, EngineEventKind};

fn bridge_event(kind: EngineEventKind, tab: u64, opener: Option<u64>, text: &str) -> EngineEvent {
    EngineEvent {
        kind,
        tab,
        has_opener: opener.is_some(),
        opener: opener.unwrap_or(0),
        text: text.to_owned(),
    }
}

fn install(
    events: impl FnMut(Event) + 'static,
    keys: impl FnMut(KeyEquivalent) -> bool + 'static,
    wake: impl FnMut() + 'static,
) {
    dispatch::install(Box::new(events), Box::new(keys), Box::new(wake));
}

fn recording() -> Rc<RefCell<Vec<Event>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    install(move |event| sink.borrow_mut().push(event), |_| false, || {});
    seen
}

fn title(tab: u64, title: &str) -> Event {
    Event::TitleChanged {
        tab,
        title: title.to_owned(),
    }
}

#[test]
fn reads_navigation_and_title_events() {
    let committed = bridge_event(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );
    let titled = bridge_event(EngineEventKind::TitleChanged, 7, None, "A");

    let read = [Event::from_bridge(&committed), Event::from_bridge(&titled)];

    assert_eq!(
        read,
        [
            Event::NavigationCommitted {
                tab: 7,
                url: "https://a.test/".to_owned()
            },
            title(7, "A")
        ]
    );
}

#[test]
fn reads_opened_pages_with_and_without_an_opener() {
    let with = bridge_event(
        EngineEventKind::PageOpenedTab,
        90,
        Some(0),
        "https://b.test/",
    );
    let without = bridge_event(EngineEventKind::PageOpenedTab, 91, None, "");

    let read = [Event::from_bridge(&with), Event::from_bridge(&without)];

    assert_eq!(
        read,
        [
            Event::PageOpenedTab {
                page: 90,
                opener: Some(0),
                url: "https://b.test/".to_owned()
            },
            Event::PageOpenedTab {
                page: 91,
                opener: None,
                url: String::new()
            }
        ]
    );
}

#[test]
fn reports_an_unknown_kind_from_a_newer_engine() {
    let newer = bridge_event(EngineEventKind { repr: 200 }, 1, None, "");

    let read = Event::from_bridge(&newer);

    assert_eq!(read, Event::Unknown);
}

#[test]
fn delivers_events_in_order_to_the_installed_handler() {
    let seen = recording();

    dispatch::deliver_event(Event::Unknown);
    dispatch::deliver_event(title(1, "T"));

    assert_eq!(*seen.borrow(), [Event::Unknown, title(1, "T")]);
}

#[test]
fn queues_an_event_raised_while_a_handler_runs() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    install(
        move |event: Event| {
            if let Event::TitleChanged { tab, .. } = event {
                // The engine reports synchronously from a call the handler makes.
                dispatch::deliver_event(Event::NavigationCommitted {
                    tab,
                    url: "u".to_owned(),
                });
            }
            sink.borrow_mut().push(event);
        },
        |_| false,
        || {},
    );

    dispatch::deliver_event(title(3, "T"));

    assert_eq!(
        *seen.borrow(),
        [
            title(3, "T"),
            Event::NavigationCommitted {
                tab: 3,
                url: "u".to_owned()
            }
        ]
    );
}

#[test]
fn drops_events_after_the_handlers_are_removed() {
    let seen = recording();
    dispatch::uninstall();

    dispatch::deliver_event(Event::Unknown);

    assert!(seen.borrow().is_empty());
}

#[test]
fn an_event_handler_that_replaces_itself_is_not_put_back() {
    let second = Rc::new(RefCell::new(0));
    let count = Rc::clone(&second);
    install(
        move |_| {
            let count = Rc::clone(&count);
            install(move |_| *count.borrow_mut() += 1, |_| false, || {});
        },
        |_| false,
        || {},
    );

    dispatch::deliver_event(Event::Unknown);
    dispatch::deliver_event(Event::Unknown);

    assert_eq!(*second.borrow(), 1);
}

#[test]
fn a_handler_installed_after_removing_the_running_one_wins() {
    let second = Rc::new(RefCell::new(0));
    let count = Rc::clone(&second);
    install(
        move |_| {
            let count = Rc::clone(&count);
            dispatch::uninstall();
            install(move |_| *count.borrow_mut() += 1, |_| false, || {});
        },
        |_| false,
        || {},
    );

    dispatch::deliver_event(Event::Unknown);
    dispatch::deliver_event(Event::Unknown);

    assert_eq!(*second.borrow(), 1);
}

#[test]
fn an_event_handler_that_removes_itself_drops_what_it_raised() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    install(
        move |event| {
            sink.borrow_mut().push(event);
            dispatch::deliver_event(title(1, "late"));
            dispatch::uninstall();
        },
        |_| false,
        || {},
    );

    dispatch::deliver_event(Event::Unknown);
    dispatch::deliver_event(Event::Unknown);

    assert_eq!(*seen.borrow(), [Event::Unknown]);
}

#[test]
fn keys_go_to_the_handler_and_to_the_page_without_one() {
    let key = KeyEquivalent {
        key_code: 0x11,
        modifiers: 1 << 20,
        character: Some('t'),
        page_declined: false,
    };
    install(|_| {}, move |offered| offered == key, || {});

    let shell = dispatch::deliver_key(key);
    let page = dispatch::deliver_key(KeyEquivalent {
        character: Some('y'),
        ..key
    });
    dispatch::uninstall();
    let unhandled = dispatch::deliver_key(key);

    assert_eq!((shell, page, unhandled), (true, false, false));
}

#[test]
fn a_key_handler_stays_installed_after_it_runs() {
    let key = KeyEquivalent {
        key_code: 0x11,
        modifiers: 0,
        character: None,
        page_declined: true,
    };
    install(|_| {}, |_| true, || {});
    dispatch::deliver_key(key);

    let again = dispatch::deliver_key(key);

    assert!(again);
}

#[test]
fn a_key_handler_that_replaces_itself_is_not_put_back() {
    let key = KeyEquivalent {
        key_code: 0x11,
        modifiers: 0,
        character: None,
        page_declined: false,
    };
    install(
        |_| {},
        |_| {
            install(|_| {}, |_| false, || {});
            true
        },
        || {},
    );
    dispatch::deliver_key(key);

    let again = dispatch::deliver_key(key);

    assert!(!again);
}

#[test]
fn a_wake_runs_the_wake_handler_until_it_is_removed() {
    let wakes = Rc::new(RefCell::new(0));
    let count = Rc::clone(&wakes);
    install(|_| {}, |_| false, move || *count.borrow_mut() += 1);

    ffi::run_wake();
    ffi::run_wake();
    dispatch::uninstall();
    ffi::run_wake();

    assert_eq!(*wakes.borrow(), 2);
}

#[test]
fn a_wake_raised_while_the_wake_handler_runs_is_dropped() {
    let wakes = Rc::new(RefCell::new(0));
    let count = Rc::clone(&wakes);
    install(
        |_| {},
        |_| false,
        move || {
            *count.borrow_mut() += 1;
            dispatch::deliver_wake();
        },
    );

    ffi::run_wake();

    assert_eq!(*wakes.borrow(), 1);
}

#[test]
fn the_event_entry_point_reaches_the_handler() {
    let seen = recording();

    ffi::on_engine_event(&bridge_event(EngineEventKind::TitleChanged, 2, None, "x"));

    assert_eq!(*seen.borrow(), [title(2, "x")]);
}

#[test]
fn the_key_entry_point_decodes_the_character() {
    let offered = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&offered);
    install(
        |_| {},
        move |key| {
            sink.borrow_mut().push(key);
            true
        },
        || {},
    );

    let consumed = [
        ffi::handle_key_equivalent(0x0d, 1 << 20, u32::from('w'), false),
        ffi::handle_key_equivalent(0x7a, 0, 0xD800, true),
    ];

    assert_eq!(consumed, [true, true]);
    assert_eq!(
        *offered.borrow(),
        [
            KeyEquivalent {
                key_code: 0x0d,
                modifiers: 1 << 20,
                character: Some('w'),
                page_declined: false
            },
            KeyEquivalent {
                key_code: 0x7a,
                modifiers: 0,
                character: None,
                page_declined: true
            }
        ]
    );
}

#[cfg(not(feature = "chromium"))]
#[test]
fn without_chromium_every_call_is_unavailable() {
    use super::{Bridge, BridgeError, Ffi, NavigationKind};

    let view = std::ptr::NonNull::<u8>::dangling().cast();

    // SAFETY: without the `chromium` feature the pointer is never passed on or dereferenced.
    let shown = unsafe { Ffi.show_tab(1, view, 1) };
    let results = [
        Ffi.create_tab(1, "https://a.test/", true, 4),
        shown,
        Ffi.navigate_tab(1, NavigationKind::Reload, ""),
        Ffi.close_tab(1),
        Ffi.adopt_tab(9, 2),
        Ffi.destroy_jar(4),
    ];

    assert_eq!(results, [Err(BridgeError::Unavailable); 6]);
}

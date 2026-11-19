//! Tests of the Rust side only: event conversion and delivery. Nothing here calls into C++.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ffi::bridge::{EngineEvent, EngineEventKind, NavigationKind};
use crate::{Engine, Event, KeyEquivalent, Navigation, dispatch};

fn bridge_event(kind: EngineEventKind, tab: u64, opener: Option<u64>, text: &str) -> EngineEvent {
    EngineEvent {
        kind,
        tab,
        has_opener: opener.is_some(),
        opener: opener.unwrap_or(0),
        text: text.to_owned(),
    }
}

fn recording_engine() -> (Engine, Rc<RefCell<Vec<Event>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    let engine = Engine::new(move |event| sink.borrow_mut().push(event), |_| false);
    (engine, seen)
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

    assert_eq!(
        Event::from_bridge(&committed),
        Event::NavigationCommitted {
            tab: 7,
            url: "https://a.test/".to_owned()
        }
    );
    assert_eq!(
        Event::from_bridge(&titled),
        Event::TitleChanged {
            tab: 7,
            title: "A".to_owned()
        }
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

    assert_eq!(
        Event::from_bridge(&with),
        Event::PageOpenedTab {
            page: 90,
            opener: Some(0),
            url: "https://b.test/".to_owned()
        }
    );
    assert_eq!(
        Event::from_bridge(&without),
        Event::PageOpenedTab {
            page: 91,
            opener: None,
            url: String::new()
        }
    );
}

#[test]
fn reports_an_unknown_kind_from_a_newer_engine() {
    let newer = bridge_event(EngineEventKind { repr: 200 }, 1, None, "");

    assert_eq!(Event::from_bridge(&newer), Event::Unknown);
}

#[test]
fn maps_navigations_to_the_bridge() {
    let cases = [
        (
            Navigation::Load("https://c.test/"),
            NavigationKind::Load,
            "https://c.test/",
        ),
        (Navigation::Back, NavigationKind::Back, ""),
        (Navigation::Forward, NavigationKind::Forward, ""),
        (Navigation::Reload, NavigationKind::Reload, ""),
        (Navigation::Stop, NavigationKind::Stop, ""),
    ];

    for (navigation, kind, url) in cases {
        assert_eq!(navigation.to_bridge(), (kind, url), "{navigation:?}");
    }
}

#[test]
fn delivers_events_in_order_to_the_installed_handler() {
    let (_engine, seen) = recording_engine();

    dispatch::deliver_event(Event::Unknown);
    dispatch::deliver_event(Event::TitleChanged {
        tab: 1,
        title: "T".to_owned(),
    });

    assert_eq!(
        *seen.borrow(),
        [
            Event::Unknown,
            Event::TitleChanged {
                tab: 1,
                title: "T".to_owned()
            }
        ]
    );
}

#[test]
fn queues_an_event_raised_while_a_handler_runs() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    let _engine = Engine::new(
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
    );

    dispatch::deliver_event(Event::TitleChanged {
        tab: 3,
        title: "T".to_owned(),
    });

    assert_eq!(
        *seen.borrow(),
        [
            Event::TitleChanged {
                tab: 3,
                title: "T".to_owned()
            },
            Event::NavigationCommitted {
                tab: 3,
                url: "u".to_owned()
            }
        ]
    );
}

#[test]
fn drops_events_after_the_engine_is_dropped() {
    let (engine, seen) = recording_engine();
    drop(engine);

    dispatch::deliver_event(Event::Unknown);

    assert!(seen.borrow().is_empty());
}

#[test]
fn a_handler_that_replaces_itself_is_not_put_back() {
    let second = Rc::new(RefCell::new(0));
    let count = Rc::clone(&second);
    let _first = Engine::new(
        move |_| {
            let count = Rc::clone(&count);
            dispatch::install(
                Box::new(move |_| *count.borrow_mut() += 1),
                Box::new(|_| false),
            );
        },
        |_| false,
    );

    dispatch::deliver_event(Event::Unknown);
    dispatch::deliver_event(Event::Unknown);

    assert_eq!(*second.borrow(), 1);
}

#[test]
fn keys_go_to_the_handler_and_to_the_page_without_one() {
    let key = KeyEquivalent {
        key_code: 0x11,
        modifiers: 1 << 20,
        character: Some('t'),
        page_declined: false,
    };
    let engine = Engine::new(|_| {}, move |offered| offered == key);

    assert!(dispatch::deliver_key(key));
    assert!(!dispatch::deliver_key(KeyEquivalent {
        character: Some('y'),
        ..key
    }));

    drop(engine);
    assert!(!dispatch::deliver_key(key));
}

#[test]
fn bridge_entry_points_reach_the_handlers() {
    let (_engine, seen) = recording_engine();

    crate::ffi::on_engine_event(&bridge_event(EngineEventKind::TitleChanged, 2, None, "x"));

    assert_eq!(
        *seen.borrow(),
        [Event::TitleChanged {
            tab: 2,
            title: "x".to_owned()
        }]
    );
}

#[test]
fn the_key_entry_point_decodes_the_character() {
    let offered = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&offered);
    let _engine = Engine::new(
        |_| {},
        move |key| {
            sink.borrow_mut().push(key);
            true
        },
    );

    assert!(crate::ffi::handle_key_equivalent(
        0x0d,
        1 << 20,
        u32::from('w'),
        false
    ));
    assert!(crate::ffi::handle_key_equivalent(0x7a, 0, 0xD800, true));

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
    use crate::{EngineError, Jar};

    let engine = Engine::new(|_| {}, |_| false);
    let view = std::ptr::NonNull::<u8>::dangling().cast();

    assert_eq!(
        engine.create_tab(1, "https://a.test/", Jar::Throwaway(4)),
        Err(EngineError::Unavailable)
    );
    // SAFETY: without the `chromium` feature the pointer is never passed on or dereferenced.
    let shown = unsafe { engine.show_tab(1, view, 1) };
    assert_eq!(shown, Err(EngineError::Unavailable));
    assert_eq!(
        engine.navigate(1, Navigation::Reload),
        Err(EngineError::Unavailable)
    );
    assert_eq!(engine.close_tab(1), Err(EngineError::Unavailable));
    assert_eq!(engine.adopt_tab(9, 2), Err(EngineError::Unavailable));
    assert_eq!(engine.destroy_jar(4), Err(EngineError::Unavailable));
}

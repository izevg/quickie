//! `ChromiumEngine` against a fake bridge: id mapping, deferred shows, jar bookkeeping, event
//! translation and error text. The C++ side's reports are simulated through the real entry
//! points (`ffi::on_engine_event`, `ffi::run_wake`).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use quickie_application::{EngineError, EngineEvent, EnginePort, Navigation};
use quickie_domain_browsing::{Focus, Jar, JarId, Profile, TabId, WindowId};
use url::Url;

use super::{ChromiumEngine, PageHandle};
use crate::bridge::{Bridge, BridgeError, KeyEquivalent, NavigationKind};
use crate::ffi;
use crate::ffi::bridge::{EngineEvent as BridgeEvent, EngineEventKind};

/// A bridge call, as the C++ side would have received it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Create {
        tab: u64,
        url: String,
        throwaway: bool,
        jar: u64,
    },
    Show {
        window: u64,
        host: usize,
        tab: u64,
    },
    Navigate {
        tab: u64,
        kind: NavigationKind,
        url: String,
    },
    Close(u64),
    Adopt {
        page: u64,
        tab: u64,
    },
    DestroyJar(u64),
}

#[derive(Default)]
struct FakeBridge {
    calls: RefCell<Vec<Call>>,
    failure: Cell<Option<BridgeError>>,
    wakes: Cell<usize>,
}

impl FakeBridge {
    fn answer(&self, call: Call) -> Result<(), BridgeError> {
        self.calls.borrow_mut().push(call);
        self.failure.get().map_or(Ok(()), Err)
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.take()
    }
}

impl Bridge for FakeBridge {
    fn create_tab(
        &self,
        tab: u64,
        url: &str,
        throwaway: bool,
        jar: u64,
    ) -> Result<(), BridgeError> {
        self.answer(Call::Create {
            tab,
            url: url.to_owned(),
            throwaway,
            jar,
        })
    }

    unsafe fn show_tab(
        &self,
        window: u64,
        host_view: NonNull<c_void>,
        tab: u64,
    ) -> Result<(), BridgeError> {
        self.answer(Call::Show {
            window,
            host: host_view.as_ptr() as usize,
            tab,
        })
    }

    fn navigate_tab(&self, tab: u64, kind: NavigationKind, url: &str) -> Result<(), BridgeError> {
        self.answer(Call::Navigate {
            tab,
            kind,
            url: url.to_owned(),
        })
    }

    fn close_tab(&self, tab: u64) -> Result<(), BridgeError> {
        self.answer(Call::Close(tab))
    }

    fn adopt_tab(&self, page: u64, tab: u64) -> Result<(), BridgeError> {
        self.answer(Call::Adopt { page, tab })
    }

    fn destroy_jar(&self, jar: u64) -> Result<(), BridgeError> {
        self.answer(Call::DestroyJar(jar))
    }

    fn schedule_wake(&self) {
        self.wakes.set(self.wakes.get() + 1);
    }
}

fn engine() -> (ChromiumEngine, Rc<FakeBridge>) {
    let fake = Rc::new(FakeBridge::default());
    let engine = ChromiumEngine::with_bridge(Rc::clone(&fake) as Rc<dyn Bridge>, |_| false);
    (engine, fake)
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn tab(raw: u64) -> TabId {
    TabId::new(Profile::Main, raw)
}

fn window(raw: u64) -> WindowId {
    WindowId::new(Profile::Main, raw)
}

fn host(address: usize) -> NonNull<c_void> {
    NonNull::new(address as *mut c_void).unwrap()
}

fn attach(engine: &mut ChromiumEngine, raw: u64, address: usize) -> Result<(), EngineError> {
    // SAFETY: the fake bridge only records the address; nothing dereferences it.
    unsafe { engine.attach_host(window(raw), host(address)) }
}

/// An engine with a main-jar page for each of `tabs`, and the calls that made them forgotten.
fn with_pages(tabs: &[u64]) -> (ChromiumEngine, Rc<FakeBridge>) {
    let (mut engine, fake) = engine();
    for &raw in tabs {
        engine
            .create(tab(raw), &url("https://a.test/"), Jar::Main)
            .unwrap();
    }
    fake.calls();
    (engine, fake)
}

fn report(kind: EngineEventKind, raw: u64, opener: Option<u64>, text: &str) {
    ffi::on_engine_event(&BridgeEvent {
        kind,
        tab: raw,
        has_opener: opener.is_some(),
        opener: opener.unwrap_or(0),
        text: text.to_owned(),
    });
}

type Event = EngineEvent<PageHandle>;

/// What the engine holds for the shell now.
fn drained(engine: &mut ChromiumEngine) -> Vec<Event> {
    let mut events = Vec::new();
    engine.drain(&mut events);
    events
}

fn committed(raw: u64, at: &str, title: &str) -> Event {
    EngineEvent::Committed {
        tab: tab(raw),
        url: url(at),
        title: title.to_owned(),
    }
}

fn rejected(text: &str) -> Result<(), EngineError> {
    Err(EngineError(format!(
        "{text}: Chromium rejected it (the reason is in Chromium's log)"
    )))
}

// ---- create --------------------------------------------------------------------------------

#[test]
fn a_main_jar_page_is_created_under_the_raw_tab_id() {
    let (mut engine, fake) = engine();

    let created = engine.create(tab(7), &url("https://a.test/x"), Jar::Main);

    assert_eq!(created, Ok(()));
    assert_eq!(
        fake.calls(),
        [Call::Create {
            tab: 7,
            url: "https://a.test/x".to_owned(),
            throwaway: false,
            jar: 0
        }]
    );
}

#[test]
fn a_throwaway_page_is_created_in_its_jar() {
    let (mut engine, fake) = engine();

    let created = engine.create(
        tab(7),
        &url("https://a.test/"),
        Jar::Throwaway(JarId::new(4)),
    );

    assert_eq!(created, Ok(()));
    assert_eq!(
        fake.calls(),
        [Call::Create {
            tab: 7,
            url: "https://a.test/".to_owned(),
            throwaway: true,
            jar: 4
        }]
    );
}

#[test]
fn a_tab_gets_one_page_only() {
    let (mut engine, fake) = with_pages(&[7]);

    let again = engine.create(tab(7), &url("https://b.test/"), Jar::Main);

    assert_eq!(
        again,
        Err(EngineError("main tab#7 already has a page".to_owned()))
    );
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_page_chromium_refused_does_not_exist() {
    let (mut engine, fake) = engine();
    fake.failure.set(Some(BridgeError::Rejected));

    let created = engine.create(tab(7), &url("https://a.test/"), Jar::Main);
    let shown = engine.show(window(1), tab(7));

    assert_eq!(created, rejected("create the page for main tab#7"));
    assert_eq!(shown, Err(EngineError("no page for main tab#7".to_owned())));
}

// ---- show ----------------------------------------------------------------------------------

#[test]
fn a_tab_shows_in_an_attached_window_by_raw_ids() {
    let (mut engine, fake) = with_pages(&[7]);
    attach(&mut engine, 2, 0x100).unwrap();

    let shown = engine.show(window(2), tab(7));

    assert_eq!(shown, Ok(()));
    assert_eq!(
        fake.calls(),
        [Call::Show {
            window: 2,
            host: 0x100,
            tab: 7
        }]
    );
}

#[test]
fn a_show_before_the_window_is_attached_waits_for_its_host() {
    let (mut engine, fake) = with_pages(&[7]);

    let shown = engine.show(window(2), tab(7));
    let before = fake.calls();
    let attached = attach(&mut engine, 2, 0x100);

    assert_eq!((shown, attached), (Ok(()), Ok(())));
    assert_eq!(before, []);
    assert_eq!(
        fake.calls(),
        [Call::Show {
            window: 2,
            host: 0x100,
            tab: 7
        }]
    );
}

#[test]
fn only_the_last_tab_shown_before_attaching_is_shown() {
    let (mut engine, fake) = with_pages(&[7, 8]);
    engine.show(window(2), tab(7)).unwrap();
    engine.show(window(2), tab(8)).unwrap();

    attach(&mut engine, 2, 0x100).unwrap();

    assert_eq!(
        fake.calls(),
        [Call::Show {
            window: 2,
            host: 0x100,
            tab: 8
        }]
    );
}

#[test]
fn a_waiting_show_runs_once() {
    let (mut engine, fake) = with_pages(&[7]);
    engine.show(window(2), tab(7)).unwrap();
    attach(&mut engine, 2, 0x100).unwrap();
    fake.calls();

    let again = attach(&mut engine, 2, 0x100);

    assert_eq!(again, Ok(()));
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_tab_shown_elsewhere_no_longer_waits_for_its_first_window() {
    let (mut engine, fake) = with_pages(&[7]);
    attach(&mut engine, 3, 0x300).unwrap();
    engine.show(window(2), tab(7)).unwrap();
    engine.show(window(3), tab(7)).unwrap();
    fake.calls();

    attach(&mut engine, 2, 0x200).unwrap();

    assert_eq!(fake.calls(), []);
}

#[test]
fn another_tab_waiting_elsewhere_keeps_waiting() {
    let (mut engine, fake) = with_pages(&[7, 8]);
    attach(&mut engine, 3, 0x300).unwrap();
    engine.show(window(2), tab(7)).unwrap();
    engine.show(window(3), tab(8)).unwrap();
    fake.calls();

    attach(&mut engine, 2, 0x200).unwrap();

    assert_eq!(
        fake.calls(),
        [Call::Show {
            window: 2,
            host: 0x200,
            tab: 7
        }]
    );
}

#[test]
fn a_window_keeps_its_first_host_view() {
    let (mut engine, fake) = with_pages(&[7]);
    attach(&mut engine, 2, 0x100).unwrap();

    let other = attach(&mut engine, 2, 0x200);
    engine.show(window(2), tab(7)).unwrap();

    assert_eq!(
        other,
        Err(EngineError(
            "main window#2 already has another host view".to_owned()
        ))
    );
    assert_eq!(
        fake.calls(),
        [Call::Show {
            window: 2,
            host: 0x100,
            tab: 7
        }]
    );
}

#[test]
fn showing_a_tab_without_a_page_fails_without_calling_chromium() {
    let (mut engine, fake) = engine();
    attach(&mut engine, 2, 0x100).unwrap();

    let shown = engine.show(window(2), tab(7));

    assert_eq!(shown, Err(EngineError("no page for main tab#7".to_owned())));
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_failed_show_names_the_tab_and_the_window() {
    let (mut engine, fake) = with_pages(&[7]);
    attach(&mut engine, 2, 0x100).unwrap();
    fake.failure.set(Some(BridgeError::Rejected));

    let shown = engine.show(window(2), tab(7));

    assert_eq!(shown, rejected("show main tab#7 in main window#2"));
}

#[test]
fn a_failed_waiting_show_is_reported_by_attaching() {
    let (mut engine, fake) = with_pages(&[7]);
    engine.show(window(2), tab(7)).unwrap();
    fake.failure.set(Some(BridgeError::Rejected));

    let attached = attach(&mut engine, 2, 0x100);

    assert_eq!(attached, rejected("show main tab#7 in main window#2"));
}

// ---- navigate ------------------------------------------------------------------------------

#[test]
fn navigations_map_to_the_bridge() {
    let (mut engine, fake) = with_pages(&[7]);
    let navigations = [
        Navigation::Load(url("https://c.test/")),
        Navigation::Back,
        Navigation::Forward,
        Navigation::Reload,
        Navigation::HardReload,
        Navigation::Stop,
    ];

    let results: Vec<_> = navigations
        .iter()
        .map(|navigation| engine.navigate(tab(7), navigation))
        .collect();

    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert_eq!(results.len(), 6);
    let navigate = |kind, url: &str| Call::Navigate {
        tab: 7,
        kind,
        url: url.to_owned(),
    };
    assert_eq!(
        fake.calls(),
        [
            navigate(NavigationKind::Load, "https://c.test/"),
            navigate(NavigationKind::Back, ""),
            navigate(NavigationKind::Forward, ""),
            navigate(NavigationKind::Reload, ""),
            navigate(NavigationKind::HardReload, ""),
            navigate(NavigationKind::Stop, ""),
        ]
    );
}

#[test]
fn a_failed_navigation_says_what_was_asked() {
    let (mut engine, fake) = with_pages(&[7]);
    fake.failure.set(Some(BridgeError::Rejected));
    let navigations = [
        Navigation::Load(url("https://c.test/")),
        Navigation::Back,
        Navigation::Forward,
        Navigation::Reload,
        Navigation::HardReload,
        Navigation::Stop,
    ];

    let results: Vec<_> = navigations
        .iter()
        .map(|navigation| engine.navigate(tab(7), navigation))
        .collect();

    assert_eq!(
        results,
        [
            rejected("load https://c.test/ in main tab#7"),
            rejected("go back in main tab#7"),
            rejected("go forward in main tab#7"),
            rejected("reload in main tab#7"),
            rejected("reload past the cache in main tab#7"),
            rejected("stop loading in main tab#7"),
        ]
    );
}

#[test]
fn navigating_a_tab_without_a_page_fails_without_calling_chromium() {
    let (mut engine, fake) = engine();

    let navigated = engine.navigate(tab(7), &Navigation::Reload);

    assert_eq!(
        navigated,
        Err(EngineError("no page for main tab#7".to_owned()))
    );
    assert_eq!(fake.calls(), []);
}

// ---- close ---------------------------------------------------------------------------------

#[test]
fn closing_destroys_the_page_by_its_raw_id() {
    let (mut engine, fake) = with_pages(&[7]);

    let closed = engine.close(tab(7));

    assert_eq!(closed, Ok(()));
    assert_eq!(fake.calls(), [Call::Close(7)]);
}

#[test]
fn a_closed_tab_has_no_page_even_when_chromium_failed() {
    let (mut engine, fake) = with_pages(&[7]);
    fake.failure.set(Some(BridgeError::Rejected));

    let closed = engine.close(tab(7));
    let again = engine.close(tab(7));

    assert_eq!(closed, rejected("close the page of main tab#7"));
    assert_eq!(again, Err(EngineError("no page for main tab#7".to_owned())));
    assert_eq!(fake.calls(), [Call::Close(7)]);
}

#[test]
fn a_closed_tab_no_longer_waits_to_be_shown() {
    let (mut engine, fake) = with_pages(&[7]);
    engine.show(window(2), tab(7)).unwrap();
    engine.close(tab(7)).unwrap();
    fake.calls();

    attach(&mut engine, 2, 0x100).unwrap();

    assert_eq!(fake.calls(), []);
}

#[test]
fn closing_keeps_other_tabs_waiting() {
    let (mut engine, fake) = with_pages(&[7, 8]);
    engine.show(window(2), tab(7)).unwrap();
    engine.close(tab(8)).unwrap();
    fake.calls();

    attach(&mut engine, 2, 0x100).unwrap();

    assert_eq!(
        fake.calls(),
        [Call::Show {
            window: 2,
            host: 0x100,
            tab: 7
        }]
    );
}

#[test]
fn a_closed_tab_reports_nothing_more() {
    let (mut engine, _fake) = with_pages(&[7]);
    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );

    engine.close(tab(7)).unwrap();

    assert_eq!(drained(&mut engine), []);
}

#[test]
fn closing_a_tab_keeps_what_other_tabs_reported() {
    let (mut engine, _fake) = with_pages(&[7, 8]);
    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );
    report(
        EngineEventKind::NavigationCommitted,
        8,
        None,
        "https://b.test/",
    );

    engine.close(tab(7)).unwrap();

    assert_eq!(drained(&mut engine), [committed(8, "https://b.test/", "")]);
}

// ---- destroy_jar ---------------------------------------------------------------------------

#[test]
fn a_jar_whose_tabs_never_got_a_page_has_nothing_to_destroy() {
    let (mut engine, fake) = engine();

    let destroyed = engine.destroy_jar(JarId::new(4));

    assert_eq!(destroyed, Ok(()));
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_jar_with_a_profile_is_destroyed_once() {
    let (mut engine, fake) = engine();
    let jar = JarId::new(4);
    engine
        .create(tab(7), &url("https://a.test/"), Jar::Throwaway(jar))
        .unwrap();
    engine.close(tab(7)).unwrap();
    fake.calls();

    let first = engine.destroy_jar(jar);
    let second = engine.destroy_jar(jar);

    assert_eq!((first, second), (Ok(()), Ok(())));
    assert_eq!(fake.calls(), [Call::DestroyJar(4)]);
}

#[test]
fn a_jar_chromium_kept_can_be_destroyed_again() {
    let (mut engine, fake) = engine();
    let jar = JarId::new(4);
    engine
        .create(tab(7), &url("https://a.test/"), Jar::Throwaway(jar))
        .unwrap();
    fake.calls();
    fake.failure.set(Some(BridgeError::Rejected));
    let first = engine.destroy_jar(jar);
    fake.failure.set(None);

    let second = engine.destroy_jar(jar);

    assert_eq!(
        first,
        rejected("destroy throwaway jar 4 (a page may still use it)")
    );
    assert_eq!(second, Ok(()));
    assert_eq!(fake.calls(), [Call::DestroyJar(4), Call::DestroyJar(4)]);
}

#[test]
fn a_main_jar_page_gives_no_throwaway_jar_a_profile() {
    let (mut engine, fake) = with_pages(&[7]);

    let destroyed = engine.destroy_jar(JarId::new(0));

    assert_eq!(destroyed, Ok(()));
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_refused_throwaway_page_gives_its_jar_no_profile() {
    let (mut engine, fake) = engine();
    let jar = JarId::new(4);
    fake.failure.set(Some(BridgeError::Rejected));
    let _ = engine.create(tab(7), &url("https://a.test/"), Jar::Throwaway(jar));
    fake.failure.set(None);
    fake.calls();

    let destroyed = engine.destroy_jar(jar);

    assert_eq!(destroyed, Ok(()));
    assert_eq!(fake.calls(), []);
}

// ---- events --------------------------------------------------------------------------------

#[test]
fn a_commit_and_a_title_fold_into_one_visit() {
    let (mut engine, _fake) = with_pages(&[7]);

    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );
    report(EngineEventKind::TitleChanged, 7, None, "A");

    assert_eq!(drained(&mut engine), [committed(7, "https://a.test/", "A")]);
    assert_eq!(drained(&mut engine), []);
}

#[test]
fn a_later_commit_keeps_the_title_until_the_page_sets_one() {
    let (mut engine, _fake) = with_pages(&[7]);
    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );
    report(EngineEventKind::TitleChanged, 7, None, "A");
    drained(&mut engine);

    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/#part",
    );

    assert_eq!(
        drained(&mut engine),
        [committed(7, "https://a.test/#part", "A")]
    );
}

#[test]
fn a_title_after_the_report_of_its_commit_renames_the_visit() {
    let (mut engine, _fake) = with_pages(&[7]);
    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );
    drained(&mut engine);

    report(EngineEventKind::TitleChanged, 7, None, "Loaded");

    assert_eq!(
        drained(&mut engine),
        [EngineEvent::Titled {
            tab: tab(7),
            url: url("https://a.test/"),
            title: "Loaded".to_owned(),
        }]
    );
}

#[test]
fn a_commit_whose_url_does_not_parse_is_dropped() {
    let (mut engine, _fake) = with_pages(&[7]);

    report(EngineEventKind::NavigationCommitted, 7, None, "not a url");

    assert_eq!(drained(&mut engine), []);
}

#[test]
fn opened_pages_come_in_order_with_their_opener_in_front() {
    let (mut engine, _fake) = with_pages(&[7]);

    report(
        EngineEventKind::PageOpenedTab,
        90,
        Some(7),
        "https://b.test/",
    );
    report(EngineEventKind::PageOpenedTab, 91, None, "https://c.test/");
    report(EngineEventKind::PageOpenedTab, 92, Some(7), "");

    assert_eq!(
        drained(&mut engine),
        [
            EngineEvent::Opened {
                opener: tab(7),
                page: PageHandle(90),
                url: url("https://b.test/"),
                focus: Focus::Foreground,
            },
            EngineEvent::Opened {
                opener: tab(7),
                page: PageHandle(92),
                url: url("about:blank"),
                focus: Focus::Foreground,
            }
        ]
    );
    assert_eq!(drained(&mut engine), []);
}

#[test]
fn an_unknown_event_is_dropped_without_a_wake() {
    let (mut engine, fake) = engine();
    engine.set_waker(|| {});

    report(EngineEventKind { repr: 200 }, 7, None, "x");

    assert_eq!(fake.wakes.get(), 0);
    assert_eq!(drained(&mut engine), []);
}

// ---- adopt ---------------------------------------------------------------------------------

#[test]
fn an_adopted_page_becomes_the_tabs_page() {
    let (mut engine, fake) = engine();

    let adopted = engine.adopt(PageHandle(90), tab(12));
    let navigated = engine.navigate(tab(12), &Navigation::Reload);

    assert_eq!((adopted, navigated), (Ok(()), Ok(())));
    assert_eq!(
        fake.calls(),
        [
            Call::Adopt { page: 90, tab: 12 },
            Call::Navigate {
                tab: 12,
                kind: NavigationKind::Reload,
                url: String::new()
            }
        ]
    );
}

#[test]
fn a_page_is_not_adopted_by_a_tab_that_has_one() {
    let (mut engine, fake) = with_pages(&[12]);

    let adopted = engine.adopt(PageHandle(90), tab(12));

    assert_eq!(
        adopted,
        Err(EngineError("main tab#12 already has a page".to_owned()))
    );
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_failed_adoption_leaves_the_tab_without_a_page() {
    let (mut engine, fake) = engine();
    fake.failure.set(Some(BridgeError::Rejected));

    let adopted = engine.adopt(PageHandle(90), tab(12));
    let closed = engine.close(tab(12));

    assert_eq!(adopted, rejected("adopt opened page 90 as main tab#12"));
    assert_eq!(
        closed,
        Err(EngineError("no page for main tab#12".to_owned()))
    );
}

// ---- waking the shell ----------------------------------------------------------------------

#[test]
fn events_wake_the_shell_once_per_batch_from_a_posted_task() {
    let (mut engine, fake) = with_pages(&[7]);
    let woken = Rc::new(Cell::new(0));
    let count = Rc::clone(&woken);
    engine.set_waker(move || count.set(count.get() + 1));

    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );
    report(EngineEventKind::PageOpenedTab, 90, Some(7), "");

    assert_eq!((fake.wakes.get(), woken.get()), (1, 0));
    ffi::run_wake();
    assert_eq!(woken.get(), 1);
}

#[test]
fn nothing_is_posted_without_a_waker() {
    let (_engine, fake) = with_pages(&[7]);

    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );

    assert_eq!(fake.wakes.get(), 0);
}

#[test]
fn setting_a_waker_wakes_it_for_events_already_waiting() {
    let (mut engine, fake) = with_pages(&[7]);
    report(EngineEventKind::PageOpenedTab, 90, Some(7), "");

    engine.set_waker(|| {});

    assert_eq!(fake.wakes.get(), 1);
}

#[test]
fn setting_a_waker_with_nothing_waiting_posts_nothing() {
    let (mut engine, fake) = engine();

    engine.set_waker(|| {});

    assert_eq!(fake.wakes.get(), 0);
}

#[test]
fn events_the_shell_left_get_the_next_wake() {
    let (mut engine, fake) = with_pages(&[7]);
    engine.set_waker(|| {});
    report(
        EngineEventKind::NavigationCommitted,
        7,
        None,
        "https://a.test/",
    );

    ffi::run_wake();

    assert_eq!(fake.wakes.get(), 2);
}

#[test]
fn a_drained_inbox_needs_no_further_wake() {
    let fake = Rc::new(FakeBridge::default());
    let engine = Rc::new(RefCell::new(ChromiumEngine::with_bridge(
        Rc::clone(&fake) as Rc<dyn Bridge>,
        |_| false,
    )));
    let drained = Rc::new(RefCell::new(Vec::new()));
    let (shell, sink) = (Rc::downgrade(&engine), Rc::clone(&drained));
    engine.borrow_mut().set_waker(move || {
        if let Some(engine) = shell.upgrade() {
            engine.borrow_mut().drain(&mut sink.borrow_mut());
        }
    });
    report(EngineEventKind::PageOpenedTab, 90, Some(7), "");

    ffi::run_wake();

    assert_eq!(drained.borrow().len(), 1);
    assert_eq!(fake.wakes.get(), 1);
}

#[test]
fn an_event_during_a_wake_is_posted_once_the_wake_returns() {
    let fake = Rc::new(FakeBridge::default());
    let engine = Rc::new(RefCell::new(ChromiumEngine::with_bridge(
        Rc::clone(&fake) as Rc<dyn Bridge>,
        |_| false,
    )));
    let shell = Rc::downgrade(&engine);
    let wakes_inside = Rc::new(Cell::new(usize::MAX));
    let inside = Rc::clone(&wakes_inside);
    let probe = Rc::clone(&fake);
    engine.borrow_mut().set_waker(move || {
        if let Some(engine) = shell.upgrade() {
            engine.borrow_mut().drain(&mut Vec::new());
        }
        report(EngineEventKind::PageOpenedTab, 91, Some(7), "");
        inside.set(probe.wakes.get());
    });
    report(EngineEventKind::PageOpenedTab, 90, Some(7), "");

    ffi::run_wake();

    assert_eq!((wakes_inside.get(), fake.wakes.get()), (1, 2));
}

// ---- connection ----------------------------------------------------------------------------

#[test]
fn keys_reach_the_engines_key_handler() {
    let fake = Rc::new(FakeBridge::default());
    let _engine = ChromiumEngine::with_bridge(fake as Rc<dyn Bridge>, |key: KeyEquivalent| {
        key.character == Some('t')
    });

    let consumed = [
        ffi::handle_key_equivalent(0x11, 0, u32::from('t'), false),
        ffi::handle_key_equivalent(0x10, 0, u32::from('y'), false),
    ];

    assert_eq!(consumed, [true, false]);
}

#[test]
fn a_dropped_engine_hears_nothing_more() {
    let fake = Rc::new(FakeBridge::default());
    let mut engine = ChromiumEngine::with_bridge(Rc::clone(&fake) as Rc<dyn Bridge>, |_| true);
    engine.set_waker(|| {});
    drop(engine);

    report(EngineEventKind::PageOpenedTab, 90, Some(7), "");

    assert_eq!(fake.wakes.get(), 0);
    assert!(!ffi::handle_key_equivalent(0x11, 0, 0, false));
}

#[test]
fn debug_output_summarizes_the_bookkeeping() {
    let (mut engine, _fake) = with_pages(&[7]);
    engine.show(window(2), tab(7)).unwrap();

    let text = format!("{engine:?}");

    assert_eq!(
        text,
        "ChromiumEngine { pages: 1, hosts: 0, deferred: {WindowId { profile: Main, serial: 2 }: \
         TabId { profile: Main, serial: 7 }}, jars: 0, .. }"
    );
}

#[cfg(not(feature = "chromium"))]
#[test]
fn without_chromium_the_engine_is_unavailable() {
    let mut engine = ChromiumEngine::new(|_| false);

    let created = engine.create(tab(7), &url("https://a.test/"), Jar::Main);

    assert_eq!(
        created,
        Err(EngineError(
            "create the page for main tab#7: built without the Chromium engine (the `chromium` feature)"
                .to_owned()
        ))
    );
}

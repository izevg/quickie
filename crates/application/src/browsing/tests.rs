use std::cell::Cell;
use std::rc::Rc;

use quickie_domain_browsing::{
    BrowsingError, Closed, Direction, Focus, Jar, JarChoice, JarId, PinnedSlot, RETENTION_MS,
    Session, TabId, WindowId,
};
use url::Url;

use super::Browsing;
use crate::fakes::{Call, FakeClock, FakeEngine, FakeRepo};
use crate::{AppError, EngineError, Navigation, StoreError};

type Subject = Browsing<FakeEngine, FakeRepo, FakeClock>;

fn url(path: &str) -> Url {
    Url::parse(&format!("https://example.test/{path}")).unwrap()
}

fn home() -> Url {
    url("home")
}

fn slot(n: u8) -> PinnedSlot {
    PinnedSlot::try_from(n).unwrap()
}

fn start_with(repo: FakeRepo) -> Subject {
    Browsing::start(FakeEngine::default(), repo, FakeClock::default(), &home()).unwrap()
}

/// A fresh start: window 0 with the home tab 0, shown; engine calls cleared.
fn started() -> Subject {
    let mut browsing = start_with(FakeRepo::default());
    browsing.engine.calls.clear();
    browsing
}

fn w0() -> WindowId {
    WindowId::new(0)
}

fn t(raw: u64) -> TabId {
    TabId::new(raw)
}

fn open(browsing: &mut Subject, path: &str) -> TabId {
    browsing
        .open(w0(), url(path), JarChoice::Main, Focus::Foreground)
        .unwrap()
}

/// A saved session: window 0 with tabs 0 (active) and 1, tab 1 pinned to slot 1.
fn saved_two_tabs() -> FakeRepo {
    let mut session = Session::new();
    let window = session.open_window();
    let back = session
        .open(window, url("b"), JarChoice::Main, Focus::Foreground)
        .unwrap();
    session
        .open(window, url("a"), JarChoice::Main, Focus::Foreground)
        .unwrap();
    session.pin(back, slot(1)).unwrap();
    FakeRepo {
        saved: Some(session.snapshot()),
        ..FakeRepo::default()
    }
}

fn fail_saves(browsing: &Subject) -> Rc<Cell<bool>> {
    let switch = Rc::clone(&browsing.repo.save_fails);
    switch.set(true);
    switch
}

mod starting {
    use super::*;

    #[test]
    fn a_first_launch_opens_home_and_saves() {
        let browsing = start_with(FakeRepo::default());

        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Create(t(0), home(), Jar::Main),
                Call::Show(w0(), t(0))
            ]
        );
        assert_eq!(browsing.session().active(w0()), Some(t(0)));
        assert_eq!(browsing.repo.saved, Some(browsing.session().snapshot()));
        assert_eq!(browsing.corrupt_on_start(), None);
    }

    #[test]
    fn a_restored_session_creates_only_the_active_pages() {
        let browsing = start_with(saved_two_tabs());

        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Create(t(1), url("a"), Jar::Main),
                Call::Show(w0(), t(1))
            ]
        );
        assert_eq!(browsing.session().in_slot(slot(1)), Some(t(0)));
    }

    #[test]
    fn an_empty_restored_window_gets_home() {
        let mut session = Session::new();
        session.open_window();
        let repo = FakeRepo {
            saved: Some(session.snapshot()),
            ..FakeRepo::default()
        };

        let browsing = start_with(repo);

        assert_eq!(
            browsing.session().tab(t(0)).map(|tab| tab.url.clone()),
            Some(home())
        );
        assert_eq!(browsing.session().windows().count(), 1);
    }

    #[test]
    fn a_corrupt_save_starts_fresh_and_says_why() {
        let mut repo = saved_two_tabs();
        if let Some(snapshot) = repo.saved.as_mut() {
            snapshot.next.tab = 0;
        }

        let browsing = start_with(repo);

        assert!(browsing.corrupt_on_start().is_some());
        assert_eq!(
            browsing.session().tab(t(0)).map(|tab| tab.url.clone()),
            Some(home())
        );
    }

    #[test]
    fn a_failed_load_stops_the_start() {
        let repo = FakeRepo {
            load_fails: true,
            ..FakeRepo::default()
        };

        let result = Browsing::start(FakeEngine::default(), repo, FakeClock::default(), &home());

        assert_eq!(
            result.err(),
            Some(AppError::Store(StoreError("load".to_owned())))
        );
    }

    #[test]
    fn a_failed_save_stops_the_start() {
        let repo = FakeRepo::default();
        repo.save_fails.set(true);

        let result = Browsing::start(FakeEngine::default(), repo, FakeClock::default(), &home());

        assert_eq!(
            result.err(),
            Some(AppError::Store(StoreError("save".to_owned())))
        );
    }

    #[test]
    fn a_failed_show_stops_the_start() {
        let engine = FakeEngine {
            failing: vec!["show"],
            ..FakeEngine::default()
        };

        let result = Browsing::start(engine, FakeRepo::default(), FakeClock::default(), &home());

        assert_eq!(
            result.err(),
            Some(AppError::Engine(EngineError("show".to_owned())))
        );
    }

    #[test]
    fn the_engine_is_reachable_for_the_shell() {
        let mut browsing = started();

        browsing.engine_mut().failing.push("show");

        assert_eq!(browsing.engine.failing, vec!["show"]);
    }
}

mod opening {
    use super::*;

    #[test]
    fn a_foreground_tab_is_created_shown_and_saved() {
        let mut browsing = started();
        let saves = browsing.repo.saves;

        let tab = open(&mut browsing, "a");

        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Create(tab, url("a"), Jar::Main),
                Call::Show(w0(), tab)
            ]
        );
        assert_eq!(browsing.repo.saves, saves + 1);
        assert_eq!(browsing.repo.saved, Some(browsing.session().snapshot()));
    }

    #[test]
    fn a_background_tab_loads_without_being_shown() {
        let mut browsing = started();

        let tab = browsing
            .open(w0(), url("a"), JarChoice::Main, Focus::Background)
            .unwrap();

        assert_eq!(
            browsing.engine.calls,
            vec![Call::Create(tab, url("a"), Jar::Main)]
        );
    }

    #[test]
    fn a_background_tab_in_an_empty_window_is_shown() {
        let mut browsing = started();
        let window = browsing.open_window().unwrap();

        let tab = browsing
            .open(window, url("a"), JarChoice::Main, Focus::Background)
            .unwrap();

        assert_eq!(browsing.engine.calls.last(), Some(&Call::Show(window, tab)));
    }

    #[test]
    fn an_unknown_window_changes_nothing() {
        let mut browsing = started();
        let before = browsing.session().clone();
        let ghost = WindowId::new(9);

        let result = browsing.open(ghost, url("a"), JarChoice::Main, Focus::Foreground);

        assert_eq!(
            result,
            Err(AppError::Browsing(BrowsingError::UnknownWindow(ghost)))
        );
        assert_eq!(browsing.session(), &before);
        assert_eq!(browsing.engine.calls, Vec::new());
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let mut browsing = started();
        let before = browsing.session().clone();
        fail_saves(&browsing);

        let result = browsing.open(w0(), url("a"), JarChoice::Main, Focus::Foreground);

        assert_eq!(result, Err(AppError::Store(StoreError("save".to_owned()))));
        assert_eq!(browsing.session(), &before);
        assert_eq!(browsing.engine.calls, Vec::new());
    }

    #[test]
    fn a_failed_create_keeps_the_tab_and_retries_on_show() {
        let mut browsing = started();
        browsing.engine.failing.push("create");

        let result = browsing.open(w0(), url("a"), JarChoice::Main, Focus::Foreground);
        browsing.engine.failing.clear();
        browsing.activate(t(1)).unwrap();

        assert_eq!(
            result,
            Err(AppError::Engine(EngineError("create".to_owned())))
        );
        assert_eq!(
            browsing
                .engine
                .calls
                .iter()
                .filter(|c| matches!(c, Call::Create(..)))
                .count(),
            2
        );
    }

    #[test]
    fn a_new_window_is_saved() {
        let mut browsing = started();

        let window = browsing.open_window().unwrap();

        assert_eq!(window, WindowId::new(1));
        assert_eq!(browsing.repo.saved, Some(browsing.session().snapshot()));
    }
}

mod adopting {
    use super::*;

    #[test]
    fn a_popup_joins_its_openers_window_and_jar_without_a_new_page() {
        let mut browsing = started();
        let window = browsing.open_window().unwrap();
        let opener = browsing
            .open(window, url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        browsing.engine.calls.clear();

        let popup = browsing
            .adopt(opener, url("login"), Focus::Foreground)
            .unwrap();

        assert_eq!(browsing.engine.calls, vec![Call::Show(window, popup)]);
        assert_eq!(
            browsing.session().tab(popup).unwrap().jar,
            Jar::Throwaway(JarId::new(0))
        );
    }

    #[test]
    fn a_background_popup_is_not_shown_and_never_recreated() {
        let mut browsing = started();

        let popup = browsing.adopt(t(0), url("b"), Focus::Background).unwrap();
        browsing.activate(popup).unwrap();

        assert_eq!(browsing.engine.calls, vec![Call::Show(w0(), popup)]);
    }

    #[test]
    fn an_unknown_opener_is_an_error() {
        let mut browsing = started();

        let result = browsing.adopt(t(9), url("b"), Focus::Foreground);

        assert_eq!(
            result,
            Err(AppError::Browsing(BrowsingError::UnknownTab(t(9))))
        );
    }
}

mod activating {
    use super::*;

    #[test]
    fn shows_the_tab_and_names_its_window() {
        let mut browsing = started();
        let tab = open(&mut browsing, "a");
        browsing.engine.calls.clear();

        let window = browsing.activate(t(0));

        assert_eq!(window, Ok(w0()));
        assert_eq!(browsing.engine.calls, vec![Call::Show(w0(), t(0))]);
        assert_eq!(browsing.session().mru(w0()), Some([t(0), tab].as_slice()));
    }

    #[test]
    fn a_restored_tab_gets_its_page_once() {
        let mut browsing = start_with(saved_two_tabs());
        browsing.engine.calls.clear();

        browsing.activate(t(0)).unwrap();
        browsing.activate(t(0)).unwrap();

        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Create(t(0), url("b"), Jar::Main),
                Call::Show(w0(), t(0)),
                Call::Show(w0(), t(0))
            ]
        );
    }

    #[test]
    fn an_unknown_tab_is_an_error() {
        let mut browsing = started();

        assert_eq!(
            browsing.activate(t(9)),
            Err(AppError::Browsing(BrowsingError::UnknownTab(t(9))))
        );
    }

    #[test]
    fn a_slot_activates_its_tab() {
        let mut browsing = started();
        open(&mut browsing, "a");
        browsing.pin(t(0), slot(2)).unwrap();
        browsing.engine.calls.clear();

        let window = browsing.go_to_slot(slot(2));

        assert_eq!(window, Ok(Some(w0())));
        assert_eq!(browsing.session().active(w0()), Some(t(0)));
    }

    #[test]
    fn an_empty_slot_does_nothing() {
        let mut browsing = started();

        assert_eq!(browsing.go_to_slot(slot(5)), Ok(None));
        assert_eq!(browsing.engine.calls, Vec::new());
    }

    #[test]
    fn pin_reports_the_displaced_tab_and_unpin_the_holder() {
        let mut browsing = started();
        let tab = open(&mut browsing, "a");
        browsing.pin(t(0), slot(1)).unwrap();

        let displaced = browsing.pin(tab, slot(1));
        let held = browsing.unpin(slot(1));

        assert_eq!(displaced, Ok(Some(t(0))));
        assert_eq!(held, Ok(Some(tab)));
        assert_eq!(browsing.repo.saved, Some(browsing.session().snapshot()));
    }

    #[test]
    fn a_cycle_commits_the_selection() {
        let mut browsing = started();
        open(&mut browsing, "a");
        let mut cycle = browsing.begin_cycle(w0(), Direction::Forward).unwrap();
        cycle.step(Direction::Forward);

        let window = browsing.commit_cycle(&cycle);

        assert_eq!(window, Ok(w0()));
        assert_eq!(browsing.session().active(w0()), Some(t(1)));
    }

    #[test]
    fn a_cycle_needs_two_tabs_in_a_known_window() {
        let browsing = started();

        assert_eq!(browsing.begin_cycle(w0(), Direction::Forward), None);
        assert_eq!(
            browsing.begin_cycle(WindowId::new(9), Direction::Forward),
            None
        );
    }
}

mod closing {
    use super::*;

    #[test]
    fn closes_the_page_and_shows_the_next_tab() {
        let mut browsing = started();
        let tab = open(&mut browsing, "a");
        browsing.engine.calls.clear();

        let closed = browsing.close(tab);

        assert_eq!(
            closed,
            Ok(Closed {
                window: w0(),
                next_active: Some(t(0)),
                released_jar: None
            })
        );
        assert_eq!(
            browsing.engine.calls,
            vec![Call::Close(tab), Call::Show(w0(), t(0))]
        );
    }

    #[test]
    fn a_tab_without_a_page_closes_without_the_engine() {
        let mut browsing = start_with(saved_two_tabs());
        browsing.engine.calls.clear();

        browsing.close(t(0)).unwrap();

        assert_eq!(browsing.engine.calls, vec![Call::Show(w0(), t(1))]);
    }

    #[test]
    fn the_last_tab_of_a_jar_destroys_it_even_if_the_page_fails_to_close() {
        let mut browsing = started();
        let tab = browsing
            .open(w0(), url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        browsing.engine.failing.push("close");
        browsing.engine.calls.clear();

        let result = browsing.close(tab);

        assert_eq!(
            result,
            Err(AppError::Engine(EngineError("close".to_owned())))
        );
        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Close(tab),
                Call::DestroyJar(JarId::new(0)),
                Call::Show(w0(), t(0))
            ]
        );
        assert_eq!(browsing.session().tab(tab), None);
    }

    #[test]
    fn closing_the_last_tab_shows_nothing() {
        let mut browsing = started();

        let closed = browsing.close(t(0)).unwrap();

        assert_eq!(closed.next_active, None);
        assert_eq!(browsing.engine.calls, vec![Call::Close(t(0))]);
    }

    #[test]
    fn closing_uses_the_clock() {
        let mut browsing = started();
        browsing.clock.0.set(42);

        browsing.close(t(0)).unwrap();

        let closed_at = browsing
            .session()
            .trash()
            .next()
            .map(|entry| entry.closed_at.as_millis());
        assert_eq!(closed_at, Some(42));
    }

    #[test]
    fn an_unknown_tab_is_an_error() {
        let mut browsing = started();

        assert_eq!(
            browsing.close(t(9)),
            Err(AppError::Browsing(BrowsingError::UnknownTab(t(9))))
        );
    }

    #[test]
    fn a_window_closes_every_page_and_jar() {
        let mut browsing = started();
        let a = browsing
            .open(w0(), url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        let b = browsing
            .open(w0(), url("b"), JarChoice::NewThrowaway, Focus::Background)
            .unwrap();
        browsing.engine.failing.push("close");
        browsing.engine.calls.clear();

        let result = browsing.close_window(w0());

        assert_eq!(
            result,
            Err(AppError::Engine(EngineError("close".to_owned())))
        );
        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Close(b),
                Call::Close(t(0)),
                Call::Close(a),
                Call::DestroyJar(JarId::new(1)),
                Call::DestroyJar(JarId::new(0))
            ]
        );
        assert_eq!(browsing.session().windows().count(), 0);
    }

    #[test]
    fn closing_a_window_reports_its_tabs() {
        let mut browsing = started();

        let closed = browsing.close_window(w0()).unwrap();

        assert_eq!(closed.tabs, vec![t(0)]);
    }

    #[test]
    fn a_failed_jar_destroy_is_reported() {
        let mut browsing = started();
        let tab = browsing
            .open(w0(), url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        browsing.engine.failing.push("destroy_jar");

        let result = browsing.close(tab);

        assert_eq!(
            result,
            Err(AppError::Engine(EngineError("destroy_jar".to_owned())))
        );
    }

    #[test]
    fn closing_an_unknown_window_is_an_error() {
        let mut browsing = started();
        let ghost = WindowId::new(9);

        assert_eq!(
            browsing.close_window(ghost),
            Err(AppError::Browsing(BrowsingError::UnknownWindow(ghost)))
        );
    }
}

mod reopening {
    use super::*;

    #[test]
    fn brings_back_the_tab_with_a_new_page() {
        let mut browsing = started();
        let tab = open(&mut browsing, "a");
        browsing.close(tab).unwrap();
        browsing.engine.calls.clear();

        let reopened = browsing.reopen_closed(w0());

        assert_eq!(reopened, Ok(Some(tab)));
        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Create(tab, url("a"), Jar::Main),
                Call::Show(w0(), tab)
            ]
        );
    }

    #[test]
    fn an_empty_trash_does_nothing() {
        let mut browsing = started();

        assert_eq!(browsing.reopen_closed(w0()), Ok(None));
        assert_eq!(browsing.engine.calls, Vec::new());
    }

    #[test]
    fn an_unknown_window_is_an_error() {
        let mut browsing = started();
        let ghost = WindowId::new(9);

        assert_eq!(
            browsing.reopen_closed(ghost),
            Err(AppError::Browsing(BrowsingError::UnknownWindow(ghost)))
        );
    }

    #[test]
    fn purging_uses_the_clock() {
        let mut browsing = started();
        open(&mut browsing, "a");
        browsing.close(t(1)).unwrap();
        browsing.clock.0.set(RETENTION_MS - 1);
        let kept = browsing.purge_trash();
        browsing.clock.0.set(RETENTION_MS);

        let purged = browsing.purge_trash();

        assert_eq!(kept, Ok(0));
        assert_eq!(purged, Ok(1));
        assert_eq!(browsing.repo.saved, Some(browsing.session().snapshot()));
    }
}

mod navigating {
    use super::*;

    #[test]
    fn a_restored_tab_gets_a_page_before_navigating() {
        let mut browsing = start_with(saved_two_tabs());
        browsing.engine.calls.clear();

        browsing.navigate(t(0), &Navigation::Reload).unwrap();

        assert_eq!(
            browsing.engine.calls,
            vec![
                Call::Create(t(0), url("b"), Jar::Main),
                Call::Navigate(t(0), Navigation::Reload)
            ]
        );
    }

    #[test]
    fn an_unknown_tab_is_an_error() {
        let mut browsing = started();

        assert_eq!(
            browsing.navigate(t(9), &Navigation::Back),
            Err(AppError::Browsing(BrowsingError::UnknownTab(t(9))))
        );
    }

    #[test]
    fn a_failed_navigation_is_reported() {
        let mut browsing = started();
        browsing.engine.failing.push("navigate");

        assert_eq!(
            browsing.navigate(t(0), &Navigation::Stop),
            Err(AppError::Engine(EngineError("navigate".to_owned())))
        );
    }

    #[test]
    fn a_committed_navigation_is_saved() {
        let mut browsing = started();

        browsing
            .navigated(t(0), url("z"), "Zed".to_owned())
            .unwrap();

        let tab = browsing.session().tab(t(0)).unwrap();
        assert_eq!((tab.url.clone(), tab.title.as_str()), (url("z"), "Zed"));
        assert_eq!(browsing.repo.saved, Some(browsing.session().snapshot()));
    }

    #[test]
    fn a_navigation_of_a_closed_tab_is_benign() {
        let mut browsing = started();

        assert_eq!(browsing.navigated(t(9), url("z"), String::new()), Ok(()));
    }

    #[test]
    fn a_failed_save_of_a_navigation_is_reported() {
        let mut browsing = started();
        fail_saves(&browsing);

        assert_eq!(
            browsing.navigated(t(0), url("z"), String::new()),
            Err(AppError::Store(StoreError("save".to_owned())))
        );
    }
}

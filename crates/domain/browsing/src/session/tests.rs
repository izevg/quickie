use super::{Closed, ClosedTab, ClosedWindow, CorruptSession, NextIds, RETENTION_MS, Session};
use crate::{
    BrowsingError, Focus, Jar, JarChoice, JarId, PinnedSlot, Tab, TabId, Timestamp, WindowId,
};
use url::Url;

fn url(path: &str) -> Url {
    Url::parse(&format!("https://example.test/{path}")).unwrap()
}

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn slot(n: u8) -> PinnedSlot {
    PinnedSlot::try_from(n).unwrap()
}

fn open(session: &mut Session, window: WindowId, path: &str) -> TabId {
    session
        .open(window, url(path), JarChoice::Main, Focus::Foreground)
        .unwrap()
}

fn mru(session: &Session, window: WindowId) -> Vec<TabId> {
    session.mru(window).unwrap().to_vec()
}

/// A window with tabs a, b, c opened in that order; c is active.
fn three_tabs() -> (Session, WindowId, [TabId; 3]) {
    let mut session = Session::new();
    let window = session.open_window();
    let a = open(&mut session, window, "a");
    let b = open(&mut session, window, "b");
    let c = open(&mut session, window, "c");
    (session, window, [a, b, c])
}

mod opening {
    use super::*;

    #[test]
    fn windows_and_tabs_get_fresh_ids() {
        let mut session = Session::new();

        let first = session.open_window();
        let second = session.open_window();
        let a = open(&mut session, first, "a");
        let b = open(&mut session, second, "b");

        assert_ne!(first, second);
        assert_ne!(a, b);
        assert_eq!(session.windows().collect::<Vec<_>>(), vec![first, second]);
    }

    #[test]
    fn a_new_tab_starts_untitled_at_its_url() {
        let mut session = Session::new();
        let window = session.open_window();

        let id = open(&mut session, window, "a");

        let expected = Tab {
            id,
            url: url("a"),
            title: String::new(),
            jar: Jar::Main,
        };
        assert_eq!(session.tab(id), Some(&expected));
    }

    #[test]
    fn a_foreground_tab_becomes_active() {
        let (session, window, [a, b, c]) = three_tabs();

        assert_eq!(session.active(window), Some(c));
        assert_eq!(mru(&session, window), vec![c, b, a]);
    }

    #[test]
    fn a_background_tab_joins_the_back() {
        let (mut session, window, [a, b, c]) = three_tabs();

        let d = session
            .open(window, url("d"), JarChoice::Main, Focus::Background)
            .unwrap();

        assert_eq!(mru(&session, window), vec![c, b, a, d]);
    }

    #[test]
    fn a_background_tab_in_an_empty_window_is_active() {
        let mut session = Session::new();
        let window = session.open_window();

        let id = session
            .open(window, url("a"), JarChoice::Main, Focus::Background)
            .unwrap();

        assert_eq!(session.active(window), Some(id));
    }

    #[test]
    fn opening_into_an_unknown_window_fails_and_allocates_nothing() {
        let mut session = Session::new();
        let ghost = WindowId::new(7);

        let result = session.open(ghost, url("a"), JarChoice::NewThrowaway, Focus::Foreground);

        assert_eq!(result, Err(BrowsingError::UnknownWindow(ghost)));
        assert_eq!(session.snapshot().next, NextIds::default());
    }

    #[test]
    fn throwaway_tabs_get_separate_jars() {
        let mut session = Session::new();
        let window = session.open_window();

        let a = session
            .open(window, url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        let b = session
            .open(window, url("b"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();

        assert_eq!(session.tab(a).unwrap().jar, Jar::Throwaway(JarId::new(0)));
        assert_eq!(session.tab(b).unwrap().jar, Jar::Throwaway(JarId::new(1)));
    }

    #[test]
    fn a_popup_shares_its_openers_jar() {
        let mut session = Session::new();
        let window = session.open_window();
        let opener = session
            .open(window, url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();

        let popup = session
            .open(
                window,
                url("b"),
                JarChoice::SameAs(opener),
                Focus::Foreground,
            )
            .unwrap();

        assert_eq!(
            session.tab(popup).unwrap().jar,
            session.tab(opener).unwrap().jar
        );
    }

    #[test]
    fn a_popup_of_an_unknown_opener_fails() {
        let mut session = Session::new();
        let window = session.open_window();
        let ghost = TabId::new(9);

        let result = session.open(
            window,
            url("b"),
            JarChoice::SameAs(ghost),
            Focus::Foreground,
        );

        assert_eq!(result, Err(BrowsingError::UnknownTab(ghost)));
    }
}

mod activating {
    use super::*;

    #[test]
    fn moves_the_tab_to_the_front_and_names_its_window() {
        let (mut session, window, [a, b, c]) = three_tabs();
        let other = session.open_window();
        open(&mut session, other, "x");

        let focused = session.activate(a);

        assert_eq!(focused, Ok(window));
        assert_eq!(mru(&session, window), vec![a, c, b]);
    }

    #[test]
    fn rejects_an_unknown_tab() {
        let (mut session, _, _) = three_tabs();

        assert_eq!(
            session.activate(TabId::new(99)),
            Err(BrowsingError::UnknownTab(TabId::new(99)))
        );
    }

    #[test]
    fn lists_tabs_in_opening_order_whatever_the_mru() {
        let (mut session, window, [a, b, c]) = three_tabs();
        let other = session.open_window();
        open(&mut session, other, "x");

        session.activate(a).unwrap();

        let ids: Vec<TabId> = session.tabs(window).unwrap().map(|tab| tab.id).collect();
        assert_eq!(ids, vec![a, b, c]);
    }

    #[test]
    fn listing_an_unknown_window_fails() {
        let session = Session::new();
        let ghost = WindowId::new(3);

        assert!(matches!(session.tabs(ghost), Err(BrowsingError::UnknownWindow(w)) if w == ghost));
        assert_eq!(session.mru(ghost), None);
        assert_eq!(session.active(ghost), None);
    }
}

mod closing {
    use super::*;

    #[test]
    fn the_previous_tab_becomes_active() {
        let (mut session, window, [a, b, c]) = three_tabs();

        let closed = session.close(c, at(5));

        assert_eq!(
            closed,
            Ok(Closed {
                window,
                next_active: Some(b),
                released_jar: None
            })
        );
        assert_eq!(mru(&session, window), vec![b, a]);
        assert_eq!(session.tab(c), None);
    }

    #[test]
    fn the_tab_goes_to_the_trash() {
        let (mut session, _, [_, _, c]) = three_tabs();
        let tab = session.tab(c).unwrap().clone();

        session.close(c, at(5)).unwrap();

        assert_eq!(
            session.trash().collect::<Vec<_>>(),
            vec![&ClosedTab {
                tab,
                closed_at: at(5)
            }]
        );
    }

    #[test]
    fn the_last_tab_leaves_an_empty_window() {
        let mut session = Session::new();
        let window = session.open_window();
        let a = open(&mut session, window, "a");

        let closed = session.close(a, at(5)).unwrap();

        assert_eq!(closed.next_active, None);
        assert_eq!(session.windows().collect::<Vec<_>>(), vec![window]);
    }

    #[test]
    fn the_slot_is_freed() {
        let (mut session, _, [a, _, _]) = three_tabs();
        session.pin(a, slot(1)).unwrap();

        session.close(a, at(5)).unwrap();

        assert_eq!(session.in_slot(slot(1)), None);
    }

    #[test]
    fn a_jar_is_released_with_its_last_tab() {
        let mut session = Session::new();
        let window = session.open_window();
        let a = session
            .open(window, url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        let b = session
            .open(window, url("b"), JarChoice::SameAs(a), Focus::Foreground)
            .unwrap();

        let first = session.close(a, at(5)).unwrap();
        let second = session.close(b, at(6)).unwrap();

        assert_eq!(first.released_jar, None);
        assert_eq!(second.released_jar, Some(JarId::new(0)));
    }

    #[test]
    fn closing_an_unknown_tab_fails() {
        let (mut session, _, _) = three_tabs();

        assert_eq!(
            session.close(TabId::new(99), at(5)),
            Err(BrowsingError::UnknownTab(TabId::new(99)))
        );
        assert_eq!(session.trash().count(), 0);
    }

    #[test]
    fn a_window_closes_with_its_tabs_least_recent_first() {
        let (mut session, window, [a, b, c]) = three_tabs();
        let other = session.open_window();
        let x = open(&mut session, other, "x");

        let closed = session.close_window(window, at(5));

        assert_eq!(
            closed,
            Ok(ClosedWindow {
                tabs: vec![a, b, c],
                released_jars: Vec::new()
            })
        );
        assert_eq!(session.windows().collect::<Vec<_>>(), vec![other]);
        let trashed: Vec<TabId> = session.trash().map(|entry| entry.tab.id).collect();
        assert_eq!(trashed, vec![a, b, c]);
        assert_eq!(session.tab(x).map(|tab| tab.id), Some(x));
    }

    #[test]
    fn closing_a_window_releases_its_jars() {
        let mut session = Session::new();
        let window = session.open_window();
        let a = session
            .open(window, url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        session
            .open(window, url("b"), JarChoice::SameAs(a), Focus::Foreground)
            .unwrap();
        session
            .open(window, url("c"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();

        let closed = session.close_window(window, at(5)).unwrap();

        assert_eq!(closed.released_jars, vec![JarId::new(0), JarId::new(1)]);
    }

    #[test]
    fn closing_an_unknown_window_fails() {
        let mut session = Session::new();

        assert_eq!(
            session.close_window(WindowId::new(4), at(5)),
            Err(BrowsingError::UnknownWindow(WindowId::new(4)))
        );
    }
}

mod reopening {
    use super::*;

    #[test]
    fn brings_back_the_latest_with_its_id_active() {
        let (mut session, window, [a, b, c]) = three_tabs();
        session.close(b, at(5)).unwrap();
        session.close(c, at(6)).unwrap();

        let reopened = session.reopen_closed(window);

        assert_eq!(reopened, Ok(Some(c)));
        assert_eq!(mru(&session, window), vec![c, a]);
        assert_eq!(session.tab(c).map(|tab| tab.url.clone()), Some(url("c")));
        assert_eq!(session.trash().count(), 1);
    }

    #[test]
    fn a_closed_window_comes_back_active_tab_first() {
        let (mut session, window, [_, _, c]) = three_tabs();
        session.close_window(window, at(5)).unwrap();
        let fresh = session.open_window();

        assert_eq!(session.reopen_closed(fresh), Ok(Some(c)));
    }

    #[test]
    fn a_throwaway_tab_comes_back_in_a_fresh_jar() {
        let mut session = Session::new();
        let window = session.open_window();
        let a = session
            .open(window, url("a"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();
        session.close(a, at(5)).unwrap();

        session.reopen_closed(window).unwrap();

        assert_eq!(session.tab(a).unwrap().jar, Jar::Throwaway(JarId::new(1)));
    }

    #[test]
    fn a_main_tab_stays_in_main() {
        let (mut session, window, [_, _, c]) = three_tabs();
        session.close(c, at(5)).unwrap();

        session.reopen_closed(window).unwrap();

        assert_eq!(session.tab(c).unwrap().jar, Jar::Main);
        assert_eq!(session.snapshot().next.jar, 0);
    }

    #[test]
    fn an_empty_trash_reopens_nothing() {
        let (mut session, window, _) = three_tabs();

        assert_eq!(session.reopen_closed(window), Ok(None));
    }

    #[test]
    fn an_unknown_window_keeps_the_trash() {
        let (mut session, _, [_, _, c]) = three_tabs();
        session.close(c, at(5)).unwrap();

        let result = session.reopen_closed(WindowId::new(9));

        assert_eq!(result, Err(BrowsingError::UnknownWindow(WindowId::new(9))));
        assert_eq!(session.trash().count(), 1);
    }

    #[test]
    fn the_trash_is_purged_after_thirty_days() {
        let (mut session, _, [a, b, _]) = three_tabs();
        session.close(a, at(0)).unwrap();
        session.close(b, at(10)).unwrap();

        let purged = session.purge_trash(at(RETENTION_MS + 5));

        assert_eq!(
            purged.iter().map(|entry| entry.tab.id).collect::<Vec<_>>(),
            vec![a]
        );
        assert_eq!(
            session
                .trash()
                .map(|entry| entry.tab.id)
                .collect::<Vec<_>>(),
            vec![b]
        );
    }
}

mod navigating {
    use super::*;

    #[test]
    fn records_the_url_and_title() {
        let (mut session, _, [a, _, _]) = three_tabs();

        session.navigated(a, url("z"), "Zed".to_owned()).unwrap();

        let tab = session.tab(a).unwrap();
        assert_eq!((&tab.url, tab.title.as_str()), (&url("z"), "Zed"));
    }

    #[test]
    fn an_unknown_tab_is_reported() {
        let mut session = Session::new();

        let result = session.navigated(TabId::new(1), url("z"), String::new());

        assert_eq!(result, Err(BrowsingError::UnknownTab(TabId::new(1))));
    }
}

mod pinning {
    use super::*;

    #[test]
    fn pins_a_tab_to_a_slot() {
        let (mut session, _, [a, _, _]) = three_tabs();

        let displaced = session.pin(a, slot(3));

        assert_eq!(displaced, Ok(None));
        assert_eq!(session.in_slot(slot(3)), Some(a));
        assert_eq!(session.slot_of(a), Some(slot(3)));
    }

    #[test]
    fn overwrites_like_a_vim_mark() {
        let (mut session, _, [a, b, _]) = three_tabs();
        session.pin(a, slot(3)).unwrap();

        let displaced = session.pin(b, slot(3));

        assert_eq!(displaced, Ok(Some(a)));
        assert_eq!(session.slot_of(a), None);
    }

    #[test]
    fn moves_a_pinned_tab_to_its_new_slot() {
        let (mut session, _, [a, _, _]) = three_tabs();
        session.pin(a, slot(9)).unwrap();

        session.pin(a, slot(2)).unwrap();

        assert_eq!(session.in_slot(slot(9)), None);
        assert_eq!(session.slot_of(a), Some(slot(2)));
    }

    #[test]
    fn rejects_an_unknown_tab() {
        let (mut session, _, [a, _, _]) = three_tabs();
        session.pin(a, slot(1)).unwrap();

        let result = session.pin(TabId::new(99), slot(1));

        assert_eq!(result, Err(BrowsingError::UnknownTab(TabId::new(99))));
        assert_eq!(session.in_slot(slot(1)), Some(a));
    }

    #[test]
    fn unpin_empties_the_slot() {
        let (mut session, _, [a, _, _]) = three_tabs();
        session.pin(a, slot(4)).unwrap();

        assert_eq!(session.unpin(slot(4)), Some(a));
        assert_eq!(session.unpin(slot(4)), None);
        assert_eq!(session.slot_of(a), None);
    }
}

mod restoring {
    use super::*;

    /// Two windows, a throwaway pair, a pinned tab, one tab in the trash.
    fn busy() -> Session {
        let mut session = Session::new();
        let first = session.open_window();
        let second = session.open_window();
        let a = open(&mut session, first, "a");
        let b = session
            .open(first, url("b"), JarChoice::NewThrowaway, Focus::Background)
            .unwrap();
        session
            .open(second, url("c"), JarChoice::SameAs(b), Focus::Foreground)
            .unwrap();
        let d = open(&mut session, second, "d");
        session.pin(a, slot(1)).unwrap();
        session.close(d, at(5)).unwrap();
        session
    }

    fn corrupt(
        edit: impl FnOnce(&mut super::super::SessionSnapshot),
    ) -> Result<Session, CorruptSession> {
        let mut snapshot = busy().snapshot();
        edit(&mut snapshot);
        Session::restore(snapshot)
    }

    #[test]
    fn round_trips() {
        let session = busy();

        assert_eq!(Session::restore(session.snapshot()), Ok(session));
    }

    #[test]
    fn counters_continue_after_restore() {
        let mut session = Session::restore(busy().snapshot()).unwrap();
        let window = session.open_window();

        let tab = session
            .open(window, url("e"), JarChoice::NewThrowaway, Focus::Foreground)
            .unwrap();

        assert_eq!(window, WindowId::new(2));
        assert_eq!(tab, TabId::new(4));
        assert_eq!(session.tab(tab).unwrap().jar, Jar::Throwaway(JarId::new(1)));
    }

    #[test]
    fn rejects_a_live_tab_twice() {
        let result = corrupt(|s| s.tabs.push(s.tabs[0].clone()));

        assert_eq!(result, Err(CorruptSession::DuplicateTab(TabId::new(0))));
    }

    #[test]
    fn rejects_a_trashed_tab_that_is_also_live() {
        let result = corrupt(|s| s.trash[0].tab.id = TabId::new(0));

        assert_eq!(result, Err(CorruptSession::DuplicateTab(TabId::new(0))));
    }

    #[test]
    fn rejects_a_tab_id_the_counter_would_hand_out() {
        let result = corrupt(|s| s.next.tab = 3);

        assert_eq!(
            result,
            Err(CorruptSession::TabIdNotAllocated(TabId::new(3)))
        );
    }

    #[test]
    fn rejects_a_jar_id_the_counter_would_hand_out() {
        let result = corrupt(|s| s.next.jar = 0);

        assert_eq!(
            result,
            Err(CorruptSession::JarIdNotAllocated(JarId::new(0)))
        );
    }

    #[test]
    fn checks_jars_in_the_trash_too() {
        let result = corrupt(|s| s.trash[0].tab.jar = Jar::Throwaway(JarId::new(1)));

        assert_eq!(
            result,
            Err(CorruptSession::JarIdNotAllocated(JarId::new(1)))
        );
    }

    #[test]
    fn rejects_a_window_twice() {
        let result = corrupt(|s| s.windows.push((WindowId::new(0), Vec::new())));

        assert_eq!(
            result,
            Err(CorruptSession::DuplicateWindow(WindowId::new(0)))
        );
    }

    #[test]
    fn rejects_a_window_id_the_counter_would_hand_out() {
        let result = corrupt(|s| s.next.window = 1);

        assert_eq!(
            result,
            Err(CorruptSession::WindowIdNotAllocated(WindowId::new(1)))
        );
    }

    #[test]
    fn rejects_a_window_listing_a_dead_tab() {
        let result = corrupt(|s| s.windows[0].1.push(TabId::new(3)));

        assert_eq!(
            result,
            Err(CorruptSession::UnknownTabInWindow(TabId::new(3)))
        );
    }

    #[test]
    fn rejects_a_tab_in_two_windows() {
        let result = corrupt(|s| s.windows[1].1.push(TabId::new(0)));

        assert_eq!(
            result,
            Err(CorruptSession::TabNotInOneWindow(TabId::new(0)))
        );
    }

    #[test]
    fn rejects_a_tab_in_no_window() {
        let result = corrupt(|s| s.windows[0].1.retain(|&id| id != TabId::new(1)));

        assert_eq!(
            result,
            Err(CorruptSession::TabNotInOneWindow(TabId::new(1)))
        );
    }

    #[test]
    fn rejects_a_slot_holding_a_dead_tab() {
        let result = corrupt(|s| s.slots[4] = Some(TabId::new(3)));

        assert_eq!(result, Err(CorruptSession::UnknownTabInSlot(TabId::new(3))));
    }

    #[test]
    fn rejects_a_tab_in_two_slots() {
        let result = corrupt(|s| s.slots[4] = Some(TabId::new(0)));

        assert_eq!(result, Err(CorruptSession::TabInTwoSlots(TabId::new(0))));
    }
}

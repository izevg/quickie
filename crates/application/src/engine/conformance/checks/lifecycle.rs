//! A page's life (core) and how its events reach the shell (events).

use std::cell::Cell;
use std::rc::Rc;

use quickie_domain_browsing::Jar;

use super::{Case, Checked, Run, about, ensure, first_navigation, same, tab, visit};
use crate::conformance::{Group, Harness, WINDOW};
use crate::{EngineEvent, EnginePort, Navigation};

pub(super) fn cases<H: Harness>() -> Vec<Case<H>> {
    let case = |group, name, run| Case { group, name, run };
    vec![
        case(
            Group::Core,
            "a_created_page_commits_its_url_and_title",
            created_page_commits,
        ),
        case(
            Group::Core,
            "a_tab_has_no_page_until_one_is_created",
            no_page_until_created,
        ),
        case(Group::Core, "a_tab_gets_one_page_only", one_page_per_tab),
        case(
            Group::Core,
            "a_page_loads_goes_back_and_forward_and_reloads",
            navigates,
        ),
        case(
            Group::Core,
            "a_closed_page_reports_nothing_more",
            closed_page_is_silent,
        ),
        case(
            Group::Events,
            "a_commit_comes_before_the_title_changes_it",
            commit_before_title,
        ),
        case(Group::Events, "the_waker_runs_when_events_wait", waker_runs),
        case(
            Group::Events,
            "nothing_is_delivered_inside_an_engine_call",
            nothing_inside_calls,
        ),
    ]
}

fn created_page_commits<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;

    run.wait_visit(1, "plain", "Plain")
}

fn no_page_until_created<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let shown = run.engine.show(WINDOW, tab(1));
    let navigated = run.engine.navigate(tab(1), &Navigation::Reload);
    let closed = run.engine.close(tab(1));

    ensure(shown.is_err(), || {
        "it showed a tab without a page".to_owned()
    })?;
    ensure(navigated.is_err(), || {
        "it navigated a tab without a page".to_owned()
    })?;
    ensure(closed.is_err(), || {
        "it closed a tab without a page".to_owned()
    })?;
    run.create(1, "plain", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing a created page failed: {error}"))
}

fn one_page_per_tab<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;

    let again = run.create(1, "other", Jar::Main);

    ensure(again.is_err(), || {
        "it created a second page for one tab".to_owned()
    })
}

fn navigates<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let other = run.url("other");
    for (navigation, path, title) in [
        (Navigation::Load(other), "other", "Other"),
        (Navigation::Back, "plain", "Plain"),
        (Navigation::Forward, "other", "Other"),
    ] {
        run.engine
            .navigate(tab(1), &navigation)
            .map_err(|error| format!("{navigation:?} failed: {error}"))?;
        run.wait_visit(1, path, title)?;
    }
    let mark = run.events.len();
    run.engine
        .navigate(tab(1), &Navigation::Reload)
        .map_err(|error| format!("reloading failed: {error}"))?;
    run.wait("the reload's commit", |events| {
        events.iter().skip(mark).any(
            |event| matches!(event, EngineEvent::Committed { tab: about, .. } if *about == tab(1)),
        )
    })
}

fn closed_page_is_silent<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "late", Jar::Main)?;
    run.create(2, "late", Jar::Main)?;
    let page = run.url("late");
    run.wait("the page's commit", |events| {
        visit(events, tab(1)).is_some_and(|(at, _)| at == page)
    })?;

    run.engine
        .close(tab(1))
        .map_err(|error| format!("closing failed: {error}"))?;
    let mark = run.events.len();
    run.idle();

    let late: Vec<_> = run
        .events
        .iter()
        .skip(mark)
        .filter(|event| about(event, tab(1)))
        .collect();
    ensure(late.is_empty(), || {
        format!("a closed page still reported {late:?}")
    })?;
    run.wait_visit(2, "late", "Late")?;
    ensure(run.engine.show(WINDOW, tab(1)).is_err(), || {
        "it showed a closed page".to_owned()
    })
}

fn commit_before_title<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "late", Jar::Main)?;

    run.wait_visit(1, "late", "Late")?;

    let first = first_navigation(&run.events, tab(1));
    ensure(matches!(first, Some(EngineEvent::Committed { .. })), || {
        format!("the first navigation report of a page that retitles itself was {first:?}")
    })
}

fn waker_runs<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let woken = Rc::new(Cell::new(0_usize));
    let counter = Rc::clone(&woken);
    H::set_waker(
        &mut run.engine,
        Box::new(move || counter.set(counter.get() + 1)),
    );
    run.create(1, "plain", Jar::Main)?;

    let held = run.harness.pump(&mut run.engine, &mut |_| woken.get() > 0);

    ensure(held, || "the waker never ran".to_owned())?;
    run.wait_visit(1, "plain", "Plain")
}

fn nothing_inside_calls<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let woken = Rc::new(Cell::new(0_usize));
    let first = Rc::clone(&woken);
    H::set_waker(
        &mut run.engine,
        Box::new(move || first.set(first.get() + 1)),
    );
    run.create(1, "late", Jar::Main)?;
    run.create(2, "plain", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    run.engine
        .navigate(tab(2), &Navigation::Reload)
        .map_err(|error| format!("reloading failed: {error}"))?;
    run.engine
        .close(tab(2))
        .map_err(|error| format!("closing failed: {error}"))?;
    same("wakes inside engine calls", &woken.get(), &0)?;
    ensure(
        run.harness.pump(&mut run.engine, &mut |_| woken.get() > 0),
        || "the waker never ran".to_owned(),
    )?;

    let again = Rc::new(Cell::new(0_usize));
    let second = Rc::clone(&again);
    H::set_waker(
        &mut run.engine,
        Box::new(move || second.set(second.get() + 1)),
    );
    same(
        "wakes inside setting a waker while events wait",
        &again.get(),
        &0,
    )?;
    ensure(
        run.harness.pump(&mut run.engine, &mut |_| again.get() > 0),
        || "events waiting before the waker was set never woke it".to_owned(),
    )?;
    run.wait_visit(1, "late", "Late")?;
    ensure(visit(&run.events, tab(2)).is_none(), || {
        "a page closed before its commit was drained still reported it".to_owned()
    })
}

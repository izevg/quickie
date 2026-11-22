//! A page's life (core), what it shows on screen (surface), and how its events reach the shell
//! (events).

use std::cell::Cell;
use std::rc::Rc;

use quickie_domain_browsing::Jar;

use super::{Case, Checked, Run, about, ensure, first_navigation, same, tab, visit};
use crate::conformance::{Group, Harness, WINDOW};
use crate::{DevTools, Engine, EngineEvent, EnginePort, Navigation, PageSurface};

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
            Group::Surface,
            "a_page_has_a_view_until_it_closes",
            view_until_closed,
        ),
        case(
            Group::Surface,
            "a_page_is_loading_until_it_has_loaded",
            loads,
        ),
        case(Group::Surface, "a_captured_page_has_a_picture", pictured),
        case(
            Group::Surface,
            "a_page_says_when_a_text_field_has_focus",
            field_focused,
        ),
        case(
            Group::Surface,
            "a_frame_added_while_a_field_has_focus_keeps_it_focused",
            field_focused_past_frame,
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

fn view_until_closed<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    nothing_on_screen(run, "before its page was created")?;
    run.create(1, "plain", Jar::Main)?;
    ensure(run.engine.view(tab(1)).is_some(), || {
        "a created page has no view".to_owned()
    })?;
    run.wait_visit(1, "plain", "Plain")?;

    run.engine
        .close(tab(1))
        .map_err(|error| format!("closing failed: {error}"))?;

    nothing_on_screen(run, "after its page closed")
}

/// Fails unless tab 1 has nothing on screen: no view, no picture, nothing loading or focused,
/// nothing to print or inspect.
fn nothing_on_screen<H: Harness>(run: &mut Run<'_, H>, when: &str) -> Checked {
    let engine = &mut run.engine;
    ensure(engine.view(tab(1)).is_none(), || {
        format!("a tab has a view {when}")
    })?;
    ensure(engine.picture(tab(1)).is_none(), || {
        format!("a tab has a picture {when}")
    })?;
    same(
        &format!("how far a tab loaded {when}"),
        &engine.loading(tab(1)),
        &None,
    )?;
    ensure(!engine.input_focused(tab(1)), || {
        format!("a tab has a focused text field {when}")
    })?;
    ensure(engine.print(tab(1)).is_err(), || {
        format!("a tab printed {when}")
    })?;
    ensure(engine.inspect(tab(1), DevTools::Toggle).is_err(), || {
        format!("a tab was inspected {when}")
    })
}

fn loads<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;

    let started = run.engine.loading(tab(1));
    ensure(
        started.is_some_and(|fraction| (0.0..=1.0).contains(&fraction)),
        || format!("a page just sent to load says it loaded {started:?}"),
    )?;
    run.wait_visit(1, "plain", "Plain")?;
    let loaded = run.harness.pump(&mut run.engine, &mut |engine| {
        engine.loading(tab(1)).is_none()
    });
    ensure(loaded, || {
        "a loaded page still says it is loading".to_owned()
    })
}

fn pictured<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    run.wait_visit(1, "plain", "Plain")?;
    let mark = run.events.len();

    run.engine.capture(tab(1));

    run.take(
        mark,
        "the page's picture",
        |event| matches!(event, EngineEvent::Pictured { tab: about } if *about == tab(1)),
    )?;
    ensure(run.engine.picture(tab(1)).is_some(), || {
        "a pictured page has no picture".to_owned()
    })?;
    run.engine
        .close(tab(1))
        .map_err(|error| format!("closing failed: {error}"))?;
    ensure(run.engine.picture(tab(1)).is_none(), || {
        "a closed page keeps its picture".to_owned()
    })
}

fn field_focused<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.create(2, "field", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(2))
        .map_err(|error| format!("showing failed: {error}"))?;
    run.wait_visit(1, "plain", "Plain")?;
    run.wait_visit(2, "field", "Field")?;
    click_focus_button(run, 2)?;
    run.wait_title(2, "Focused")?;

    let focused = run
        .harness
        .pump(&mut run.engine, &mut |engine| engine.input_focused(tab(2)));

    ensure(focused, || {
        "a page whose text field has focus does not say so".to_owned()
    })?;
    ensure(!run.engine.input_focused(tab(1)), || {
        "a page without a text field says one has focus".to_owned()
    })
}

fn field_focused_past_frame<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "field-framed", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    run.wait_visit(1, "field-framed", "Field")?;
    click_focus_button(run, 1)?;
    run.wait_title(1, "framed:plain")?;

    run.harness.idle(&mut run.engine);

    ensure(run.engine.input_focused(tab(1)), || {
        "a frame loaded while a text field has focus took the focus report from it".to_owned()
    })
}

fn click_focus_button<H: Harness>(run: &mut Run<'_, H>, serial: u64) -> Checked {
    // An unshown WebKit view can suppress programmatic focus events outside a user gesture.
    let button = run
        .targets(serial)?
        .first()
        .ok_or("the focus button is missing")?
        .frame;
    run.harness
        .click(&mut run.engine, tab(serial), button, false)?
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
    run.engine
        .set_waker(Box::new(move || counter.set(counter.get() + 1)));
    run.create(1, "plain", Jar::Main)?;

    let held = run.harness.pump(&mut run.engine, &mut |_| woken.get() > 0);

    ensure(held, || "the waker never ran".to_owned())?;
    run.wait_visit(1, "plain", "Plain")
}

fn nothing_inside_calls<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let woken = Rc::new(Cell::new(0_usize));
    let first = Rc::clone(&woken);
    run.engine
        .set_waker(Box::new(move || first.set(first.get() + 1)));
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
    run.engine
        .set_waker(Box::new(move || second.set(second.get() + 1)));
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

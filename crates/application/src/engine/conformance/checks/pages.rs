//! What pages do to the browser: open pages and close themselves (opened), keep cookies in their
//! jar (jars), ask the user (dialogs, permissions), go full screen, and play videos (video).

use quickie_domain_browsing::{Focus, Jar, JarId, Video};

use super::{Case, Checked, Run, about, ensure, same, tab};
use crate::conformance::{Group, Harness, WINDOW};
use crate::{
    Answer, Ask, AskKind, Dialog, EngineEvent, EnginePort, Fullscreen, Navigation, PageAsks,
    PageContent, PageVideo, Permission,
};

pub(super) fn cases<H: Harness>() -> Vec<Case<H>> {
    let case = |group, name, run| Case { group, name, run };
    vec![
        case(
            Group::Opened,
            "window_open_hands_over_a_page_to_adopt",
            window_open,
        ),
        case(
            Group::Opened,
            "a_blank_target_link_opens_in_front",
            blank_target,
        ),
        case(
            Group::Opened,
            "a_command_click_opens_in_the_background",
            command_click,
        ),
        case(
            Group::Opened,
            "an_adopted_page_closes_itself",
            closes_itself,
        ),
        case(
            Group::Opened,
            "a_tab_with_a_page_adopts_no_other",
            adopts_into_a_vacant_tab_only,
        ),
        case(
            Group::Jars,
            "a_throwaway_jar_keeps_its_cookies_to_itself",
            jars_are_apart,
        ),
        case(
            Group::Jars,
            "a_popup_shares_its_openers_jar",
            popup_shares_the_jar,
        ),
        case(
            Group::Jars,
            "a_destroyed_jar_forgets_its_cookies",
            destroyed_jar_forgets,
        ),
        case(Group::Dialogs, "an_alert_waits_for_its_answer", alert_waits),
        case(
            Group::Dialogs,
            "a_confirm_hears_ok_or_cancel",
            confirm_hears,
        ),
        case(Group::Dialogs, "a_prompt_hears_the_text", prompt_hears),
        case(
            Group::Dialogs,
            "closing_a_tab_declines_what_its_page_asked",
            closing_declines,
        ),
        case(
            Group::Permissions,
            "a_camera_request_waits_for_its_answer",
            camera_waits,
        ),
        case(
            Group::Fullscreen,
            "a_clicked_page_fills_its_view_in_full_screen",
            fullscreen_fills,
        ),
        case(
            Group::Fullscreen,
            "the_browser_takes_a_page_out_of_full_screen",
            fullscreen_taken_out,
        ),
        case(
            Group::Fullscreen,
            "a_page_leaves_full_screen_by_itself",
            fullscreen_left_by_page,
        ),
        case(
            Group::Fullscreen,
            "a_page_that_navigates_leaves_full_screen",
            fullscreen_left_by_navigation,
        ),
        case(
            Group::Fullscreen,
            "a_page_goes_full_screen_only_on_a_click",
            fullscreen_needs_a_click,
        ),
        case(
            Group::Video,
            "a_video_says_it_plays_pauses_and_leaves",
            video_reports,
        ),
    ]
}

/// Clicks control `index` of tab `serial`'s page the way link hints do.
fn click<H: Harness>(run: &mut Run<'_, H>, serial: u64, index: usize) -> Checked {
    run.targets(serial)?;
    run.engine
        .follow_link(tab(serial), index, false)
        .map_err(|error| format!("clicking in {} failed: {error}", tab(serial)))?;
    Ok(())
}

/// Opens the opener page in tab 1, clicks its control `index`, and adopts what it opened as
/// tab 2: where it is and in what focus it opened.
fn open<H: Harness>(run: &mut Run<'_, H>, index: usize) -> Result<(url::Url, Focus), String> {
    run.create(1, "opener", Jar::Main)?;
    run.wait_visit(1, "opener", "Opener")?;
    let mark = run.events.len();
    click(run, 1, index)?;
    run.adopt_opened(mark, 1, 2)
}

fn window_open<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let (url, focus) = open(run, 2)?;

    same(
        "the opened page",
        &(url, focus),
        &(run.url("popup"), Focus::Foreground),
    )?;
    run.wait_visit(2, "popup", "Popup")
}

fn blank_target<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let (url, focus) = open(run, 3)?;

    same(
        "the opened page",
        &(url, focus),
        &(run.url("popup"), Focus::Foreground),
    )?;
    run.wait_visit(2, "popup", "Popup")
}

fn command_click<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "opener", Jar::Main)?;
    run.wait_visit(1, "opener", "Opener")?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    let link = run
        .targets(1)?
        .get(1)
        .ok_or("the page lost its link")?
        .frame;
    let mark = run.events.len();

    run.harness.click(&mut run.engine, tab(1), link, true)??;

    let (url, focus) = run.adopt_opened(mark, 1, 2)?;
    same(
        "the opened page",
        &(url, focus),
        &(run.url("other"), Focus::Background),
    )?;
    run.wait_visit(2, "other", "Other")?;
    run.idle();
    run.wait_visit(1, "opener", "Opener")
}

fn closes_itself<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    open(run, 2)?;
    run.wait_visit(2, "popup", "Popup")?;

    click(run, 2, 0)?;

    run.wait("the popup closing itself", |events| {
        events.iter().any(
            |event| matches!(event, EngineEvent::ClosedByPage { tab: closed } if *closed == tab(2)),
        )
    })
}

fn adopts_into_a_vacant_tab_only<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "opener", Jar::Main)?;
    run.wait_visit(1, "opener", "Opener")?;
    let mark = run.events.len();
    click(run, 1, 2)?;
    let opened = run.take(mark, "a page opened", |event| {
        matches!(event, EngineEvent::Opened { .. })
    })?;
    let EngineEvent::Opened { page, .. } = opened else {
        return Err("an opened page that is not one".to_owned());
    };

    let adopted = run.engine.adopt(page, tab(1));

    ensure(adopted.is_err(), || {
        "it adopted a page into a tab that has one".to_owned()
    })
}

/// The throwaway jar the jar checks use, and another.
const A: Jar = Jar::Throwaway(JarId::new(1));
const B: Jar = Jar::Throwaway(JarId::new(2));

/// What the cookies page in tab `serial`, in `jar`, sees.
fn cookies<H: Harness>(run: &mut Run<'_, H>, serial: u64, jar: Jar, want: &str) -> Checked {
    run.create(serial, "cookies", jar)?;
    run.wait_title(serial, want)
}

fn jars_are_apart<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "set-cookie", A)?;
    run.wait_title(1, "cookies:jar=a")?;

    cookies(run, 2, Jar::Main, "cookies:")?;
    cookies(run, 3, B, "cookies:")?;
    cookies(run, 4, A, "cookies:jar=a")
}

fn popup_shares_the_jar<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "set-cookie", A)?;
    run.wait_title(1, "cookies:jar=a")?;
    let mark = run.events.len();

    click(run, 1, 0)?;
    run.adopt_opened(mark, 1, 2)?;

    run.wait_title(2, "cookies:jar=a")
}

fn destroyed_jar_forgets<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "set-cookie", A)?;
    run.wait_title(1, "cookies:jar=a")?;
    run.engine
        .close(tab(1))
        .map_err(|error| format!("closing failed: {error}"))?;

    run.engine
        .destroy_jar(JarId::new(1))
        .map_err(|error| format!("destroying the jar failed: {error}"))?;

    cookies(run, 2, A, "cookies:")
}

/// Opens `path` in tab 1 and waits for what it asks.
fn asked<H: Harness>(run: &mut Run<'_, H>, path: &str) -> Result<Ask, String> {
    run.create(1, path, Jar::Main)?;
    let ask = run.take(0, "the page's ask", |event| {
        matches!(event, EngineEvent::Asked(_))
    })?;
    let EngineEvent::Asked(ask) = ask else {
        return Err("an ask that is not one".to_owned());
    };
    same(
        "who asks",
        &(ask.tab, &ask.origin),
        &(tab(1), &run.harness.base()),
    )?;
    Ok(ask)
}

/// Answers `ask`, which must still wait.
fn answer<H: Harness>(run: &mut Run<'_, H>, ask: &Ask, answer: &Answer) -> Checked {
    let waited = run.engine.answer(ask.token, answer);
    ensure(waited, || {
        "the page no longer waited for its answer".to_owned()
    })
}

fn alert_waits<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let ask = asked(run, "alert")?;
    same(
        "the dialog",
        &ask.kind,
        &AskKind::Dialog(Dialog::Alert {
            message: "Hello".to_owned(),
        }),
    )?;
    run.idle();
    run.wait_visit(1, "alert", "Alert")?;

    answer(run, &ask, &Answer::Accept(None))?;

    run.wait_title(1, "Alerted")?;
    let again = run.engine.answer(ask.token, &Answer::Decline);
    ensure(!again, || "an alert took a second answer".to_owned())
}

fn confirm_hears<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let ask = asked(run, "confirm")?;
    same(
        "the dialog",
        &ask.kind,
        &AskKind::Dialog(Dialog::Confirm {
            message: "Sure".to_owned(),
        }),
    )?;

    answer(run, &ask, &Answer::Accept(None))?;

    run.wait_title(1, "confirm:true")?;
    let mark = run.events.len();
    run.engine
        .navigate(tab(1), &Navigation::Reload)
        .map_err(|error| format!("reloading failed: {error}"))?;
    let again = run.take(mark, "the reloaded page's ask", |event| {
        matches!(event, EngineEvent::Asked(_))
    })?;
    let EngineEvent::Asked(again) = again else {
        return Err("an ask that is not one".to_owned());
    };
    ensure(again.token != ask.token, || {
        format!("two asks got one token: {:?}", ask.token)
    })?;
    answer(run, &again, &Answer::Decline)?;
    run.wait_title(1, "confirm:false")
}

fn prompt_hears<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let ask = asked(run, "prompt")?;
    same(
        "the dialog",
        &ask.kind,
        &AskKind::Dialog(Dialog::Prompt {
            message: "Name".to_owned(),
            default: "Ada".to_owned(),
        }),
    )?;

    answer(run, &ask, &Answer::Accept(Some("Grace".to_owned())))?;

    run.wait_title(1, "prompt:Grace")
}

fn closing_declines<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let ask = asked(run, "alert")?;

    run.engine
        .close(tab(1))
        .map_err(|error| format!("closing failed: {error}"))?;
    let mark = run.events.len();
    run.idle();

    let waited = run.engine.answer(ask.token, &Answer::Accept(None));
    ensure(!waited, || {
        "a closed page still waited for its answer".to_owned()
    })?;
    ensure(
        !run.events
            .iter()
            .skip(mark)
            .any(|event| about(event, tab(1))),
        || "a closed page still reported".to_owned(),
    )
}

fn camera_waits<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let ask = asked(run, "camera")?;
    same(
        "the request",
        &ask.kind,
        &AskKind::Permission(Permission::Camera),
    )?;

    answer(run, &ask, &Answer::Decline)?;

    run.wait_title(1, "camera:denied")
}

/// True when what tab `serial`'s page said of its video last is `want`; false before it said
/// anything.
fn said<P>(events: &[EngineEvent<P>], serial: u64, want: Option<Video>) -> bool {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            EngineEvent::Video { tab: about, video } if *about == tab(serial) => {
                Some(*video == want)
            }
            _ => None,
        })
        .unwrap_or(false)
}

/// Waits until tab `serial`'s page last said `video` of its video.
fn wait_video<H: Harness>(run: &mut Run<'_, H>, serial: u64, video: Option<Video>) -> Checked {
    run.wait(
        &format!("{} saying its video is {video:?}", tab(serial)),
        |events| said(events, serial, video),
    )
}

/// The page plays a muted video by itself, which pauses and plays again when asked, and is
/// gone once the page leaves.
fn video_reports<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let playing = Some(Video::Playing { audible: false });
    run.create(1, "video", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    wait_video(run, 1, playing)?;

    run.engine
        .play_video(tab(1), false)
        .map_err(|error| format!("pausing failed: {error}"))?;
    wait_video(run, 1, Some(Video::Paused))?;
    run.engine
        .play_video(tab(1), true)
        .map_err(|error| format!("playing failed: {error}"))?;
    wait_video(run, 1, playing)?;

    let plain = run.url("plain");
    run.engine
        .navigate(tab(1), &Navigation::Load(plain))
        .map_err(|error| format!("navigating failed: {error}"))?;
    wait_video(run, 1, None)
}

/// Waits from `mark` on until tab 1 reports full screen `state`, and takes the report out.
fn fullscreen_state<H: Harness>(run: &mut Run<'_, H>, mark: usize, state: Fullscreen) -> Checked {
    run.take(
        mark,
        &format!("{} in full screen {state:?}", tab(1)),
        |event| {
            matches!(event, EngineEvent::Fullscreen { tab: about, state: now }
            if *about == tab(1) && *now == state)
        },
    )
    .map(|_| ())
}

/// Opens the page at `path` in tab 1, shown in the window, and clicks its button as the user
/// does: the page asks for full screen and fills its view.
fn enter_fullscreen<H: Harness>(run: &mut Run<'_, H>, path: &str) -> Checked {
    run.create(1, path, Jar::Main)?;
    run.wait_visit(1, path, "Full screen")?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    let button = run
        .targets(1)?
        .first()
        .ok_or("the page lost its button")?
        .frame;
    let mark = run.events.len();

    run.harness
        .click(&mut run.engine, tab(1), button, false)??;

    fullscreen_state(run, mark, Fullscreen::InWindow)?;
    run.wait_title(1, "fullscreen:filled")
}

/// No full-screen report of tab 1 from `mark` on.
fn no_fullscreen<H: Harness>(run: &Run<'_, H>, mark: usize, what: &str) -> Checked {
    let reported = run.events.iter().skip(mark).find(
        |event| matches!(event, EngineEvent::Fullscreen { tab: about, .. } if *about == tab(1)),
    );
    ensure(reported.is_none(), || format!("{what}, yet {reported:?}"))
}

fn fullscreen_fills<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    enter_fullscreen(run, "fullscreen")?;

    let mark = run.events.len();
    run.idle();
    no_fullscreen(run, mark, "the page stays full screen")
}

fn fullscreen_taken_out<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    enter_fullscreen(run, "fullscreen")?;
    let mark = run.events.len();

    run.engine.exit_fullscreen(tab(1));

    fullscreen_state(run, mark, Fullscreen::Left)?;
    run.wait_title(1, "fullscreen:none")
}

fn fullscreen_left_by_page<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let mark = run.events.len();

    enter_fullscreen(run, "fullscreen-brief")?;

    fullscreen_state(run, mark, Fullscreen::Left)?;
    run.wait_title(1, "fullscreen:none")
}

fn fullscreen_left_by_navigation<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    enter_fullscreen(run, "fullscreen")?;
    let mark = run.events.len();
    let other = run.url("other");

    run.engine
        .navigate(tab(1), &Navigation::Load(other))
        .map_err(|error| format!("navigating failed: {error}"))?;

    fullscreen_state(run, mark, Fullscreen::Left)?;
    run.wait_visit(1, "other", "Other")
}

fn fullscreen_needs_a_click<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "fullscreen-unasked", Jar::Main)?;
    run.engine
        .show(WINDOW, tab(1))
        .map_err(|error| format!("showing failed: {error}"))?;
    run.wait_title(1, "fullscreen:denied")?;

    run.engine.exit_fullscreen(tab(1));

    run.idle();
    no_fullscreen(run, 0, "the page went full screen on its own")
}

//! What is on a page (find, hints, text), what it fetches (downloads, blocker), and its
//! renderer dying (crash).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use quickie_domain_browsing::{FindMatches, Jar};
use quickie_domain_settings::ZoomLevel;
use quickie_domain_shields::{ContentHash, FilterListId, Site};

use super::{Case, Checked, Run, ensure, same, tab};
use crate::conformance::{Group, Harness, site};
use crate::{
    BlockerRules, DownloadEvent, DownloadToken, Downloads, EngineError, EngineEvent, EnginePort,
    FindStep, Followed, Found, ListRules, Navigation, PageContent, PageZoom, Ticket,
};

pub(super) fn cases<H: Harness>() -> Vec<Case<H>> {
    let case = |group, name, run| Case { group, name, run };
    vec![
        case(
            Group::Find,
            "find_counts_the_matches_and_steps_through_them",
            find_steps,
        ),
        case(
            Group::Find,
            "an_empty_query_or_no_match_finds_nothing",
            find_nothing,
        ),
        case(Group::Find, "clearing_a_find_finds_nothing", find_cleared),
        case(
            Group::Hints,
            "the_targets_are_the_links_and_controls_in_order",
            targets_in_order,
        ),
        case(
            Group::Hints,
            "following_a_link_names_where_it_goes",
            following_a_link,
        ),
        case(
            Group::Hints,
            "following_a_button_clicks_it",
            following_a_button,
        ),
        case(Group::Text, "the_text_is_what_the_page_shows", page_text),
        case(
            Group::Text,
            "the_text_is_cut_to_the_most_asked_for",
            text_cut,
        ),
        case(
            Group::Downloads,
            "an_attachment_downloads_into_the_folder",
            attachment_downloads,
        ),
        case(
            Group::Downloads,
            "a_download_asked_for_downloads_the_same_way",
            asked_download,
        ),
        case(
            Group::Downloads,
            "a_cancelled_download_leaves_no_file",
            cancelled_download,
        ),
        case(
            Group::Downloads,
            "without_a_folder_a_download_is_refused",
            refused_download,
        ),
        case(
            Group::Downloads,
            "a_page_saved_as_a_pdf_is_a_file_in_the_folder",
            saved_pdf,
        ),
        case(Group::Blocker, "a_blocked_script_never_runs", blocked),
        case(
            Group::Blocker,
            "an_unshielded_site_loads_everything",
            unshielded,
        ),
        case(
            Group::Crash,
            "a_crashed_page_is_reported_and_reloads",
            crashed,
        ),
        case(
            Group::Zoom,
            "a_zoomed_page_lays_out_larger_across_navigations",
            zoomed,
        ),
        case(
            Group::Zoom,
            "zooming_a_tab_without_a_page_changes_nothing",
            zoomed_without_a_page,
        ),
    ]
}

/// Finds `query` in tab 1 and waits for the matches.
fn find<H: Harness>(
    run: &mut Run<'_, H>,
    query: &str,
    step: FindStep,
) -> Result<FindMatches, String> {
    matches(run, query, |engine| engine.find(tab(1), query, step))
}

/// Asks the engine for a find in tab 1 with `ask` and waits for the matches of `query`.
fn matches<H: Harness>(
    run: &mut Run<'_, H>,
    query: &str,
    ask: impl FnOnce(&mut H::Engine) -> Result<(), EngineError>,
) -> Result<FindMatches, String> {
    let mark = run.events.len();
    ask(&mut run.engine).map_err(|error| format!("finding {query:?} failed: {error}"))?;
    let found = run.take(
        mark,
        &format!("the matches of {query:?}"),
        |event| matches!(event, EngineEvent::Found(Found { tab: about, .. }) if *about == tab(1)),
    )?;
    match found {
        EngineEvent::Found(found) => {
            same(
                "the query the matches are for",
                &found.query.as_str(),
                &query,
            )?;
            Ok(found.matches)
        }
        other => Err(format!("{other:?} is not a find")),
    }
}

fn find_steps<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let mut steps = Vec::new();
    for step in [
        FindStep::Refine,
        FindStep::Next,
        FindStep::Next,
        FindStep::Next,
        FindStep::Previous,
    ] {
        steps.push(find(run, "alpha", step)?);
    }

    same(
        "the matches of alpha, step by step",
        &steps,
        &[1, 2, 3, 1, 3]
            .map(|current| FindMatches::new(Some(current), 3))
            .to_vec(),
    )
}

fn find_nothing<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let missing = find(run, "zeta", FindStep::Refine)?;
    let next = find(run, "zeta", FindStep::Next)?;
    let empty = find(run, "", FindStep::Refine)?;

    same(
        "the matches of zeta",
        &(missing, next),
        &(FindMatches::NONE, FindMatches::NONE),
    )?;
    same("the matches of nothing", &empty, &FindMatches::NONE)
}

fn find_cleared<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;
    find(run, "alpha", FindStep::Refine)?;

    let cleared = matches(run, "", |engine| engine.clear_find(tab(1)))?;

    same("the matches once cleared", &cleared, &FindMatches::NONE)?;
    let without = run.engine.clear_find(tab(9));
    ensure(without.is_err(), || {
        "a tab without a page cleared a find".to_owned()
    })
}

/// Opens the opener page in tab 1.
fn opener<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "opener", Jar::Main)?;
    run.wait_visit(1, "opener", "Opener")
}

/// Collects tab 1's targets, follows target `index`, and waits for what following it left to
/// do.
fn follow<H: Harness>(
    run: &mut Run<'_, H>,
    index: usize,
    background: bool,
) -> Result<Followed, String> {
    run.targets(1)?;
    let ticket = run
        .engine
        .follow_link(tab(1), index, background)
        .map_err(|error| format!("following target {index} failed: {error}"))?;
    let answer = run.take(0, "what following left to do", |event| {
        matches!(event, EngineEvent::Followed { ticket: answered, .. } if *answered == ticket)
    })?;
    match answer {
        EngineEvent::Followed { followed, .. } => Ok(followed),
        other => Err(format!("{other:?} is not a followed link")),
    }
}

fn targets_in_order<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    opener(run)?;

    let targets = run.targets(1)?;

    same(
        "which targets are links",
        &targets.iter().map(|target| target.link).collect::<Vec<_>>(),
        &vec![true, true, false, false, false],
    )?;
    let sized = targets.iter().all(|target| {
        target.frame.x >= 0.0 && target.frame.width > 0.0 && target.frame.height > 0.0
    });
    let ordered = targets
        .windows(2)
        .all(|pair| matches!(pair, [above, below] if above.frame.y < below.frame.y));
    ensure(sized && ordered, || {
        format!("the targets are not laid out top to bottom in the view: {targets:?}")
    })?;
    let first = run.engine.link_targets(tab(1));
    let second = run.engine.link_targets(tab(1));
    ensure(first != second, || {
        format!("two requests got one ticket: {first:?}")
    })?;
    ensure(run.engine.link_targets(tab(9)).is_err(), || {
        "a tab without a page has link targets".to_owned()
    })
}

fn following_a_link<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let popup = run.url("popup");
    opener(run)?;

    let background = follow(run, 0, true)?;
    let blank = follow(run, 0, false)?;

    same(
        "following a link in the background",
        &background,
        &Followed::Background(popup.clone()),
    )?;
    same(
        "following a link to a new window",
        &blank,
        &Followed::NewTab(popup),
    )
}

fn following_a_button<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    opener(run)?;

    let followed = follow(run, 4, false)?;

    same("following a button", &followed, &Followed::Done)?;
    run.wait_title(1, "Clicked")?;
    ensure(run.engine.follow_link(tab(9), 0, false).is_err(), || {
        "a tab without a page followed a link".to_owned()
    })
}

/// The most text a check reads: far more than any of the suite's pages shows.
const ALL_TEXT: usize = 10_000;

/// Reads at most `max_chars` characters of tab 1's text and waits for them.
fn read<H: Harness>(run: &mut Run<'_, H>, max_chars: usize) -> Result<String, String> {
    let ticket: Ticket = run
        .engine
        .read_text(tab(1), max_chars)
        .map_err(|error| format!("reading the text failed: {error}"))?;
    let answer = run.take(
        0,
        "the page's text",
        |event| matches!(event, EngineEvent::Text { ticket: answered, .. } if *answered == ticket),
    )?;
    let EngineEvent::Text {
        tab: about, text, ..
    } = answer
    else {
        return Err(format!("{answer:?} is not page text"));
    };
    same("whose text", &about, &tab(1))?;
    Ok(text)
}

fn page_text<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let text = read(run, ALL_TEXT)?;

    ensure(text.contains("alpha beta Alpha gamma alpha"), || {
        format!("the page's text is {text:?}")
    })?;
    ensure(run.engine.read_text(tab(9), ALL_TEXT).is_err(), || {
        "a tab without a page has text".to_owned()
    })
}

fn text_cut<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let whole = read(run, ALL_TEXT)?;
    let cut = read(run, 5)?;

    same(
        "the text cut to 5 characters",
        &cut,
        &whole.chars().take(5).collect(),
    )
}

/// The downloads reported so far, oldest first.
fn downloads<P>(events: &[EngineEvent<P>]) -> impl Iterator<Item = &DownloadEvent> {
    events.iter().filter_map(|event| match event {
        EngineEvent::Download(download) => Some(download),
        _ => None,
    })
}

/// Waits for the download of `path` to start, from tab 1's page, into `folder`: its token.
fn started<H: Harness>(
    run: &mut Run<'_, H>,
    path: &str,
    folder: &Path,
) -> Result<DownloadToken, String> {
    run.wait("a download to start", |events| {
        downloads(events).any(|event| matches!(event, DownloadEvent::Started { .. }))
    })?;
    let want = (tab(1), Jar::Main, run.url(path), folder.join(path));
    let started = downloads(&run.events).find_map(|event| match event {
        DownloadEvent::Started {
            token,
            tab,
            jar,
            url,
            file,
        } => Some((*token, (*tab, *jar, url.clone(), file.clone()))),
        _ => None,
    });
    let (token, got) = started.ok_or("the download went missing")?;
    same("the started download", &got, &want)?;
    Ok(token)
}

/// Waits for download `token` to finish and checks the file holds all of `path`.
fn finished<H: Harness>(
    run: &mut Run<'_, H>,
    token: DownloadToken,
    path: &str,
    folder: &Path,
) -> Checked {
    run.wait("the download to end", |events| {
        downloads(events).any(|event| {
            event.token() == token
                && matches!(
                    event,
                    DownloadEvent::Finished { .. } | DownloadEvent::Failed { .. }
                )
        })
    })?;
    ensure(
        downloads(&run.events).any(|event| *event == DownloadEvent::Finished { token }),
        || "the download failed".to_owned(),
    )?;
    let want = site::resource(path).map(|resource| resource.body().into_bytes());
    let file = run.harness.downloaded(&run.engine, &folder.join(path))?;
    same("the downloaded file", &file, &want)
}

/// Makes a new folder the engine's downloads folder.
fn downloads_into<H: Harness>(run: &mut Run<'_, H>) -> Result<PathBuf, String> {
    let folder = run.harness.new_folder()??;
    run.engine.set_directory(folder.clone());
    Ok(folder)
}

fn attachment_downloads<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let folder = downloads_into(run)?;
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let attachment = run.url("report.bin");
    run.engine
        .navigate(tab(1), &Navigation::Load(attachment))
        .map_err(|error| format!("loading the attachment failed: {error}"))?;

    let token = started(run, "report.bin", &folder)?;
    finished(run, token, "report.bin", &folder)?;
    run.wait_visit(1, "plain", "Plain")
}

fn asked_download<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let folder = downloads_into(run)?;
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let attachment = run.url("report.bin");
    run.engine
        .download(tab(1), &attachment)
        .map_err(|error| format!("downloading failed: {error}"))?;

    let token = started(run, "report.bin", &folder)?;
    finished(run, token, "report.bin", &folder)?;
    let without = run.engine.download(tab(9), &attachment);
    ensure(without.is_err(), || {
        "a tab without a page downloaded".to_owned()
    })
}

fn cancelled_download<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let folder = downloads_into(run)?;
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;
    let stalled = run.url("stalled.bin");
    run.engine
        .download(tab(1), &stalled)
        .map_err(|error| format!("downloading failed: {error}"))?;
    let token = started(run, "stalled.bin", &folder)?;

    run.engine.cancel(token);
    let mark = run.events.len();
    run.idle();

    let after: Vec<_> = downloads(run.events.get(mark..).unwrap_or_default())
        .filter(|event| event.token() == token)
        .collect();
    ensure(after.is_empty(), || {
        format!("a cancelled download still reported {after:?}")
    })?;
    let file = run
        .harness
        .downloaded(&run.engine, &folder.join("stalled.bin"))?;
    same("the cancelled download's file", &file, &None)
}

fn refused_download<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    let attachment = run.url("report.bin");
    run.engine
        .navigate(tab(1), &Navigation::Load(attachment.clone()))
        .map_err(|error| format!("loading the attachment failed: {error}"))?;
    run.engine
        .download(tab(1), &attachment)
        .map_err(|error| format!("downloading failed: {error}"))?;

    run.wait("both downloads to be refused", |events| {
        refused(events).len() == 2
    })?;
    let tokens = refused(&run.events);
    ensure(
        matches!(tokens.as_slice(), [first, second] if first != second),
        || format!("two refused downloads have the tokens {tokens:?}"),
    )
}

fn saved_pdf<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let folder = downloads_into(run)?;
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    run.engine
        .save_pdf(tab(1))
        .map_err(|error| format!("saving a PDF failed: {error}"))?;

    run.wait("the PDF to be saved or refused", |events| {
        downloads(events).any(|event| {
            matches!(
                event,
                DownloadEvent::Saved { .. } | DownloadEvent::Refused { .. }
            )
        })
    })?;
    let saved = downloads(&run.events).find_map(|event| match event {
        DownloadEvent::Saved {
            tab,
            jar,
            url,
            file,
            ..
        } => Some(((*tab, *jar, url.clone()), file.clone())),
        _ => None,
    });
    let (page, file) = saved.ok_or_else(|| {
        format!(
            "the PDF was refused: {:?}",
            downloads(&run.events).collect::<Vec<_>>()
        )
    })?;
    same(
        "the saved page",
        &page,
        &(tab(1), Jar::Main, run.url("plain")),
    )?;
    ensure(file.parent() == Some(folder.as_path()), || {
        format!("{} is not in {}", file.display(), folder.display())
    })?;
    let written = run.harness.downloaded(&run.engine, &file)?;
    ensure(
        written.is_some_and(|bytes| bytes.starts_with(b"%PDF")),
        || format!("{} holds no PDF", file.display()),
    )?;

    run.engine
        .save_pdf(tab(1))
        .map_err(|error| format!("saving a second PDF failed: {error}"))?;
    run.wait("the second PDF to be saved", |events| {
        saved_files(events).len() == 2
    })?;
    let tokens = saved_files(&run.events);
    ensure(
        matches!(tokens.as_slice(), [first, second] if first != second),
        || format!("two saved PDFs have the tokens {tokens:?}"),
    )?;
    ensure(run.engine.save_pdf(tab(9)).is_err(), || {
        "a tab without a page was saved".to_owned()
    })
}

/// The files made from pages so far.
fn saved_files<P>(events: &[EngineEvent<P>]) -> Vec<DownloadToken> {
    downloads(events)
        .filter(|event| matches!(event, DownloadEvent::Saved { .. }))
        .map(DownloadEvent::token)
        .collect()
}

/// The downloads refused so far.
fn refused<P>(events: &[EngineEvent<P>]) -> Vec<DownloadToken> {
    downloads(events)
        .filter(|event| matches!(event, DownloadEvent::Refused { .. }))
        .map(DownloadEvent::token)
        .collect()
}

/// Rules from one list that blocks the tracker, unshielding `unshielded`.
fn rules(unshielded: Vec<Site>) -> BlockerRules {
    let list: &[u8] = b"! the conformance suite's list\n/tracker.js\n";
    BlockerRules {
        lists: vec![ListRules {
            list: FilterListId::EasyPrivacy,
            hash: ContentHash::of(list),
            body: Arc::from(list),
        }],
        unshielded,
    }
}

/// The tracked page's title when every script ran.
const EVERY_SCRIPT: &str = "scripts:app.js;tracker.js;";

/// The tracked page's title when the tracker was blocked.
const NO_TRACKER: &str = "scripts:app.js;";

fn blocked<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "tracked", Jar::Main)?;
    run.wait_title(1, EVERY_SCRIPT)?;

    run.harness.block(&mut run.engine, &rules(Vec::new()))??;
    run.engine
        .navigate(tab(1), &Navigation::Reload)
        .map_err(|error| format!("reloading failed: {error}"))?;

    run.wait_title(1, NO_TRACKER)
}

fn unshielded<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let site = Site::try_from(&run.harness.base()).map_err(|error| error.to_string())?;
    run.harness.block(&mut run.engine, &rules(Vec::new()))??;
    run.create(1, "tracked", Jar::Main)?;
    run.wait_title(1, NO_TRACKER)?;

    run.harness.block(&mut run.engine, &rules(vec![site]))??;
    run.engine
        .navigate(tab(1), &Navigation::Reload)
        .map_err(|error| format!("reloading failed: {error}"))?;

    run.wait_title(1, EVERY_SCRIPT)
}

/// Waits for tab `serial`'s page, from `mark` on, to say how wide its viewport is.
fn width<H: Harness>(run: &mut Run<'_, H>, mark: usize, serial: u64) -> Result<f64, String> {
    let event = run.take(mark, &format!("{}'s width", tab(serial)), |event| {
        measured(event, serial).is_some()
    })?;
    let width = measured(&event, serial).unwrap_or_default();
    width
        .parse()
        .map_err(|_| format!("{width:?} is not a width"))
}

/// The width tab `serial`'s page says it is, when `event` is it saying so.
fn measured<P>(event: &EngineEvent<P>, serial: u64) -> Option<&str> {
    match event {
        EngineEvent::Committed {
            tab: about, title, ..
        }
        | EngineEvent::Titled {
            tab: about, title, ..
        } if *about == tab(serial) => title.strip_prefix("width:"),
        _ => None,
    }
}

/// Navigates tab 1 as `navigation` says and waits for the measured page's width.
fn remeasure<H: Harness>(run: &mut Run<'_, H>, navigation: &Navigation) -> Result<f64, String> {
    let mark = run.events.len();
    run.engine
        .navigate(tab(1), navigation)
        .map_err(|error| format!("navigating failed: {error}"))?;
    width(run, mark, 1)
}

/// Fails unless `got` is `want`, to a pixel's rounding.
fn about_as_wide(what: &str, got: f64, want: f64) -> Checked {
    ensure((got - want).abs() <= 1.0, || {
        format!("{what}: expected {want} CSS pixels wide, the page was {got}")
    })
}

fn zoomed<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let twice = ZoomLevel::from_percent(200).ok_or("200 % is no zoom level")?;
    run.create(1, "measured", Jar::Main)?;
    run.create(2, "measured", Jar::Main)?;
    let actual = width(run, 0, 1)?;
    width(run, 0, 2)?;

    run.engine.set_zoom(tab(1), twice);
    let mark = run.events.len();
    for serial in [1, 2] {
        run.engine
            .navigate(tab(serial), &Navigation::Reload)
            .map_err(|error| format!("reloading failed: {error}"))?;
    }
    // The other tab first: its page keeps its own zoom, whatever the zoomed one reports.
    let other = width(run, mark, 2)?;
    about_as_wide("another tab", other, actual)?;
    let zoomed = width(run, mark, 1)?;
    about_as_wide("at 200 %", zoomed, actual / 2.0)?;

    remeasure_elsewhere(run)?;
    let back = remeasure(run, &Navigation::Load(run.url("measured")))?;
    about_as_wide("at 200 % after going elsewhere", back, actual / 2.0)?;

    run.engine.set_zoom(tab(1), ZoomLevel::DEFAULT);
    let reset = remeasure(run, &Navigation::Reload)?;
    about_as_wide("back at 100 %", reset, actual)
}

/// Takes tab 1 to another page and waits until it shows.
fn remeasure_elsewhere<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.engine
        .navigate(tab(1), &Navigation::Load(run.url("other")))
        .map_err(|error| format!("loading another page failed: {error}"))?;
    run.wait_visit(1, "other", "Other")
}

fn zoomed_without_a_page<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    let twice = ZoomLevel::from_percent(200).ok_or("200 % is no zoom level")?;
    run.engine.set_zoom(tab(1), twice);
    run.create(1, "measured", Jar::Main)?;
    run.create(2, "measured", Jar::Main)?;

    let first = width(run, 0, 1)?;
    let second = width(run, 0, 2)?;
    about_as_wide("a page zoomed before it existed", first, second)
}

fn crashed<H: Harness>(run: &mut Run<'_, H>) -> Checked {
    run.create(1, "plain", Jar::Main)?;
    run.wait_visit(1, "plain", "Plain")?;

    run.harness.crash(&mut run.engine, tab(1))??;

    run.wait("the crash", |events| {
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Crashed { tab: about } if *about == tab(1)))
    })?;
    let mark = run.events.len();
    run.engine
        .navigate(tab(1), &Navigation::Reload)
        .map_err(|error| format!("reloading failed: {error}"))?;
    run.wait("the reload's commit", |events| {
        events.iter().skip(mark).any(
            |event| matches!(event, EngineEvent::Committed { tab: about, .. } if *about == tab(1)),
        )
    })?;
    run.wait_visit(1, "plain", "Plain")
}

//! End to end on real `WKWebView`s against a server on this machine. Tiny local filter lists,
//! never a real one, and no request leaves the machine.
//!
//! - Requests: a list compiled while a page is open blocks on a page made after it and on the
//!   open page once it reloads; the blocked loads are counted; a site with shields off loads
//!   everything; compiled rules are found again by a new blocker.
//! - Cosmetics and scriptlets: a page that mimics a video site's player response
//!   (`ytInitialPlayerResponse` with `adPlacements`, a `JSON.parse`d response with `playerAds`,
//!   a fetched `/player` response) has its ad fields pruned before its own scripts read them,
//!   its ad elements hidden (host rules, generic rules, generic class rules, a procedural rule),
//!   and nothing of Quickie's is reachable from the page's world.
//! - Shorts: a page that mimics the Shorts feed of a video site loses the ad reel from its
//!   `reel_watch_sequence` answers (fetched, through `XMLHttpRequest`, and parsed) by the lists'
//!   own `[-]` rules, keeps the other reels, and hides a reel with an ad slot inserted later.
//!
//! `WebKit` wants the main thread and its run loop, so this test has its own `main` instead of
//! the test harness, whose tests run on worker threads.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2_foundation::{NSDate, NSRunLoop};
use quickie_application::{
    BlockerRules, ContentBlockerPort, EngineEvent, EnginePort, HttpError, HttpPort, ListRules,
    Navigation,
};
use quickie_domain_browsing::{Jar, Profile, TabId};
use quickie_domain_shields::{ContentHash, FilterListId, Site};
use quickie_engine_webkit::{BlockerStatus, UrlSessionHttp, WebKitContentBlocker, WebKitEngine};
use url::Url;

type Outcome = Result<(), String>;

/// One test: runs on the main thread, says why it failed.
type Case = fn(MainThreadMarker) -> Outcome;

/// The request list: blocks one script by its path, the way EasyPrivacy names trackers.
const LIST: &str = "! Title: local test list\n/tracker.js\n";

/// The cosmetic list, in uBlock Origin's syntax, as uBlock filters writes it for video sites.
const VIDEO_LIST: &str = "! Title: local video list\n\
    localhost##+js(set, ytInitialPlayerResponse.adPlacements, undefined)\n\
    localhost##+js(json-prune, playerAds adSlots)\n\
    localhost##+js(json-prune-fetch-response, adPlacements, , propsToMatch, /player)\n\
    localhost##.host-ad\n\
    localhost##div:has-text(Sponsored)\n\
    ##div[data-generic-ad]\n\
    ##.banner-ad\n";

/// The page: a tracker script, then a request that tells the page was parsed past it.
const PAGE: &str = "<!doctype html><title>t</title><script src=\"/tracker.js\"></script>\
                    <script>fetch('/done')</script>";

/// A page that mimics a video site: its player response arrives inline, parsed, and fetched;
/// once the cosmetic filters had time, it reports what its own scripts saw.
const WATCH: &str = r#"<!doctype html><title>watch</title>
<div class="host-ad" id="a">ad</div>
<div data-generic-ad id="b">ad</div>
<div class="banner-ad" id="c">ad</div>
<div id="d">Sponsored</div>
<div id="e">the video</div>
<script>
var ytInitialPlayerResponse = {adPlacements: [1], videoDetails: {id: "v"}};
const initial = ytInitialPlayerResponse.adPlacements === undefined ? "pruned" : "kept";
const parsed = JSON.parse('{"playerAds":[1],"adSlots":[2],"keep":3}');
fetch("/player").then((r) => r.json()).then((fetched) => {
  setTimeout(() => {
    const shown = (id) => getComputedStyle(document.getElementById(id)).display;
    const report = {
      initial,
      parsed: Object.keys(parsed).join(","),
      fetched: Object.keys(fetched).join(","),
      a: shown("a"), b: shown("b"), c: shown("c"), d: shown("d"), e: shown("e"),
      library: typeof quickieScriptlet,
      handler: typeof (window.webkit && window.webkit.messageHandlers &&
        window.webkit.messageHandlers.quickieShields),
    };
    fetch("/report?" + encodeURIComponent(JSON.stringify(report)));
  }, 800);
});
</script>"#;

/// What the watch page must report.
const WATCHED: &str = r#"{"initial":"pruned","parsed":"keep","fetched":"videoDetails","a":"none","b":"none","c":"none","d":"none","e":"block","library":"undefined","handler":"undefined"}"#;

/// The Shorts feed's ad rules, as uBlock filters and `EasyList` write them for youtube.com, on
/// localhost; the XHR and `{-}` rules are not in the lists but use the same syntax.
const SHORTS_LIST: &str = "! Title: local shorts list\n\
    localhost##+js(json-prune-fetch-response, \
    reelWatchSequenceResponse.entries.[-].command.reelWatchEndpoint.adClientParams.isAd \
    entries.[-].command.reelWatchEndpoint.adClientParams.isAd, , propsToMatch, \
    url:/reel_watch_sequence?)\n\
    localhost##+js(json-prune-xhr-response, \
    entries.[-].command.reelWatchEndpoint.adClientParams.isAd, , propsToMatch, \
    url:/reel_watch_sequence?)\n\
    localhost##+js(json-prune, entries.[-].command.reelWatchEndpoint.adClientParams.isAd)\n\
    localhost##+js(json-prune, slots.{-}.adSlotRenderer)\n\
    localhost###shorts-inner-container > .ytd-shorts:has(> .ytd-reel-video-renderer > ytd-ad-slot-renderer)\n";

/// A `reel_watch_sequence` answer in the shape the video site sends it: three reels, the second an ad.
/// Hand-written, not recorded.
const REELS: &str = r#"{"entries":[{"command":{"reelWatchEndpoint":{"videoId":"v1"}}},{"command":{"reelWatchEndpoint":{"videoId":"v2","adClientParams":{"isAd":true}}}},{"command":{"reelWatchEndpoint":{"videoId":"v3"}}}],"continuationEndpoint":{"token":"t"}}"#;

/// The same answer as some clients get it, wrapped in `reelWatchSequenceResponse`.
const WRAPPED_REELS: &str = r#"{"reelWatchSequenceResponse":{"entries":[{"command":{"reelWatchEndpoint":{"videoId":"v1"}}},{"command":{"reelWatchEndpoint":{"videoId":"v2","adClientParams":{"isAd":true}}}},{"command":{"reelWatchEndpoint":{"videoId":"v3"}}}]}}"#;

/// A page that mimics the Shorts feed: it fetches the next reels with `fetch` and
/// `XMLHttpRequest` and reads them as text, parses them with `JSON.parse`, and inserts reels,
/// one with an ad slot, after it loaded. It reports the reels it got and which it shows.
const SHORTS: &str = r#"<!doctype html><title>shorts</title>
<div id="shorts-inner-container"></div>
<script>
const sequence = "/youtubei/v1/reel/reel_watch_sequence?prettyPrint=false";
const ids = (text) => [...text.matchAll(/"videoId":"(\w+)"/g)].map((m) => m[1]).join(",");
const fetched = fetch(new Request(sequence)).then((r) => r.text());
const wrapped = fetch(sequence + "&wrapped=1").then((r) => r.text());
const xhr = new Promise((resolve) => {
  const request = new XMLHttpRequest();
  request.open("GET", sequence);
  request.onload = () => resolve(request.responseText);
  request.send();
});
const parsed = JSON.parse('{"entries":[{"command":{"reelWatchEndpoint":{"videoId":"v1"}}},' +
  '{"command":{"reelWatchEndpoint":{"videoId":"v2","adClientParams":{"isAd":true}}}}],' +
  '"slots":{"a":{"adSlotRenderer":{}},"b":{"reel":{}}}}');
setTimeout(() => {
  document.getElementById("shorts-inner-container").innerHTML =
    '<div class="ytd-shorts" id="r1"><div class="ytd-reel-video-renderer">video</div></div>' +
    '<div class="ytd-shorts" id="r2"><div class="ytd-reel-video-renderer">' +
    '<ytd-ad-slot-renderer>ad</ytd-ad-slot-renderer></div></div>';
}, 100);
Promise.all([fetched, wrapped, xhr]).then(([fetched, wrapped, xhr]) => {
  setTimeout(() => {
    const shown = (id) => getComputedStyle(document.getElementById(id)).display;
    const report = {
      fetched: ids(fetched), wrapped: ids(wrapped), xhr: ids(xhr),
      parsed: parsed.entries.map((e) => e.command.reelWatchEndpoint.videoId).join(","),
      length: parsed.entries.length, slots: Object.keys(parsed.slots).join(","),
      r1: shown("r1"), r2: shown("r2"),
    };
    fetch("/report?" + encodeURIComponent(JSON.stringify(report)));
  }, 800);
});
</script>"#;

/// What the Shorts page must report: the ad reel is gone from every answer, the others stay,
/// and the inserted reel with an ad slot is hidden.
const SHORTS_SEEN: &str = r#"{"fetched":"v1,v3","wrapped":"v1,v3","xhr":"v1,v3","parsed":"v1","length":1,"slots":"b","r1":"block","r2":"none"}"#;

/// Requests the local server saw, by path and query.
type Seen = Arc<Mutex<Vec<String>>>;

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return report("not on the main thread", Err("WebKit needs it".to_owned()));
    };
    let cases: [(&str, Case); 6] = [
        (
            "a_list_compiled_while_a_page_is_open_blocks_on_new_and_reloaded_pages",
            blocks_on_new_and_reloaded_pages,
        ),
        (
            "an_unshielded_site_loads_everything",
            unshielded_site_loads_everything,
        ),
        (
            "compiled_rules_are_found_again_by_a_new_blocker",
            rules_are_reused,
        ),
        (
            "a_video_page_s_ads_are_pruned_and_hidden_before_its_scripts_see_them",
            video_page_ads_are_pruned_and_hidden,
        ),
        (
            "a_shorts_feed_s_ad_reels_are_removed_from_its_answers_and_hidden",
            shorts_feed_ads_are_removed,
        ),
        (
            "a_request_on_the_main_thread_is_refused",
            main_thread_refused,
        ),
    ];
    let mut code = ExitCode::SUCCESS;
    for (name, case) in cases {
        if report(name, case(mtm)).ne(&ExitCode::SUCCESS) {
            code = ExitCode::FAILURE;
        }
    }
    code
}

#[expect(
    clippy::print_stdout,
    reason = "the test runner's own report, like the harness's"
)]
fn report(name: &str, outcome: Outcome) -> ExitCode {
    match outcome {
        Ok(()) => {
            println!("test {name} ... ok");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            println!("test {name} ... FAILED: {reason}");
            ExitCode::FAILURE
        }
    }
}

fn blocks_on_new_and_reloaded_pages(mtm: MainThreadMarker) -> Outcome {
    let (base, seen) = serve("127.0.0.1")?;
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut engine = WebKitEngine::new(&[], mtm);
    let mut blocker = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;
    let (open, new) = (TabId::new(Profile::Main, 1), TabId::new(Profile::Main, 2));
    let page = base.join("page.html").map_err(|error| error.to_string())?;

    engine
        .create(open, &page, Jar::Main)
        .map_err(|error| error.to_string())?;
    wait_for(&seen, 1)?;
    let before_rules = count(&seen, "/tracker.js");
    install(&mut blocker, &rules(LIST, &[]))?;
    engine
        .create(new, &page, Jar::Main)
        .map_err(|error| error.to_string())?;
    wait_for(&seen, 2)?;
    let on_new = count(&seen, "/tracker.js") - before_rules;
    engine
        .navigate(open, &Navigation::Reload)
        .map_err(|error| error.to_string())?;
    wait_for(&seen, 3)?;
    let on_reload = count(&seen, "/tracker.js") - before_rules;
    let counted = blocked_counts(&mut engine, &[open, new])?;

    check(before_rules == 1, "the open page loaded before any rule")?;
    check(
        on_new == 0,
        "the tracker loaded on a page made after the rules",
    )?;
    check(
        on_reload == 0,
        "the tracker loaded on the open page after a reload",
    )?;
    check(
        counted == [1, 1],
        &format!("blocked loads counted {counted:?}, not one per page"),
    )
}

fn unshielded_site_loads_everything(mtm: MainThreadMarker) -> Outcome {
    let (base, seen) = serve("127.0.0.1")?;
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut engine = WebKitEngine::new(&[], mtm);
    let mut blocker = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;
    let tab = TabId::new(Profile::Main, 1);

    install(&mut blocker, &rules(LIST, &["127.0.0.1"]))?;
    let page = base.join("page.html").map_err(|error| error.to_string())?;
    engine
        .create(tab, &page, Jar::Main)
        .map_err(|error| error.to_string())?;
    wait_for(&seen, 1)?;

    check(
        count(&seen, "/tracker.js") == 1,
        "the tracker stayed blocked with shields off",
    )
}

fn rules_are_reused(mtm: MainThreadMarker) -> Outcome {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let engine = WebKitEngine::new(&[], mtm);
    let mut first = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;
    let compiled = install(&mut first, &rules(LIST, &[]))?;
    let mut second = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;

    let found = install(&mut second, &rules(LIST, &[]))?;

    check(
        found == compiled,
        &format!("the second blocker found {found:?}, not {compiled:?}"),
    )
}

fn video_page_ads_are_pruned_and_hidden(mtm: MainThreadMarker) -> Outcome {
    let report = page_report(mtm, VIDEO_LIST, "watch.html")?;

    check(
        report == WATCHED,
        &format!("the page saw {report}, not {WATCHED}"),
    )
}

fn shorts_feed_ads_are_removed(mtm: MainThreadMarker) -> Outcome {
    let report = page_report(mtm, SHORTS_LIST, "shorts.html")?;

    check(
        report == SHORTS_SEEN,
        &format!("the page saw {report}, not {SHORTS_SEEN}"),
    )
}

/// Opens `page` on localhost with the cosmetic `list`, as uBlock filters (a trusted list), and
/// returns what the page reported.
fn page_report(mtm: MainThreadMarker, list: &str, page: &str) -> Result<String, String> {
    let (base, seen) = serve("localhost")?;
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut engine = WebKitEngine::new(&[], mtm);
    let mut blocker = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;
    let tab = TabId::new(Profile::Main, 1);

    let mut cosmetic = rules(list, &[]);
    cosmetic.lists[0].list = FilterListId::UblockFilters;
    blocker
        .install(&cosmetic)
        .map_err(|error| error.to_string())?;
    spin_until(|| blocker.cosmetics_ready())?;
    let url = base.join(page).map_err(|error| error.to_string())?;
    engine
        .create(tab, &url, Jar::Main)
        .map_err(|error| error.to_string())?;
    spin_until(|| reported(&seen).is_some())?;
    Ok(reported(&seen).unwrap_or_default())
}

fn main_thread_refused(_mtm: MainThreadMarker) -> Outcome {
    let url = Url::parse("http://127.0.0.1:9/").map_err(|error| error.to_string())?;

    let answer = UrlSessionHttp::new().get(&url);

    check(
        answer
            == Err(HttpError(
                "a request never runs on the main thread".to_owned(),
            )),
        "a request ran on the main thread",
    )
}

fn check(holds: bool, otherwise: &str) -> Outcome {
    if holds {
        Ok(())
    } else {
        Err(otherwise.to_owned())
    }
}

fn rules(list: &str, unshielded: &[&str]) -> BlockerRules {
    BlockerRules {
        lists: vec![ListRules {
            list: FilterListId::EasyPrivacy,
            hash: ContentHash::of(list.as_bytes()),
            body: Arc::from(list.as_bytes()),
        }],
        unshielded: unshielded
            .iter()
            .filter_map(|site| Site::try_from(*site).ok())
            .collect(),
    }
}

/// Installs `rules`, spins the main run loop until the blocker settles, and returns what it
/// installed.
fn install(
    blocker: &mut WebKitContentBlocker,
    rules: &BlockerRules,
) -> Result<quickie_engine_webkit::Installed, String> {
    blocker.install(rules).map_err(|error| error.to_string())?;
    spin_until(|| blocker.status() != BlockerStatus::Compiling)?;
    match blocker.status() {
        BlockerStatus::Installed(installed) if installed.lists == 1 && installed.rules > 0 => {
            Ok(installed)
        }
        other => Err(format!("the blocker ended {other:?}")),
    }
}

/// The blocked loads the engine reported for each tab, in order.
fn blocked_counts(engine: &mut WebKitEngine, tabs: &[TabId]) -> Result<Vec<u32>, String> {
    let mut counts = vec![0; tabs.len()];
    let mut events = Vec::new();
    // The callbacks arrive with the loads; give the last one a moment.
    let settle = Instant::now() + Duration::from_millis(300);
    spin_until(|| Instant::now() > settle)?;
    engine.drain(&mut events);
    for event in events {
        if let EngineEvent::Blocked { tab, requests } = event
            && let Some(at) = tabs.iter().position(|known| *known == tab)
        {
            counts[at] += requests;
        }
    }
    Ok(counts)
}

/// Spins until the page was parsed `times` times: its `/done` request arrived.
fn wait_for(seen: &Seen, times: usize) -> Outcome {
    spin_until(|| count(seen, "/done") >= times)
}

fn count(seen: &Seen, path: &str) -> usize {
    seen.lock()
        .map_or(0, |seen| seen.iter().filter(|seen| *seen == path).count())
}

/// The watch page's report, decoded, once it arrived.
fn reported(seen: &Seen) -> Option<String> {
    let seen = seen.lock().ok()?;
    let query = seen.iter().find_map(|path| path.strip_prefix("/report?"))?;
    let url = Url::parse(&format!("http://localhost/?{query}")).ok()?;
    url.query_pairs().next().map(|(key, _)| key.into_owned())
}

/// Runs the main run loop, which also drains the main dispatch queue, until `done` holds.
fn spin_until(done: impl Fn() -> bool) -> Outcome {
    let give_up = Instant::now() + Duration::from_secs(60);
    while !done() {
        if Instant::now() > give_up {
            return Err("timed out".to_owned());
        }
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
    }
    Ok(())
}

/// A server on 127.0.0.1 that answers every request and records its path and query; pages
/// are addressed by `host`, which must resolve to it.
fn serve(host: &str) -> Result<(Url, Seen), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let log = Arc::clone(&log);
            thread::spawn(move || answer(stream, &log));
        }
    });
    let base = Url::parse(&format!("http://{host}:{port}/")).map_err(|error| error.to_string())?;
    Ok((base, seen))
}

fn answer(stream: TcpStream, log: &Seen) {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
        header.clear();
    }
    let target = request
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let path = target.split('?').next().unwrap_or_default().to_owned();
    let (kind, body) = match path.as_str() {
        "/page.html" => ("text/html", PAGE),
        "/watch.html" => ("text/html", WATCH),
        "/shorts.html" => ("text/html", SHORTS),
        "/youtubei/v1/reel/reel_watch_sequence" if target.contains("wrapped") => {
            ("application/json", WRAPPED_REELS)
        }
        "/youtubei/v1/reel/reel_watch_sequence" => ("application/json", REELS),
        "/tracker.js" => ("text/javascript", "window.tracked = true;"),
        "/player" => (
            "application/json",
            r#"{"adPlacements":[1],"videoDetails":{"id":"v"}}"#,
        ),
        _ => ("text/plain", ""),
    };
    if let Ok(mut seen) = log.lock() {
        seen.push(if path == "/report" { target } else { path });
    }
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
}

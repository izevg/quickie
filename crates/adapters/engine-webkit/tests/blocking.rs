//! End to end on real `WKWebView`s: a page on 127.0.0.1 asks for a tracker script, a tiny local
//! filter list blocks it once compiled, and the site's shields going off lets it through. No
//! request leaves the machine, and no real filter list is fetched.
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
    BlockerRules, ContentBlockerPort, EnginePort, HttpError, HttpPort, ListRules, Navigation,
};
use quickie_domain_browsing::{Jar, TabId};
use quickie_domain_shields::{ContentHash, FilterListId, Site};
use quickie_engine_webkit::{BlockerStatus, UrlSessionHttp, WebKitContentBlocker, WebKitEngine};
use url::Url;

type Outcome = Result<(), String>;

/// One test: runs on the main thread, says why it failed.
type Case = fn(MainThreadMarker) -> Outcome;

/// The test list: blocks one script by its path, the way EasyPrivacy names trackers.
const LIST: &str = "! Title: local test list\n/tracker.js\n";

/// The page: a tracker script, then a request that tells the page was parsed past it.
const PAGE: &str = "<!doctype html><title>t</title><script src=\"/tracker.js\"></script>\
                    <script>fetch('/done')</script>";

/// Requests the local server saw, by path.
type Seen = Arc<Mutex<Vec<String>>>;

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return report("not on the main thread", Err("WebKit needs it".to_owned()));
    };
    let cases: [(&str, Case); 3] = [
        (
            "a_compiled_list_blocks_the_tracker_until_the_site_is_unshielded",
            blocks_until_unshielded,
        ),
        (
            "compiled_rules_are_found_again_by_a_new_blocker",
            rules_are_reused,
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

fn blocks_until_unshielded(mtm: MainThreadMarker) -> Outcome {
    let (base, seen) = serve()?;
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut engine = WebKitEngine::new(&[], mtm);
    let mut blocker = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;
    let tab = TabId::new(1);

    install(&mut blocker, &rules(&[]))?;
    let page = base.join("page.html").map_err(|error| error.to_string())?;
    engine
        .create(tab, &page, Jar::Main)
        .map_err(|error| error.to_string())?;
    wait_for(&seen, 1)?;
    let shielded = count(&seen, "/tracker.js");
    install(&mut blocker, &rules(&["127.0.0.1"]))?;
    engine
        .navigate(tab, &Navigation::Reload)
        .map_err(|error| error.to_string())?;
    wait_for(&seen, 2)?;
    let unshielded = count(&seen, "/tracker.js");

    check(shielded == 0, "the tracker loaded while shields were up")?;
    check(
        unshielded == 1,
        "the tracker stayed blocked with shields off",
    )
}

fn rules_are_reused(mtm: MainThreadMarker) -> Outcome {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let engine = WebKitEngine::new(&[], mtm);
    let mut first = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;
    install(&mut first, &rules(&[]))?;
    let mut second = engine
        .content_blocker(dir.path())
        .map_err(|error| error.to_string())?;

    install(&mut second, &rules(&[]))?;

    check(
        second.status() == BlockerStatus::Installed(1),
        "the second blocker did not install",
    )
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

fn rules(unshielded: &[&str]) -> BlockerRules {
    BlockerRules {
        lists: vec![ListRules {
            list: FilterListId::EasyPrivacy,
            hash: ContentHash::of(LIST.as_bytes()),
            body: Arc::from(LIST.as_bytes()),
        }],
        unshielded: unshielded
            .iter()
            .filter_map(|site| Site::try_from(*site).ok())
            .collect(),
    }
}

/// Installs `rules` and spins the main run loop until the blocker settles.
fn install(blocker: &mut WebKitContentBlocker, rules: &BlockerRules) -> Outcome {
    blocker.install(rules).map_err(|error| error.to_string())?;
    spin_until(|| blocker.status() != BlockerStatus::Compiling)?;
    match blocker.status() {
        BlockerStatus::Installed(1) => Ok(()),
        other => Err(format!("the blocker ended {other:?}")),
    }
}

/// Spins until the page was parsed `times` times: its `/done` request arrived.
fn wait_for(seen: &Seen, times: usize) -> Outcome {
    spin_until(|| count(seen, "/done") >= times)
}

fn count(seen: &Seen, path: &str) -> usize {
    seen.lock()
        .map_or(0, |seen| seen.iter().filter(|seen| *seen == path).count())
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

/// A server on 127.0.0.1 that answers every request and records its path.
fn serve() -> Result<(Url, Seen), String> {
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
    let base =
        Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|error| error.to_string())?;
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
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    let path = path.split('?').next().unwrap_or_default().to_owned();
    let (kind, body) = match path.as_str() {
        "/page.html" => ("text/html", PAGE),
        "/tracker.js" => ("text/javascript", "window.tracked = true;"),
        _ => ("text/plain", ""),
    };
    if let Ok(mut seen) = log.lock() {
        seen.push(path);
    }
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
}

//! A paired comparison in one offscreen web process with a fixed reference from commit
//! `48501264252595d7d98a0aec62da60c1f030bbf9`. Compile the candidate parser directly so this
//! developer-only harness does not expose a new parser API in the adapter.

#[path = "baseline/mod.rs"]
mod baseline;
#[path = "../../src/page/scripts/agent_links/parse.rs"]
mod candidate;

use std::cell::RefCell;
use std::ffi::CStr;
use std::hint::black_box;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use block2::RcBlock;
use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};
use quickie_application::conformance::Harness;
use quickie_application::{AgentLinks, EngineError, PageSurface};
use quickie_domain_browsing::TabId;
use quickie_engine_webkit::WebKitEngine;
use serde_json::{Value, json};

use super::{PERF_ROUTE, WebKit, open, read};
use crate::{failed, print};

const WEBKIT_BATCH: u32 = 32;
const RUST_BATCH: u32 = 128;
const WARMUP_PAIRS: usize = 4;
const CANDIDATE_COLLECTOR: &str = include_str!("../../src/page/scripts/agent_links/links.js");
type Parse = fn(&str) -> Result<AgentLinks, EngineError>;

pub(crate) fn benchmark(harness: &mut WebKit) -> Option<ExitCode> {
    const FLAG: &str = "--agent-links-perf=";
    let argument = std::env::args().find(|value| value.starts_with(FLAG))?;
    let Some(samples) = argument
        .strip_prefix(FLAG)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|samples| (5..=80).contains(samples))
    else {
        return Some(failed("agent link performance needs 5 to 80 samples"));
    };
    Some(match run(harness, samples) {
        Ok(report) => {
            print(&format!("{report}\n"));
            ExitCode::SUCCESS
        }
        Err(why) => failed(&why),
    })
}

fn run(harness: &mut WebKit, samples: usize) -> Result<Value, String> {
    let app = NSApplication::sharedApplication(harness.mtm);
    if app.activationPolicy() != NSApplicationActivationPolicy::Prohibited
        && !app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited)
    {
        return Err("the performance harness could not disable activation".to_owned());
    }
    let framework = pinned_framework()?;
    let typical = measure_page(harness, PERF_ROUTE, samples)?;
    let deep = measure_page(harness, &format!("{PERF_ROUTE}?deep"), samples)?;
    Ok(json!({
        "benchmark": "agent-links",
        "baseline": "48501264252595d7d98a0aec62da60c1f030bbf9",
        "framework": framework,
        "framework_uuid": std::env::var("QUICKIE_PERF_FRAMEWORK_UUID").ok(),
        "architecture": std::env::consts::ARCH,
        "macos": std::env::var("QUICKIE_PERF_OS").ok(),
        "cpu": std::env::var("QUICKIE_PERF_CPU").ok(),
        "samples_per_variant": samples,
        "warmup_pairs": WARMUP_PAIRS,
        "webkit_iterations_per_sample": WEBKIT_BATCH,
        "rust_iterations_per_sample": RUST_BATCH,
        "microseconds_per_call": {"typical": typical, "deep": deep},
    }))
}

fn measure_page(harness: &mut WebKit, route: &str, samples: usize) -> Result<Value, String> {
    let (mut engine, tab, _) = open(harness, route)?;
    let before = measured_script(baseline::COLLECT_SCRIPT);
    let after = measured_script(CANDIDATE_COLLECTOR);
    let mut webkit_before = Vec::with_capacity(samples);
    let mut webkit_after = Vec::with_capacity(samples);
    let mut rust_before = Vec::with_capacity(samples);
    let mut rust_after = Vec::with_capacity(samples);

    for pair in 0..samples + WARMUP_PAIRS {
        let (old, new) = if pair % 2 == 0 {
            let old = evaluate(harness, &mut engine, tab, &before)?;
            let new = evaluate(harness, &mut engine, tab, &after)?;
            (old, new)
        } else {
            let new = evaluate(harness, &mut engine, tab, &after)?;
            let old = evaluate(harness, &mut engine, tab, &before)?;
            (old, new)
        };
        if old.reply != new.reply {
            return Err("WebKit variants returned different link snapshots".to_owned());
        }
        let old_links = baseline::parse(&old.reply).map_err(|error| error.to_string())?;
        let new_links = candidate::parse(&old.reply).map_err(|error| error.to_string())?;
        if old_links != new_links || new_links.links.len() != 100 {
            return Err("Rust variants returned different or undersized link snapshots".to_owned());
        }
        if pair == 0 && read(harness, &mut engine, tab)? != new_links {
            return Err("the production engine port returned a different link snapshot".to_owned());
        }
        let (old_rust, new_rust) = if pair % 2 == 0 {
            (
                measure_parse(baseline::parse, &old.reply)?,
                measure_parse(candidate::parse, &old.reply)?,
            )
        } else {
            let new_rust = measure_parse(candidate::parse, &old.reply)?;
            let old_rust = measure_parse(baseline::parse, &old.reply)?;
            (old_rust, new_rust)
        };
        if pair >= WARMUP_PAIRS {
            webkit_before.push(old.microseconds);
            webkit_after.push(new.microseconds);
            rust_before.push(old_rust);
            rust_after.push(new_rust);
        }
    }
    Ok(json!({
        "webkit": {"before": percentiles(webkit_before), "after": percentiles(webkit_after)},
        "rust": {"before": percentiles(rust_before), "after": percentiles(rust_after)},
    }))
}

struct WebSample {
    microseconds: f64,
    reply: String,
}

fn measured_script(collector: &str) -> Retained<NSString> {
    NSString::from_str(&format!(
        r"(() => {{
            const collect = () => (
                {collector}
            );
            let reply;
            const start = performance.now();
            for (let i = 0; i < {WEBKIT_BATCH}; ++i)
                reply = collect();
            return JSON.stringify({{milliseconds: performance.now() - start, reply}});
        }})()"
    ))
}

fn evaluate(
    harness: &mut WebKit,
    engine: &mut WebKitEngine,
    tab: TabId,
    script: &NSString,
) -> Result<WebSample, String> {
    let view = engine
        .view(tab)
        .and_then(|view| view.downcast_ref::<WKWebView>())
        .ok_or("the performance fixture has no web view")?;
    let reply = Rc::new(RefCell::new(None));
    let received = Rc::clone(&reply);
    let block = RcBlock::new(move |answer: *mut AnyObject, error: *mut NSError| {
        // SAFETY: WebKit owns the callback's nullable object pointers until it returns.
        let value = unsafe { answer.as_ref() }
            .filter(|_| error.is_null())
            .and_then(|answer| answer.downcast_ref::<NSString>())
            .map(NSString::to_string);
        received.replace(Some(value));
    });
    // SAFETY: the named production world is obtained on the main thread.
    let world =
        unsafe { WKContentWorld::worldWithName(&NSString::from_str("quickie"), harness.mtm) };
    // SAFETY: the main-thread view and world outlive the call. WebKit copies the callback.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            script,
            None,
            &world,
            Some(&block),
        );
    }
    if !harness.pump(engine, &mut |_| reply.borrow().is_some()) {
        return Err("the performance fixture evaluation timed out".to_owned());
    }
    let answer = reply
        .take()
        .flatten()
        .ok_or("the performance fixture evaluation did not return text")?;
    let answer: Value = serde_json::from_str(&answer).map_err(|error| error.to_string())?;
    let milliseconds = answer["milliseconds"]
        .as_f64()
        .ok_or("missing WebKit timing")?;
    if milliseconds <= 0.0 {
        return Err("WebKit timer resolution was too coarse for a batch".to_owned());
    }
    Ok(WebSample {
        microseconds: milliseconds * 1_000.0 / f64::from(WEBKIT_BATCH),
        reply: answer["reply"]
            .as_str()
            .ok_or("missing link snapshot")?
            .to_owned(),
    })
}

fn measure_parse(parse: Parse, reply: &str) -> Result<f64, String> {
    let start = Instant::now();
    for _ in 0..RUST_BATCH {
        let links = parse(black_box(reply)).map_err(|error| error.to_string())?;
        black_box(links);
    }
    Ok(start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(RUST_BATCH))
}

fn percentiles(mut micros: Vec<f64>) -> Value {
    micros.sort_unstable_by(f64::total_cmp);
    let middle = micros.len() / 2;
    let median = if micros.len().is_multiple_of(2) {
        f64::midpoint(micros[middle - 1], micros[middle])
    } else {
        micros[middle]
    };
    json!({"p50": median, "p95": micros[(micros.len() * 95).div_ceil(100) - 1]})
}

fn pinned_framework() -> Result<PathBuf, String> {
    let directory = std::env::var_os("DYLD_FRAMEWORK_PATH")
        .map(PathBuf::from)
        .ok_or("set DYLD_FRAMEWORK_PATH to the pinned WebKit build")?;
    let expected = directory
        .join("WebKit.framework/WebKit")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    // SAFETY: Objective-C supplies a static, null-terminated image path for the loaded class.
    let image = unsafe { objc2::ffi::class_getImageName(WKWebView::class()) };
    if image.is_null() {
        return Err("the loaded WebKit image could not be identified".to_owned());
    }
    // SAFETY: the image pointer has just been checked for null and denotes a C string.
    let actual = unsafe { CStr::from_ptr(image) }.to_string_lossy();
    let actual = PathBuf::from(actual.as_ref());
    if actual.canonicalize().ok().as_ref() != Some(&expected) {
        return Err(format!(
            "expected {}, loaded {}",
            expected.display(),
            actual.display()
        ));
    }
    Ok(expected)
}

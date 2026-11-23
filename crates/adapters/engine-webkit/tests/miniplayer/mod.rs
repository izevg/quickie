//! Exercises the production video observer and moves the same web view through an offscreen slot.

use std::io::Write;
use std::net::TcpStream;

use objc2::MainThreadOnly;
use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_application::conformance::{Harness, WINDOW};
use quickie_application::{Engine, EngineEvent, EnginePort, PageContent, PageSurface, PageVideo};
use quickie_domain_browsing::{Jar, JarId, Profile, TabId, Video};
use quickie_engine_webkit::WebKitEngine;
use url::Url;

use super::WebKit;

pub(super) const ROUTE: &str = "video-miniplayer/fixture";

pub(super) fn answer(stream: &mut TcpStream) {
    let body = include_str!("fixture.html")
        .replace("__QUICKIE_VIDEO__", include_str!("tone.base64").trim());
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

pub(super) fn check(harness: &mut WebKit) -> Result<(), String> {
    let url = harness
        .base
        .join(ROUTE)
        .map_err(|error| error.to_string())?;
    let tab = TabId::new(Profile::Main, 1);
    let jar = Jar::Throwaway(JarId::new(1));
    let mut engine = harness.engine();
    engine
        .create(tab, &url, jar)
        .and_then(|()| engine.show(WINDOW, tab))
        .map_err(|error| error.to_string())?;

    let mut events = Vec::new();
    let mut loaded = false;
    if !harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        loaded |= events.drain(..).any(|event| {
            matches!(event, EngineEvent::Committed { tab: owner, url: at, .. } if owner == tab && at == url)
        });
        loaded && engine.loading(tab).is_none()
    }) {
        return Err("the miniplayer fixture did not load".to_owned());
    }
    let ticket = engine
        .link_targets(tab)
        .map_err(|error| error.to_string())?;
    let mut button = None;
    harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        button = events.drain(..).find_map(|event| match event {
            EngineEvent::Targets {
                ticket: ready,
                targets,
                ..
            } if ready == ticket => targets.first().map(|target| target.frame),
            _ => None,
        });
        button.is_some()
    });
    harness.click(
        &mut engine,
        tab,
        button.ok_or("the fixture has no play button")?,
        false,
    )??;

    let mut video = None;
    let mut title = String::new();
    if !harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        for event in events.drain(..) {
            match event {
                EngineEvent::Video {
                    tab: owner,
                    video: state,
                } if owner == tab => video = state,
                EngineEvent::Titled {
                    tab: owner,
                    title: value,
                    ..
                } if owner == tab => title = value,
                _ => {}
            }
        }
        title.starts_with("play failed")
            || (title == "playing" && video == Some(Video::Playing { audible: true }))
    }) || title != "playing"
        || video != Some(Video::Playing { audible: true })
    {
        return Err(format!(
            "the audible video did not remain selected: {title:?}, {video:?}"
        ));
    }

    check_floating(harness, &mut engine, tab, jar)
}

fn check_floating(
    harness: &mut WebKit,
    engine: &mut WebKitEngine,
    tab: TabId,
    jar: Jar,
) -> Result<(), String> {
    let other = TabId::new(Profile::Main, 2);
    let blank = Url::parse("about:blank").map_err(|error| error.to_string())?;
    let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 180.0));
    let slot = NSView::initWithFrame(NSView::alloc(harness.mtm), bounds);
    harness
        .windows
        .last()
        .and_then(|window| window.contentView())
        .ok_or("the offscreen window has no host")?
        .addSubview(&slot);
    engine
        .create(other, &blank, jar)
        .and_then(|()| engine.show(WINDOW, other))
        .and_then(|()| engine.float_video(tab, &slot))
        .map_err(|error| error.to_string())?;
    expect_title(harness, engine, tab, "audible video fills the slot")?;
    // SAFETY: the live views are read on the harness's main thread.
    let superview = engine
        .view(tab)
        .and_then(|view| unsafe { view.superview() });
    if superview.as_deref() != Some(&slot) {
        return Err("the web view was not attached to the miniplayer slot".to_owned());
    }
    engine
        .show(WINDOW, tab)
        .map_err(|error| error.to_string())?;
    expect_title(harness, engine, tab, "docked")?;

    engine
        .show(WINDOW, other)
        .and_then(|()| engine.float_video(tab, &slot))
        .map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    if !harness.pump(engine, &mut |engine| {
        engine.drain(&mut events);
        events.drain(..).any(|event| {
            matches!(
                event,
                EngineEvent::Video { tab: owner, video: None } if owner == tab
            )
        })
    }) {
        return Err("the removed floating video was still reported as present".to_owned());
    }
    Ok(())
}

fn expect_title(
    harness: &mut WebKit,
    engine: &mut WebKitEngine,
    tab: TabId,
    expected: &str,
) -> Result<(), String> {
    let mut actual = String::new();
    let mut events = Vec::new();
    harness.pump(engine, &mut |engine| {
        engine.drain(&mut events);
        for event in events.drain(..) {
            if let EngineEvent::Titled {
                tab: owner, title, ..
            } = event
                && owner == tab
            {
                actual = title;
            }
        }
        actual == expected
            || matches!(
                actual.as_str(),
                "video did not fill the slot" | "muted video floated"
            )
    });
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "expected miniplayer fixture title {expected:?}, got {actual:?}"
        ))
    }
}

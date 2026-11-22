//! Drives the macOS video presentation API in a normal tab of the offscreen harness.

use std::io::Write;
use std::net::TcpStream;

use quickie_application::conformance::{Harness, WINDOW};
use quickie_application::{Engine, EngineEvent, EnginePort, Fullscreen, PageContent, PageSurface};
use quickie_domain_browsing::{Jar, JarId, Profile, TabId};

use super::WebKit;

pub(super) const ROUTE: &str = "video-fullscreen/fixture";

pub(super) fn answer(stream: &mut TcpStream) {
    let body = include_str!("fixture.html");
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

pub(super) fn check(harness: &mut WebKit) -> Result<(), String> {
    let document = harness
        .base
        .join(ROUTE)
        .map_err(|error| error.to_string())?;
    let tab = TabId::new(Profile::Main, 1);
    let mut engine = harness.engine();
    engine
        .create(tab, &document, Jar::Throwaway(JarId::new(1)))
        .and_then(|()| engine.show(WINDOW, tab))
        .map_err(|error| error.to_string())?;

    let mut events = Vec::new();
    let mut committed = false;
    if !harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        committed |= events.drain(..).any(|event| {
            matches!(event, EngineEvent::Committed { tab: owner, url, .. } if owner == tab && url == document)
        });
        committed && engine.loading(tab).is_none()
    }) {
        return Err("the video fullscreen fixture did not load".to_owned());
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
        button.ok_or("the fixture has no fullscreen button")?,
        false,
    )??;

    let mut states = Vec::new();
    let mut title = String::new();
    let done = harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        for event in events.drain(..) {
            match event {
                EngineEvent::Fullscreen { state, .. } => states.push(state),
                EngineEvent::Titled {
                    title: new_title, ..
                } => title = new_title,
                _ => {}
            }
        }
        matches!(
            title.as_str(),
            "restored" | "not in page" | "video did not fill the page"
        ) || states.contains(&Fullscreen::OwnWindow)
    });
    if !done || title != "restored" || states != [Fullscreen::InWindow, Fullscreen::Left] {
        return Err(format!(
            "video presentation did not enter and leave the same page: {title:?}, {states:?}"
        ));
    }
    Ok(())
}

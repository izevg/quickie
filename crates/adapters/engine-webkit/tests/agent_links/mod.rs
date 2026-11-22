//! Reads an actual page through the production `WebKit` adapter without showing a window.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::net::TcpStream;
use std::rc::Rc;

use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::WKWebView;
use quickie_application::conformance::{Harness, WINDOW};
use quickie_application::{AgentLinks, Engine, EngineEvent, EnginePort, PageContent, PageSurface};
use quickie_domain_browsing::{Jar, JarId, Profile, TabId};
use quickie_engine_webkit::WebKitEngine;

use super::WebKit;

pub(super) const ROUTE: &str = "agent-links/fixture";

pub(super) fn answer(stream: &mut TcpStream) {
    let body = include_str!("fixture.html");
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

pub(super) fn check(harness: &mut WebKit) -> Result<(), String> {
    let _app = NSApplication::sharedApplication(harness.mtm);
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
        return Err("the agent link fixture never committed".to_owned());
    }

    let first = read(harness, &mut engine, tab)?;
    let expected = [
        ("/watch", Some("v=first"), "First video"),
        ("/watch", Some("v=second"), "Second video"),
        ("/videos/below", None, "Below the viewport"),
    ];
    if first.document != document
        || first
            .links
            .iter()
            .map(|link| (link.url.path(), link.url.query(), link.label.as_str()))
            .ne(expected)
    {
        return Err(format!(
            "wrong candidates for the agent link fixture: {first:?}"
        ));
    }

    replace_in_page(harness, &mut engine, tab)?;
    let current = read(harness, &mut engine, tab)?;
    let expected = harness
        .base
        .join("agent-links/updated")
        .map_err(|error| error.to_string())?;
    if current.document != expected
        || current.links.first().is_none_or(|link| {
            link.url.query() != Some("v=replacement") || link.label != "Replacement video"
        })
        || current
            .links
            .iter()
            .any(|link| link.url.query() == Some("v=first"))
    {
        return Err(format!("agent link capture kept stale data: {current:?}"));
    }
    if engine
        .agent_links(TabId::new(Profile::Main, 2), |_| {})
        .is_ok()
    {
        return Err("the engine accepted a link read for a missing tab".to_owned());
    }
    Ok(())
}

fn read(harness: &mut WebKit, engine: &mut WebKitEngine, tab: TabId) -> Result<AgentLinks, String> {
    let reply = Rc::new(RefCell::new(None));
    let received = Rc::clone(&reply);
    engine
        .agent_links(tab, move |result| *received.borrow_mut() = Some(result))
        .map_err(|error| error.to_string())?;
    if !harness.pump(engine, &mut |_| reply.borrow().is_some()) {
        return Err("the agent link read did not answer".to_owned());
    }
    reply
        .take()
        .ok_or("the agent link read lost its result")?
        .map_err(|error| error.to_string())
}

fn replace_in_page(
    harness: &mut WebKit,
    engine: &mut WebKitEngine,
    tab: TabId,
) -> Result<(), String> {
    let view = engine
        .view(tab)
        .and_then(|view| view.downcast_ref::<WKWebView>())
        .ok_or("the fixture has no web view")?;
    let changed = Rc::new(Cell::new(None));
    let result = Rc::clone(&changed);
    let reply = RcBlock::new(move |_: *mut AnyObject, error: *mut NSError| {
        result.set(Some(error.is_null()));
    });
    let script = NSString::from_str(
        r"(() => {
            if (window.linkClicks)
                throw new Error('the first snapshot activated a link');
            const link = document.getElementById('first');
            link.href = '/watch?v=replacement';
            link.title = 'Replacement video';
            document.getElementById('duplicate').remove();
            history.replaceState(null, '', '/agent-links/updated');
            Element.prototype.checkVisibility = () => { throw new Error('page world'); };
        })()",
    );
    // SAFETY: this fixture uses the live main-thread view to make a same-document change in
    // the page's own world. WebKit copies the completion block.
    unsafe { view.evaluateJavaScript_completionHandler(&script, Some(&reply)) };
    if !harness.pump(engine, &mut |_| changed.get().is_some()) || changed.get() != Some(true) {
        return Err("the fixture could not change its document without a link click".to_owned());
    }
    Ok(())
}

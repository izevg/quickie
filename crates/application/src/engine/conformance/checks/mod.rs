//! The checks, and what they share: a fresh engine per check, the events it reported so far,
//! and waiting for the next ones.

mod content;
mod lifecycle;
mod pages;

use std::fmt::Debug;

use quickie_domain_browsing::{Focus, Jar, Profile, TabId};
use url::Url;

use super::{Event, Group, Harness, NotDriven, Outcome, Page};
use crate::{EngineEvent, EnginePort, LinkTarget, PageContent};

/// Why a check failed.
type Checked = Result<(), String>;

/// A check, run on a fresh engine.
pub(super) type Check<H> = fn(&mut Run<'_, H>) -> Checked;

/// A named check of a group.
pub(super) struct Case<H: Harness> {
    pub(super) group: Group,
    pub(super) name: &'static str,
    pub(super) run: Check<H>,
}

/// Every check, group by group.
pub(super) fn cases<H: Harness>() -> Vec<Case<H>> {
    let mut cases = lifecycle::cases();
    cases.extend(pages::cases());
    cases.extend(content::cases());
    cases
}

/// Runs `check` on a fresh engine of `harness`.
pub(super) fn check<H: Harness>(harness: &mut H, check: Check<H>) -> Outcome {
    let engine = harness.engine();
    let mut run = Run {
        harness,
        engine,
        events: Vec::new(),
    };
    match check(&mut run) {
        Ok(()) => Outcome::Passed,
        Err(why) => Outcome::Failed(why),
    }
}

impl From<NotDriven> for String {
    fn from(NotDriven(feature): NotDriven) -> Self {
        format!("the harness cannot drive `{feature}`: drive it or declare the group unsupported")
    }
}

/// The tab numbered `serial`.
fn tab(serial: u64) -> TabId {
    TabId::new(Profile::Main, serial)
}

/// Fails with `why` unless `holds`.
fn ensure(holds: bool, why: impl FnOnce() -> String) -> Checked {
    if holds { Ok(()) } else { Err(why()) }
}

/// Fails unless `got` is `want`, naming `what`.
fn same<T: PartialEq + Debug>(what: &str, got: &T, want: &T) -> Checked {
    ensure(got == want, || {
        format!("{what}: expected {want:?}, the engine gave {got:?}")
    })
}

/// One check's engine and what it reported.
pub(super) struct Run<'h, H: Harness> {
    harness: &'h mut H,
    engine: H::Engine,
    /// Every event drained so far, oldest first.
    events: Vec<Event<H>>,
}

impl<H: Harness> Run<'_, H> {
    /// Where the suite's resource at `path` is served.
    fn url(&self, path: &str) -> Url {
        self.harness
            .base()
            .join(path)
            .unwrap_or_else(|_| self.harness.base())
    }

    /// Creates the page of tab `serial` in `jar`, at `path`.
    fn create(&mut self, serial: u64, path: &str, jar: Jar) -> Checked {
        let url = self.url(path);
        self.engine
            .create(tab(serial), &url, jar)
            .map_err(|error| format!("creating {url} failed: {error}"))
    }

    /// Lets the engine work until the events drained so far satisfy `done`.
    fn wait(&mut self, what: &str, done: impl Fn(&[Event<H>]) -> bool) -> Checked {
        let events = &mut self.events;
        H::drain(&mut self.engine, events);
        let held = done(events)
            || self.harness.pump(&mut self.engine, &mut |engine| {
                H::drain(engine, events);
                done(events)
            });
        ensure(held, || {
            format!("timed out waiting for {what}; the engine reported {events:?}")
        })
    }

    /// Lets the engine work for a while, then drains what it reported.
    fn idle(&mut self) {
        self.harness.idle(&mut self.engine);
        H::drain(&mut self.engine, &mut self.events);
    }

    /// Waits until tab `serial` is at `path`, titled `title`.
    fn wait_visit(&mut self, serial: u64, path: &str, title: &str) -> Checked {
        let want = Some((self.url(path), title.to_owned()));
        self.wait(
            &format!("{} at {path} titled {title:?}", tab(serial)),
            |events| visit(events, tab(serial)) == want,
        )
    }

    /// Waits until tab `serial` is titled `title`.
    fn wait_title(&mut self, serial: u64, title: &str) -> Checked {
        self.wait(&format!("{} titled {title:?}", tab(serial)), |events| {
            visit(events, tab(serial)).is_some_and(|(_, now)| now == title)
        })
    }

    /// Waits for an event `pick` takes out of the stream from `mark` on, and takes it out.
    fn take(
        &mut self,
        mark: usize,
        what: &str,
        pick: impl Fn(&Event<H>) -> bool,
    ) -> Result<Event<H>, String> {
        self.wait(what, |events| events.iter().skip(mark).any(&pick))?;
        let at = (mark..self.events.len())
            .find(|&at| self.events.get(at).is_some_and(&pick))
            .ok_or_else(|| format!("{what} went missing"))?;
        Ok(self.events.remove(at))
    }

    /// Asks for tab `serial`'s link targets and waits for the answer to that request.
    fn targets(&mut self, serial: u64) -> Result<Vec<LinkTarget>, String> {
        let ticket = self
            .engine
            .link_targets(tab(serial))
            .map_err(|error| format!("asking for the link targets failed: {error}"))?;
        let answer = self.take(0, "the link targets", |event| {
            matches!(event, EngineEvent::Targets { ticket: answered, .. } if *answered == ticket)
        })?;
        match answer {
            EngineEvent::Targets {
                tab: about,
                targets,
                ..
            } => {
                same("whose targets", &about, &tab(serial))?;
                Ok(targets)
            }
            other => Err(format!("{other:?} is not link targets")),
        }
    }

    /// Waits for a page tab `opener` opened, from `mark` on, and adopts it as tab `serial`;
    /// says where it is and in what focus it opened.
    fn adopt_opened(
        &mut self,
        mark: usize,
        opener: u64,
        adopter: u64,
    ) -> Result<(Url, Focus), String> {
        let event = self.take(
            mark,
            "a page opened",
            |event| matches!(event, EngineEvent::Opened { opener: by, .. } if *by == tab(opener)),
        )?;
        let EngineEvent::Opened {
            page, url, focus, ..
        } = event
        else {
            return Err("an opened page that is not one".to_owned());
        };
        self.adopt(page, adopter)?;
        Ok((url, focus))
    }

    /// Adopts `page` as tab `serial`.
    fn adopt(&mut self, page: Page<H>, serial: u64) -> Checked {
        self.engine
            .adopt(page, tab(serial))
            .map_err(|error| format!("adopting the opened page failed: {error}"))
    }
}

/// Where a tab's page is and its title, as its last navigation events say.
fn visit<P>(events: &[EngineEvent<P>], tab: TabId) -> Option<(Url, String)> {
    events.iter().rev().find_map(|event| match event {
        EngineEvent::Committed {
            tab: about,
            url,
            title,
        }
        | EngineEvent::Titled {
            tab: about,
            url,
            title,
        } if *about == tab => Some((url.clone(), title.clone())),
        _ => None,
    })
}

/// The first of `tab`'s navigation reports, a commit or a rename; loading may come before it.
fn first_navigation<P>(events: &[EngineEvent<P>], tab: TabId) -> Option<&EngineEvent<P>> {
    events.iter().find(|event| {
        matches!(
            event,
            EngineEvent::Committed { tab: about, .. } | EngineEvent::Titled { tab: about, .. }
                if *about == tab
        )
    })
}

/// True when `event` is about `tab`'s page itself: a navigation, its loading, or an ask.
fn about<P>(event: &EngineEvent<P>, tab: TabId) -> bool {
    match event {
        EngineEvent::Committed { tab: about, .. }
        | EngineEvent::Titled { tab: about, .. }
        | EngineEvent::Loading { tab: about, .. } => *about == tab,
        EngineEvent::Asked(ask) => ask.tab == tab,
        _ => false,
    }
}

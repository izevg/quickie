//! The engine conformance suite: what every engine adapter must do, checked the same way on each.
//! An adapter implements [`Harness`] for its engine (a fresh engine, a way to let it work, the
//! page features that are not ports yet) and [`PageSource`] (where it serves the suite's pages,
//! [`SITE`], from memory: no request leaves the machine), then calls [`run`] and fails its test
//! when the [`Report`] holds a failure.
//!
//! The checks are grouped by what an engine can do ([`Group`]). An engine that cannot do a group
//! says so, with why, in [`Harness::unsupported`]; nothing is skipped silently. A group the
//! harness does not declare runs, and a page feature it needs that the harness does not drive
//! fails it. A feature that is a port (the whole [`Engine`]: its events, [`PageSurface`],
//! [`PageContent`], [`PageZoom`], [`PageAsks`], [`Downloads`], [`PageVideo`]) is called on the
//! engine itself,
//! and an engine that answers it with an error fails the group the same way.
//!
//! [`MemoryEngine`] is the reference: an engine whose pages live in memory and do what the suite's
//! pages say, in turns of its own loop. It passes every group, which also keeps the suite honest.

mod checks;
mod memory;
mod report;
mod site;
#[cfg(test)]
mod tests;

use std::fmt;
use std::path::{Path, PathBuf};

use quickie_domain_browsing::{Profile, TabId, WindowId};
use url::Url;

use crate::{BlockerRules, Engine, EngineEvent, EnginePort, PageRect};
#[cfg(doc)]
use crate::{Downloads, PageAsks, PageContent, PageSurface, PageVideo, PageZoom};

pub use memory::{MemoryBlocker, MemoryEngine, MemoryHttp, MemoryPage, MemoryView};
pub use report::{Outcome, Report, Verdict};
pub use site::{Act, Control, Resource, SITE, TestPage};

/// The window every check shows its pages in: [`Harness::engine`] gives it a host.
pub const WINDOW: WindowId = WindowId::new(Profile::Main, 1);

/// The page an engine opened by itself, as the harness's engine names it.
pub type Page<H> = <<H as Harness>::Engine as EnginePort>::Page;

/// What the harness's engine reports.
pub type Event<H> = EngineEvent<Page<H>>;

/// A page feature the harness drives, or [`NotDriven`] when it does not.
pub type Driven<T> = Result<T, NotDriven>;

/// What an engine can do, as the suite checks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Pages exist only once created; created pages commit, navigate, show, and close.
    Core,
    /// Events come in order, after the waker ran, and never inside an engine call.
    Events,
    /// Pages open pages: `window.open`, a `target=_blank` link, a `⌘`-click; a page closes itself.
    Opened,
    /// Throwaway jars keep cookies apart; a popup shares its opener's jar; a destroyed jar
    /// forgets.
    Jars,
    /// JavaScript dialogs wait for an answer with a reply token.
    Dialogs,
    /// Device permission requests wait for an answer with a reply token.
    Permissions,
    /// Find counts the matches and steps through them.
    Find,
    /// Link hints: the targets on screen, and following one.
    Hints,
    /// A page's rendered text.
    Text,
    /// Downloads start, finish, are cancelled, and are refused without a folder.
    Downloads,
    /// The content blocker stops what its rules name.
    Blocker,
    /// A page whose renderer died is reported and reloads.
    Crash,
    /// A zoomed page lays out larger, and stays zoomed across its navigations.
    Zoom,
    /// A page has a view while it exists, says how far it loaded, has a picture once captured,
    /// and says when a text field of its own has focus.
    Surface,
    /// A page says what its video does: it plays, pauses when asked, plays again, and is gone
    /// when the page leaves.
    Video,
}

impl Group {
    /// Every group, in the order the suite runs them.
    pub const ALL: [Self; 15] = [
        Self::Core,
        Self::Surface,
        Self::Events,
        Self::Opened,
        Self::Jars,
        Self::Dialogs,
        Self::Permissions,
        Self::Find,
        Self::Hints,
        Self::Text,
        Self::Downloads,
        Self::Blocker,
        Self::Crash,
        Self::Zoom,
        Self::Video,
    ];
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// The harness cannot drive the page feature named. A group that needs it fails unless the
/// harness declares the group unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotDriven(pub &'static str);

/// Where the suite's resources ([`SITE`]) are served, from memory.
///
/// A resource at `path` is at `base().join(path)`. A [`Resource::Page`] is HTML
/// ([`TestPage::html`]), a [`Resource::Script`] JavaScript; an attachment is served with
/// `Content-Disposition: attachment; filename="<path>"`, and a [`Resource::Stalled`] one sends
/// its first bytes and never the rest.
pub trait PageSource {
    /// The origin every resource is served from, as a URL ending in `/`.
    fn base(&self) -> Url;
}

/// One engine, as the suite drives it: an [`Engine`], how its loop runs, and what the user or
/// the machine does to its pages that no port does, each mapped to the engine's own call. A
/// feature the harness cannot drive answers [`NotDriven`].
pub trait Harness: PageSource {
    /// The engine under test.
    type Engine: Engine<Page: fmt::Debug>;

    /// Why the engine does not do `group`, or `None` when it does and the group runs.
    fn unsupported(&self, group: Group) -> Option<&'static str>;

    /// A fresh engine with no pages, whose window [`WINDOW`] has a host to show pages in.
    fn engine(&mut self) -> Self::Engine;

    /// Lets the engine work, turn after turn of its loop, until `done` holds; false when the
    /// harness gave up waiting first.
    fn pump(
        &mut self,
        engine: &mut Self::Engine,
        done: &mut dyn FnMut(&mut Self::Engine) -> bool,
    ) -> bool;

    /// Lets the engine work long enough for whatever would come to come.
    fn idle(&mut self, engine: &mut Self::Engine);

    /// Clicks a tab's page in the middle of `at`, a link target's frame, as the user does,
    /// holding `⌘` when `command`; or says why it could not. The page is shown in [`WINDOW`].
    ///
    /// # Errors
    ///
    /// The harness cannot drive it.
    fn click(
        &mut self,
        engine: &mut Self::Engine,
        tab: TabId,
        at: PageRect,
        command: bool,
    ) -> Driven<Result<(), String>>;

    /// Makes a new, empty folder where the engine may save downloads, and names it, or says why
    /// it could not. The check makes it the engine's downloads folder.
    ///
    /// # Errors
    ///
    /// The harness cannot drive it.
    fn new_folder(&mut self) -> Driven<Result<PathBuf, String>>;

    /// What a downloaded `file` holds, `None` when there is no such file.
    ///
    /// # Errors
    ///
    /// The harness cannot drive it.
    fn downloaded(&self, engine: &Self::Engine, file: &Path) -> Driven<Option<Vec<u8>>>;

    /// Installs `rules` into the engine's content blocker and returns once pages load with
    /// them, or with why they did not take.
    ///
    /// # Errors
    ///
    /// The harness cannot drive it.
    fn block(
        &mut self,
        engine: &mut Self::Engine,
        rules: &BlockerRules,
    ) -> Driven<Result<(), String>>;

    /// Kills the process that renders a tab's page, or says why it could not.
    ///
    /// # Errors
    ///
    /// The harness cannot drive it.
    fn crash(&mut self, engine: &mut Self::Engine, tab: TabId) -> Driven<Result<(), String>>;
}

/// Runs every check on a fresh engine of `harness`, skipping the groups it declares unsupported.
pub fn run<H: Harness>(harness: &mut H) -> Report {
    let verdicts = checks::cases::<H>()
        .into_iter()
        .map(|case| {
            let outcome = match harness.unsupported(case.group) {
                Some(why) => Outcome::Skipped(why),
                None => checks::check(harness, case.run),
            };
            Verdict {
                group: case.group,
                case: case.name,
                outcome,
            }
        })
        .collect();
    Report::new(verdicts)
}

//! The engine conformance suite against the parked Chromium adapter. Cargo builds it without
//! the C++ side, where every engine call fails as unavailable, so it declares every group
//! unsupported; compiling against the suite keeps its API honest until it runs for real.

use std::path::{Path, PathBuf};

use quickie_application::conformance::{
    self, Driven, Group, Harness, NotDriven, Outcome, PageSource,
};
use quickie_application::{BlockerRules, PageRect};
use quickie_domain_browsing::TabId;
use quickie_engine_chromium::ChromiumEngine;
use url::Url;

/// Why no group runs.
const PARKED: &str = "the Chromium engine is parked: Cargo builds it without the C++ side \
                      (the `chromium` feature), so there is no engine to drive";

/// The Chromium engine as the suite would drive it; its pages would be served at `base`.
struct Chromium {
    base: Url,
}

impl PageSource for Chromium {
    fn base(&self) -> Url {
        self.base.clone()
    }
}

impl Harness for Chromium {
    type Engine = ChromiumEngine;

    fn unsupported(&self, group: Group) -> Option<&'static str> {
        match group {
            Group::Core
            | Group::Surface
            | Group::Events
            | Group::Opened
            | Group::Jars
            | Group::Dialogs
            | Group::Permissions
            | Group::Fullscreen
            | Group::Find
            | Group::Hints
            | Group::Text
            | Group::Downloads
            | Group::Blocker
            | Group::Crash
            | Group::Zoom
            | Group::Video => Some(PARKED),
        }
    }

    fn engine(&mut self) -> ChromiumEngine {
        ChromiumEngine::new(|_| false)
    }

    fn pump(
        &mut self,
        _engine: &mut ChromiumEngine,
        _done: &mut dyn FnMut(&mut ChromiumEngine) -> bool,
    ) -> bool {
        // Without the C++ side there is no loop to run.
        false
    }

    fn idle(&mut self, _engine: &mut ChromiumEngine) {}

    // The bridge has none of the page features yet; its page content port fails each request.

    fn click(
        &mut self,
        _: &mut ChromiumEngine,
        _: TabId,
        _: PageRect,
        _: bool,
    ) -> Driven<Result<(), String>> {
        Err(NotDriven("click"))
    }

    fn new_folder(&mut self) -> Driven<Result<PathBuf, String>> {
        Err(NotDriven("new_folder"))
    }

    fn downloaded(&self, _: &ChromiumEngine, _: &Path) -> Driven<Option<Vec<u8>>> {
        Err(NotDriven("downloaded"))
    }

    fn block(&mut self, _: &mut ChromiumEngine, _: &BlockerRules) -> Driven<Result<(), String>> {
        Err(NotDriven("block"))
    }

    fn crash(&mut self, _: &mut ChromiumEngine, _: TabId) -> Driven<Result<(), String>> {
        Err(NotDriven("crash"))
    }
}

#[test]
fn every_group_is_declared_unsupported_while_the_engine_is_parked() {
    let mut chromium = Chromium {
        base: Url::parse("http://127.0.0.1/").unwrap(),
    };

    let report = conformance::run(&mut chromium);

    assert!(report.passed(), "{report}");
    assert!(
        report
            .verdicts()
            .iter()
            .all(|verdict| verdict.outcome == Outcome::Skipped(PARKED)),
        "{report}"
    );
}

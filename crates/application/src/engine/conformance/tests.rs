mod faults;
mod rendering;

use std::path::{Path, PathBuf};

use quickie_domain_browsing::{Jar, Profile, TabId};
use url::Url;

use super::{Driven, Event, Group, Harness, MemoryEngine, Outcome, PageSource, Report, run};
use crate::{
    Answer, AskKind, BlockerRules, ContentBlockerPort, EngineEvent, EnginePort, PageAsks, PageRect,
    Permission,
};

/// The reference engine, driven as an adapter would drive its own.
#[derive(Clone, Copy)]
struct Memory;

impl PageSource for Memory {
    fn base(&self) -> Url {
        Url::parse("http://conformance.test/").unwrap()
    }
}

impl Harness for Memory {
    type Engine = MemoryEngine;

    fn unsupported(&self, _group: Group) -> Option<&'static str> {
        None
    }

    fn engine(&mut self) -> MemoryEngine {
        MemoryEngine::new(self.base())
    }

    fn pump(
        &mut self,
        engine: &mut MemoryEngine,
        done: &mut dyn FnMut(&mut MemoryEngine) -> bool,
    ) -> bool {
        (0..50).any(|_| {
            engine.turn();
            done(engine)
        })
    }

    fn idle(&mut self, engine: &mut MemoryEngine) {
        for _ in 0..20 {
            engine.turn();
        }
    }

    fn set_waker(engine: &mut MemoryEngine, wake: Box<dyn Fn()>) {
        engine.set_waker(wake);
    }

    fn drain(engine: &mut MemoryEngine, into: &mut Vec<Event<Self>>) {
        engine.drain(into);
    }

    fn click(
        &mut self,
        engine: &mut MemoryEngine,
        tab: TabId,
        at: PageRect,
        command: bool,
    ) -> Driven<Result<(), String>> {
        engine.click(tab, at, command);
        Ok(Ok(()))
    }

    fn new_folder(&mut self) -> Driven<Result<PathBuf, String>> {
        Ok(Ok(PathBuf::from("/downloads")))
    }

    fn downloaded(&self, engine: &MemoryEngine, file: &Path) -> Driven<Option<Vec<u8>>> {
        Ok(engine.downloaded(file).map(<[u8]>::to_vec))
    }

    fn block(
        &mut self,
        engine: &mut MemoryEngine,
        rules: &BlockerRules,
    ) -> Driven<Result<(), String>> {
        Ok(engine.install(rules).map_err(|error| error.to_string()))
    }

    fn crash(&mut self, engine: &mut MemoryEngine, tab: TabId) -> Driven<Result<(), String>> {
        Ok(engine.crash(tab).map_err(|error| error.to_string()))
    }
}

/// The checks that failed in `report`, by name.
fn failed(report: &Report) -> Vec<&'static str> {
    report
        .verdicts()
        .iter()
        .filter(|verdict| matches!(verdict.outcome, Outcome::Failed(_)))
        .map(|verdict| verdict.case)
        .collect()
}

#[test]
fn the_memory_engine_passes_every_check() {
    let report = run(&mut Memory);

    assert!(report.passed(), "{report}");
    assert!(
        report
            .verdicts()
            .iter()
            .all(|verdict| verdict.outcome == Outcome::Passed),
        "{report}"
    );
    for group in Group::ALL {
        assert!(
            report
                .verdicts()
                .iter()
                .any(|verdict| verdict.group == group),
            "{group} has no checks"
        );
    }
}

#[test]
fn the_memory_engine_grants_the_camera_when_allowed() {
    let mut engine = Memory.engine();
    let tab = TabId::new(Profile::Main, 1);
    engine
        .create(tab, &Memory.base().join("camera").unwrap(), Jar::Main)
        .unwrap();
    // One turn commits the page, the next asks.
    engine.turn();
    engine.turn();
    let mut events = Vec::new();
    engine.drain(&mut events);
    let token = events
        .iter()
        .find_map(|event| match event {
            EngineEvent::Asked(ask) if ask.kind == AskKind::Permission(Permission::Camera) => {
                Some(ask.token)
            }
            _ => None,
        })
        .unwrap();

    let waited = engine.answer(token, &Answer::Accept(None));

    engine.drain(&mut events);
    assert!(waited);
    assert!(matches!(
        events.last(),
        Some(EngineEvent::Titled { title, .. }) if title == "camera:allowed"
    ));
}

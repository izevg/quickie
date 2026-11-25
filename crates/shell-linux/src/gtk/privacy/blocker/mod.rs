//! Translates filter lists off the GTK thread and installs native content filters as a set.

mod native;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use quickie_application::{
    ActiveRules, BlockerError, BlockerRules, Clock, ContentBlockerPort, RulesStatus,
};
use quickie_engine_webkit::rules;

use super::super::ffi::Object;
use super::WallClock;
use super::cosmetic::Cosmetics;
use native::Filter;

type Encoded = Result<Vec<(String, usize)>, String>;
type Built = Rc<RefCell<Option<Result<Vec<Filter>, String>>>>;

struct Compilation {
    name: String,
    source: Receiver<Encoded>,
}

struct NativeCompilation {
    name: String,
    result: Built,
    rules: usize,
}

struct Installed {
    name: String,
    filters: Vec<Filter>,
    summary: ActiveRules,
}

pub(super) struct Blocker {
    managers: Vec<(Object, Object)>,
    installed: Installed,
    status: RulesStatus,
    wanted: Option<String>,
    pending: Option<Compilation>,
    compiling: Option<NativeCompilation>,
    settled: Option<Box<dyn Fn()>>,
}

impl Default for Blocker {
    fn default() -> Self {
        Self {
            managers: Vec::new(),
            installed: Installed {
                name: String::new(),
                filters: Vec::new(),
                summary: ActiveRules {
                    rules: 0,
                    lists: 0,
                    compiled: WallClock.now(),
                },
            },
            status: RulesStatus::Idle,
            wanted: None,
            pending: None,
            compiling: None,
            settled: None,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct NativeBlocker {
    state: Rc<RefCell<Blocker>>,
    pub(super) cosmetic: Rc<Cosmetics>,
}

impl NativeBlocker {
    pub(super) fn observe(&self, view: Object, data: Object) -> bool {
        let mut state = self.state.borrow_mut();
        if state.managers.iter().any(|&(known, _)| known == view) {
            return false;
        }
        let manager = native::content_manager(view);
        for filter in &state.installed.filters {
            filter.add_to(manager);
        }
        state.managers.push((view, manager));
        drop(state);
        self.cosmetic.observe(view, manager, data);
        true
    }

    pub(super) fn forget(&self, view: Object) {
        self.state
            .borrow_mut()
            .managers
            .retain(|&(known, _)| known != view);
        self.cosmetic.forget(view);
    }

    pub(super) fn pending(&self) -> bool {
        matches!(self.status(), RulesStatus::Compiling)
    }

    pub(super) fn poll(&self) -> bool {
        self.state.borrow_mut().poll()
    }
}

impl Blocker {
    fn poll(&mut self) -> bool {
        if let Some(compiling) = &self.compiling {
            let Some(result) = compiling.result.borrow_mut().take() else {
                return false;
            };
            let Some(compiling) = self.compiling.take() else {
                return false;
            };
            if self.wanted.as_deref() == Some(&compiling.name) {
                self.apply(compiling.name, compiling.rules, result);
                return true;
            }
        }
        let Some(pending) = &self.pending else {
            return false;
        };
        let encoded = match pending.source.try_recv() {
            Ok(encoded) => encoded,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("The filter conversion stopped".to_owned()),
        };
        let Some(pending) = self.pending.take() else {
            return false;
        };
        if pending.name == self.installed.name {
            self.status = RulesStatus::Active(self.installed.summary);
            return true;
        }
        match encoded {
            Ok(documents) if !documents.is_empty() => {
                let count = documents.iter().map(|(_, count)| count).sum();
                let result = native::compile(&pending.name, documents);
                self.compiling = Some(NativeCompilation {
                    name: pending.name,
                    result,
                    rules: count,
                });
                false
            }
            result => {
                self.apply(pending.name, 0, result.map(|_| Vec::new()));
                true
            }
        }
    }

    fn apply(&mut self, name: String, rules: usize, result: Result<Vec<Filter>, String>) {
        self.status = match result {
            Ok(filters) => {
                for &(_, manager) in &self.managers {
                    for filter in &self.installed.filters {
                        filter.remove_from(manager);
                    }
                    for filter in &filters {
                        filter.add_to(manager);
                    }
                }
                let summary = ActiveRules {
                    rules,
                    lists: filters.len(),
                    compiled: WallClock.now(),
                };
                self.installed = Installed {
                    name,
                    filters,
                    summary,
                };
                RulesStatus::Active(summary)
            }
            Err(error) => RulesStatus::Failed(error),
        };
        if let Some(settled) = &self.settled {
            settled();
        }
    }
}

impl ContentBlockerPort for NativeBlocker {
    fn install(&mut self, config: &BlockerRules) -> Result<(), BlockerError> {
        self.cosmetic.install(config);
        let mut state = self.state.borrow_mut();
        let name = rules::fingerprint(config);
        if state.wanted.as_ref() == Some(&name) && !matches!(state.status, RulesStatus::Failed(_)) {
            return Ok(());
        }
        let (sender, source) = mpsc::channel();
        if state.installed.name == name || config.lists.is_empty() {
            let _ = sender.send(Ok(Vec::new()));
        } else {
            let config = config.clone();
            std::thread::Builder::new()
                .name("quickie-filter-rules".to_owned())
                .spawn(move || {
                    let _ = sender.send(rules::encode(&config, rules::MAX_RULES));
                })
                .map_err(|error| BlockerError(error.to_string()))?;
        }
        state.wanted = Some(name.clone());
        state.pending = Some(Compilation { name, source });
        state.status = RulesStatus::Compiling;
        Ok(())
    }

    fn status(&self) -> RulesStatus {
        self.state.borrow().status.clone()
    }

    fn cosmetics_ready(&self) -> bool {
        self.cosmetic.ready() && !self.cosmetic.failed()
    }

    fn on_settled(&mut self, settled: Box<dyn Fn()>) {
        self.state.borrow_mut().settled = Some(settled);
    }
}

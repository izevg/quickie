//! GTK delivery of the shared cosmetic filters and document-start scriptlets.

mod native;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_int;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use quickie_application::BlockerRules;
use quickie_domain_shields::Site;
use quickie_engine_webkit::cosmetic_rules::{
    Filters, HostBundle, HostTable, Report, lists_name, page_script,
};

use super::super::{App, connect, ffi};
use ffi::{Bool, Object};
use native::{Decision, Reply, Script};

const WAIT: Duration = Duration::from_millis(300);
const HOSTS: usize = 96;
const MAX_LOOKUPS: usize = 12;
const MAX_WAITING: usize = 128;
const MAX_REPORTS: usize = 32;

#[derive(Default)]
pub(super) struct Cosmetics(RefCell<State>);

struct State {
    lists: Option<String>,
    sites: Arc<[Site]>,
    filters: Option<Arc<Filters>>,
    building: Option<Receiver<Filters>>,
    failed: bool,
    hosts: HostTable,
    lookups: HashMap<String, Receiver<HostBundle>>,
    waiting: Vec<Navigation>,
    reports: Vec<PendingReply>,
    views: Vec<(Object, Object)>,
    scripts: Vec<Script>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            lists: None,
            sites: Arc::from([]),
            filters: None,
            building: None,
            failed: false,
            hosts: HostTable::new(HOSTS),
            lookups: HashMap::new(),
            waiting: Vec::new(),
            reports: Vec::new(),
            views: Vec::new(),
            scripts: Vec::new(),
        }
    }
}

struct Navigation {
    view: Object,
    host: String,
    deadline: Instant,
    decision: Decision,
}

struct PendingReply {
    result: Receiver<String>,
    reply: Reply,
}

impl Cosmetics {
    pub(super) fn install(&self, config: &BlockerRules) {
        let mut state = self.0.borrow_mut();
        let lists = lists_name(&config.lists);
        let changed = state.lists != lists || state.failed;
        if !changed && state.sites.as_ref() == config.unshielded {
            return;
        }
        state.sites = Arc::from(config.unshielded.clone());
        state.hosts.clear();
        state.lookups.clear();
        state.reports.clear();
        if changed {
            state.lists = lists;
            state.filters = None;
            state.failed = false;
            state.building = None;
            if state.lists.is_some() {
                let lists = config.lists.clone();
                state.building = worker("quickie-cosmetic-filters", move || Filters::build(&lists));
                state.failed = state.building.is_none();
            }
        }
        state.install_scripts();
    }

    pub(super) fn ready(&self) -> bool {
        self.0.borrow().building.is_none()
    }

    pub(super) fn failed(&self) -> bool {
        self.0.borrow().failed
    }

    pub(super) fn working(&self) -> bool {
        let state = self.0.borrow();
        state.building.is_some()
            || !state.lookups.is_empty()
            || !state.waiting.is_empty()
            || !state.reports.is_empty()
    }

    pub(super) fn observe(&self, view: Object, manager: Object, data: Object) {
        let mut state = self.0.borrow_mut();
        if !state.views.iter().any(|&(_, known)| known == manager) {
            native::register(manager, reported as *const _, data);
            for script in &state.scripts {
                script.add_to(manager);
            }
        }
        state.views.push((view, manager));
        connect(view, c"decide-policy", decide as *const _, data);
    }

    pub(super) fn forget(&self, view: Object) {
        let mut state = self.0.borrow_mut();
        let removed = state
            .views
            .iter()
            .find(|&&(known, _)| known == view)
            .map(|&(_, manager)| manager);
        state.views.retain(|&(known, _)| known != view);
        if let Some(manager) = removed
            && !state.views.iter().any(|&(_, known)| known == manager)
        {
            for script in &state.scripts {
                script.remove_from(manager);
            }
        }
        let decisions: Vec<_> = state
            .waiting
            .extract_if(.., |waiting| waiting.view == view)
            .collect();
        drop(state);
        for waiting in decisions {
            waiting.decision.discard();
        }
    }

    pub(super) fn stop(&self) {
        let mut state = self.0.borrow_mut();
        let waiting = std::mem::take(&mut state.waiting);
        state.reports.clear();
        drop(state);
        for item in waiting {
            item.decision.discard();
        }
    }

    pub(super) fn poll(&self) -> bool {
        let mut state = self.0.borrow_mut();
        let was_building = state.building.is_some();
        state.finish_build();
        let changed = state.finish_lookups();
        if changed || (was_building && state.building.is_none()) {
            state.install_scripts();
        }
        let waiting = state.ready_navigations();
        let replies = state.ready_replies();
        let finished = was_building && state.building.is_none();
        drop(state);
        for decision in waiting {
            decision.allow();
        }
        for (reply, result) in replies {
            reply.send(&result);
        }
        finished
    }

    fn prepare(&self, view: Object, host: String, decision: Object) -> bool {
        let mut state = self.0.borrow_mut();
        if (state.filters.is_none() && state.building.is_none())
            || state.hosts.get(&host).is_some()
            || state.waiting.len() >= MAX_WAITING
        {
            return false;
        }
        state.look_up(&host);
        if state.building.is_none() && !state.lookups.contains_key(&host) {
            return false;
        }
        state.waiting.push(Navigation {
            view,
            host,
            decision: Decision::hold(decision),
            deadline: Instant::now() + WAIT,
        });
        true
    }

    fn report(&self, manager: Object, report: Report, value: Object, reply: Object) -> bool {
        let mut state = self.0.borrow_mut();
        if state.reports.len() >= MAX_REPORTS
            || !state.views.iter().any(|&(_, known)| known == manager)
        {
            return false;
        }
        let Some(filters) = state.filters.clone() else {
            return false;
        };
        let bundle = state.hosts.get(&report.host);
        let sites = state.sites.clone();
        let result = worker("quickie-cosmetic-classes", move || {
            let bundle = bundle.unwrap_or_else(|| Arc::new(filters.lookup(&report.host, &sites)));
            filters.answer(&report, Some(&bundle))
        });
        let Some(result) = result else { return false };
        state.reports.push(PendingReply {
            result,
            reply: Reply::hold(value, reply),
        });
        true
    }
}

impl State {
    fn look_up(&mut self, host: &str) {
        if self.lookups.len() >= MAX_LOOKUPS || self.lookups.contains_key(host) {
            return;
        }
        let Some(filters) = self.filters.clone() else {
            return;
        };
        let sites = self.sites.clone();
        let address = host.to_owned();
        if let Some(result) = worker("quickie-cosmetic-lookup", move || {
            filters.lookup(&address, &sites)
        }) {
            self.lookups.insert(host.to_owned(), result);
        }
    }

    fn finish_build(&mut self) {
        let Some(building) = &self.building else {
            return;
        };
        match building.try_recv() {
            Ok(filters) => {
                self.filters = Some(Arc::new(filters));
                self.building = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.failed = true;
                self.building = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn finish_lookups(&mut self) -> bool {
        let mut changed = false;
        self.lookups.retain(|host, job| {
            let bundle = match job.try_recv() {
                Ok(bundle) => bundle,
                Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            };
            changed |= self.hosts.insert(host, Arc::new(bundle));
            false
        });
        changed
    }

    fn ready_navigations(&mut self) -> Vec<Decision> {
        let mut ready = Vec::new();
        for waiting in std::mem::take(&mut self.waiting) {
            if self.hosts.get(&waiting.host).is_none() && Instant::now() < waiting.deadline {
                self.look_up(&waiting.host);
                if self.lookups.contains_key(&waiting.host) || self.building.is_some() {
                    self.waiting.push(waiting);
                    continue;
                }
            }
            ready.push(waiting.decision);
        }
        ready
    }

    fn ready_replies(&mut self) -> Vec<(Reply, String)> {
        let mut ready = Vec::new();
        for pending in std::mem::take(&mut self.reports) {
            let result = match pending.result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Disconnected) => "[]".to_owned(),
                Err(TryRecvError::Empty) => {
                    self.reports.push(pending);
                    continue;
                }
            };
            ready.push((pending.reply, result));
        }
        ready
    }

    fn install_scripts(&mut self) {
        let mut managers = self
            .views
            .iter()
            .map(|&(_, manager)| manager)
            .collect::<Vec<_>>();
        managers.sort_unstable();
        managers.dedup();
        for script in self.scripts.drain(..) {
            for &manager in &managers {
                script.remove_from(manager);
            }
        }
        let Some(filters) = &self.filters else { return };
        for script in [
            page_script(&self.hosts).and_then(|source| Script::new(&source, false)),
            Script::new(&filters.script(&self.hosts), true),
        ]
        .into_iter()
        .flatten()
        {
            for &manager in &managers {
                script.add_to(manager);
            }
            self.scripts.push(script);
        }
    }
}

fn worker<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<Receiver<T>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .ok()?;
    Some(receiver)
}

extern "C" fn decide(view: Object, decision: Object, kind: c_int, data: Object) -> Bool {
    if kind != native::NAVIGATION {
        return ffi::FALSE;
    }
    let Some(url) =
        native::navigation(decision).filter(|url| matches!(url.scheme(), "http" | "https"))
    else {
        return ffi::FALSE;
    };
    let Some(host) = url.host_str() else {
        return ffi::FALSE;
    };
    // SAFETY: This view uses the live app as data on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if !app
        .privacy
        .engine
        .cosmetic
        .prepare(view, host.to_owned(), decision)
    {
        return ffi::FALSE;
    }
    super::kick(app);
    ffi::TRUE
}

extern "C" fn reported(manager: Object, value: Object, reply: Object, data: Object) -> Bool {
    let Some(report) = native::text(value).as_deref().and_then(Report::parse) else {
        return ffi::FALSE;
    };
    // SAFETY: The content manager uses the original live App on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if !app
        .privacy
        .engine
        .cosmetic
        .report(manager, report, value, reply)
    {
        return ffi::FALSE;
    }
    super::kick(app);
    ffi::TRUE
}

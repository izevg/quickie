//! The development engine's content blocker: the application's [`ContentBlockerPort`] on
//! `WKContentRuleList`s, installed in the user content controller every page shares.
//!
//! Installing never blocks the main thread. The filter lists are converted on a worker thread;
//! `WKContentRuleListStore` compiles the result on its own queue and answers on the main thread.
//! Compiled lists stay in the store under a name made from the rules, so the next launch with
//! the same rules finds them instead of compiling again. Every install supersedes the ones still
//! in flight; their results are dropped when they arrive.

mod rules;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread;

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKUserContentController};
use quickie_application::{BlockerError, BlockerRules, ContentBlockerPort};

use rules::{MAX_RULES, PREFIX, cached, encode, fingerprint, identifier};

/// What the blocker is doing about the latest rules it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockerStatus {
    /// It was never given any.
    Idle,
    /// Converting or compiling them; pages load with the previous rules meanwhile.
    Compiling,
    /// Pages load with them, compiled into this many rule lists (none when there is nothing to
    /// block).
    Installed(usize),
    /// They could not be compiled; pages keep the previous rules.
    Failed(String),
}

/// Blocks requests of every page of a [`crate::WebKitEngine`], by the rules it was last given.
/// Lives on the main thread.
pub struct WebKitContentBlocker {
    state: Rc<State>,
}

impl std::fmt::Debug for WebKitContentBlocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebKitContentBlocker")
            .field("status", &*self.state.status.borrow())
            .finish_non_exhaustive()
    }
}

impl WebKitContentBlocker {
    /// A blocker that keeps compiled rules in `store` and installs them in `controller`.
    pub(crate) fn new(
        store: Retained<WKContentRuleListStore>,
        controller: Retained<WKUserContentController>,
        mtm: MainThreadMarker,
    ) -> Self {
        Self {
            state: Rc::new(State {
                mtm,
                store,
                controller,
                generation: Cell::new(0),
                wanted: RefCell::new(None),
                status: RefCell::new(BlockerStatus::Idle),
                on_settled: RefCell::new(None),
            }),
        }
    }

    /// What became of the latest rules.
    #[must_use]
    pub fn status(&self) -> BlockerStatus {
        self.state.status.borrow().clone()
    }

    /// Sets what runs on the main thread each time the latest rules are installed or fail.
    pub fn on_settled(&mut self, settled: impl Fn() + 'static) {
        *self.state.on_settled.borrow_mut() = Some(Rc::new(settled));
    }
}

impl ContentBlockerPort for WebKitContentBlocker {
    fn install(&mut self, rules: &BlockerRules) -> Result<(), BlockerError> {
        let fingerprint = fingerprint(rules);
        if self.state.wanted.borrow().as_deref() == Some(fingerprint.as_str()) {
            return Ok(());
        }
        State::begin(&self.state, rules.clone(), fingerprint);
        Ok(())
    }
}

/// A rule list document converted off the main thread.
struct Document(Retained<NSString>);

// SAFETY: the string is created immutable by `NSString::from_str` and never mutated; Foundation
// documents immutable strings as safe to use from any thread, and only the main thread uses it
// after the hand-over.
unsafe impl Send for Document {}

struct State {
    mtm: MainThreadMarker,
    store: Retained<WKContentRuleListStore>,
    controller: Retained<WKUserContentController>,
    /// Counts installs; work started for an earlier one is dropped.
    generation: Cell<u64>,
    /// The fingerprint of the rules installed or on their way, `None` after a failure so that
    /// the same rules are tried again.
    wanted: RefCell<Option<String>>,
    status: RefCell<BlockerStatus>,
    on_settled: RefCell<Option<Rc<dyn Fn()>>>,
}

/// Rule lists arriving one by one from the store, kept in their order.
struct Gathering {
    generation: u64,
    fingerprint: String,
    lists: RefCell<Vec<Option<Retained<WKContentRuleList>>>>,
    missing: Cell<usize>,
    /// Applied or given up; later answers are ignored.
    over: Cell<bool>,
}

/// What a store call does when one rule list could not be had.
type OnMissing = Rc<dyn Fn(&Rc<State>, String)>;

impl State {
    /// Starts installing `rules`: from the store when it already compiled them, else by
    /// converting and compiling them.
    fn begin(this: &Rc<Self>, rules: BlockerRules, fingerprint: String) {
        let generation = this.generation.get().wrapping_add(1);
        this.generation.set(generation);
        *this.wanted.borrow_mut() = Some(fingerprint.clone());
        if rules.lists.is_empty() {
            Self::apply(this, generation, &fingerprint, &[]);
            return;
        }
        *this.status.borrow_mut() = BlockerStatus::Compiling;
        let weak = Rc::downgrade(this);
        let pending = RefCell::new(Some(rules));
        let block = RcBlock::new(move |ids: *mut NSArray<NSString>| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let Some(rules) = pending.borrow_mut().take() else {
                return;
            };
            if !state.is_current(generation) {
                return;
            }
            match cached(&names(ids), &fingerprint) {
                Some(count) => Self::look_up(&state, generation, &fingerprint, count, rules),
                None => Self::convert(&state, generation, fingerprint.clone(), rules),
            }
        });
        let store = &this.store;
        // SAFETY: the store calls the block once, on the main thread.
        unsafe { store.getAvailableContentRuleListIdentifiers(Some(&block)) };
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.get() == generation
    }

    /// Takes the compiled lists from the store; if one is missing after all, compiles anew.
    fn look_up(
        this: &Rc<Self>,
        generation: u64,
        fingerprint: &str,
        count: usize,
        rules: BlockerRules,
    ) {
        let gathering = Gathering::new(generation, fingerprint, count);
        let wanted = fingerprint.to_owned();
        let on_missing: OnMissing = Rc::new(move |state, _| {
            Self::convert(state, generation, wanted.clone(), rules.clone());
        });
        for index in 0..count {
            let id = NSString::from_str(&identifier(fingerprint, index, count));
            let block = Self::collector(this, &gathering, index, Rc::clone(&on_missing));
            // SAFETY: the store calls the block once, on the main thread.
            unsafe {
                this.store
                    .lookUpContentRuleListForIdentifier_completionHandler(Some(&id), Some(&block));
            }
        }
    }

    /// Converts the lists on a worker thread, then compiles the result on the main thread.
    fn convert(this: &Rc<Self>, generation: u64, fingerprint: String, rules: BlockerRules) {
        *this.status.borrow_mut() = BlockerStatus::Compiling;
        let state = MainThreadBound::new(Rc::downgrade(this), this.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-content-rules".to_owned())
            .spawn(move || {
                let documents = encode(&rules, MAX_RULES).map(|documents| {
                    documents
                        .iter()
                        .map(|json| Document(NSString::from_str(json)))
                        .collect::<Vec<_>>()
                });
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(state) = state.into_inner(mtm).upgrade() {
                        Self::compile(&state, generation, &fingerprint, documents);
                    }
                });
            });
        if let Err(error) = spawned {
            this.fail(generation, &format!("cannot start converting: {error}"));
        }
    }

    /// Hands each converted document to the store to compile.
    fn compile(
        this: &Rc<Self>,
        generation: u64,
        fingerprint: &str,
        documents: Result<Vec<Document>, String>,
    ) {
        if !this.is_current(generation) {
            return;
        }
        let documents = match documents {
            Ok(documents) => documents,
            Err(error) => return this.fail(generation, &error),
        };
        if documents.is_empty() {
            return Self::apply(this, generation, fingerprint, &[]);
        }
        let count = documents.len();
        let gathering = Gathering::new(generation, fingerprint, count);
        let on_missing: OnMissing = Rc::new(move |state, error| state.fail(generation, &error));
        for (index, Document(json)) in documents.iter().enumerate() {
            let id = NSString::from_str(&identifier(fingerprint, index, count));
            let block = Self::collector(this, &gathering, index, Rc::clone(&on_missing));
            // SAFETY: the store calls the block once, on the main thread; it copies the JSON.
            unsafe {
                this.store
                    .compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                        Some(&id),
                        Some(json),
                        Some(&block),
                    );
            }
        }
    }

    /// The completion block of one store call for rule list `index`: keeps the list, and once
    /// all are in, installs them; on the first one missing, runs `on_missing` instead.
    fn collector(
        this: &Rc<Self>,
        gathering: &Rc<Gathering>,
        index: usize,
        on_missing: OnMissing,
    ) -> RcBlock<dyn Fn(*mut WKContentRuleList, *mut NSError)> {
        let weak = Rc::downgrade(this);
        let gathering = Rc::clone(gathering);
        RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if gathering.over.get() || !state.is_current(gathering.generation) {
                return;
            }
            // SAFETY: WebKit passes a valid rule list or nil; retaining keeps it past the call.
            let Some(list) = (unsafe { Retained::retain(list) }) else {
                gathering.over.set(true);
                return on_missing(&state, describe(error));
            };
            if let Some(slot) = gathering.lists.borrow_mut().get_mut(index) {
                *slot = Some(list);
            }
            gathering
                .missing
                .set(gathering.missing.get().saturating_sub(1));
            if gathering.missing.get() == 0 {
                gathering.over.set(true);
                let lists: Vec<_> = gathering.lists.take().into_iter().flatten().collect();
                Self::apply(&state, gathering.generation, &gathering.fingerprint, &lists);
            }
        })
    }

    /// Makes `lists` the rules of every page, and drops what the store keeps of earlier rules.
    fn apply(
        this: &Rc<Self>,
        generation: u64,
        fingerprint: &str,
        lists: &[Retained<WKContentRuleList>],
    ) {
        // SAFETY: the controller is valid, on the main thread it belongs to.
        unsafe { this.controller.removeAllContentRuleLists() };
        for list in lists {
            // SAFETY: the controller and the list are valid, on the main thread they belong to.
            unsafe { this.controller.addContentRuleList(list) };
        }
        *this.status.borrow_mut() = BlockerStatus::Installed(lists.len());
        Self::prune(this, generation, fingerprint);
        this.settle();
    }

    /// Removes this adapter's compiled lists of other rules from the store, unless newer rules
    /// arrived meanwhile.
    fn prune(this: &Rc<Self>, generation: u64, fingerprint: &str) {
        let weak = Rc::downgrade(this);
        let keep = format!("{PREFIX}{fingerprint}-");
        let block = RcBlock::new(move |ids: *mut NSArray<NSString>| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if !state.is_current(generation) {
                return;
            }
            let removed = RcBlock::new(|_error: *mut NSError| {});
            for id in names(ids) {
                if id.starts_with(PREFIX) && !id.starts_with(&keep) {
                    // SAFETY: the store calls the block once; it ignores the outcome.
                    unsafe {
                        state
                            .store
                            .removeContentRuleListForIdentifier_completionHandler(
                                Some(&NSString::from_str(&id)),
                                Some(&removed),
                            );
                    }
                }
            }
        });
        let store = &this.store;
        // SAFETY: the store calls the block once, on the main thread.
        unsafe { store.getAvailableContentRuleListIdentifiers(Some(&block)) };
    }

    fn fail(&self, generation: u64, reason: &str) {
        if !self.is_current(generation) {
            return;
        }
        *self.wanted.borrow_mut() = None;
        *self.status.borrow_mut() = BlockerStatus::Failed(reason.to_owned());
        self.settle();
    }

    fn settle(&self) {
        let settled = self.on_settled.borrow().clone();
        if let Some(settled) = settled {
            settled();
        }
    }
}

impl Gathering {
    fn new(generation: u64, fingerprint: &str, count: usize) -> Rc<Self> {
        Rc::new(Self {
            generation,
            fingerprint: fingerprint.to_owned(),
            lists: RefCell::new(vec![None; count]),
            missing: Cell::new(count),
            over: Cell::new(false),
        })
    }
}

/// The identifiers a store listed.
fn names(ids: *mut NSArray<NSString>) -> Vec<String> {
    // SAFETY: WebKit passes a valid array or nil, alive for the duration of the call.
    unsafe { ids.as_ref() }.map_or_else(Vec::new, |ids| {
        ids.to_vec().iter().map(ToString::to_string).collect()
    })
}

/// A store error's description, or a fallback when `WebKit` gave none.
fn describe(error: *mut NSError) -> String {
    // SAFETY: WebKit passes a valid error or nil, alive for the duration of the call.
    unsafe { error.as_ref() }.map_or_else(
        || "the content rule list store gave no rule list".to_owned(),
        |error| error.localizedDescription().to_string(),
    )
}

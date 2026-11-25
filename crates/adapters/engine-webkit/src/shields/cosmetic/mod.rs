//! Cosmetic filtering and scriptlets: what content rule lists cannot do.
//!
//! Network rules cannot keep ads out of a page that serves them from its own domain, as video
//! sites do; the lists answer those with cosmetic filters (hide this, run that procedure) and
//! scriptlets (small scripts that change the page's own objects before its scripts run, such as
//! `json-prune` on a player's ad placements). `adblock` says what each host gets; Quickie runs
//! it with two user scripts in each controller pages share (the main jar's and the throwaway
//! jars'):
//!
//! - **Page world**, at document start, in every frame: each known host's scriptlet calls and
//!   Quickie's own scriptlet library , evaluated only on a host that has some.
//!   Scriptlets must patch the page's own `JSON.parse`, `fetch`, and globals, so they run where
//!   the page runs, as in uBlock Origin. Nothing in them talks to Quickie.
//! - **Shields world** (Quickie's own content world for shields), at document start, in every
//!   frame: the generic and the host's own hiding rules as a constructed stylesheet, the
//!   procedural filters, and a batched report of the classes and ids the page uses. Only this
//!   world has the message handler; it answers with the generic rules for those classes and
//!   ids (`hidden_class_id_selectors`).
//!
//! Lookups never block the main thread: the filters are built, and each host is looked up, on
//! worker threads; hosts are cached ([`HostTable`]). A navigation asks for its host
//! when it starts and waits, at most [`WAIT`], only between its response and its document, so
//! the scripts are in place before the page's first script runs.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use block2::{DynBlock, RcBlock};
use dispatch2::{DispatchQueue, DispatchTime, MainThreadBound};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSString;
use objc2_web_kit::{
    WKContentWorld, WKScriptMessage, WKScriptMessageHandlerWithReply, WKUserContentController,
    WKUserScript, WKUserScriptInjectionTime,
};
use quickie_application::BlockerRules;
use quickie_domain_shields::Site;
use url::Url;

use crate::cosmetic_rules::{
    CHANNEL, Filters, HostBundle, HostTable, Report, lists_name, page_script,
};

/// How long a document waits for its host's filters before it loads without them.
pub(crate) const WAIT: Duration = Duration::from_millis(300);

/// How many hosts' filters are kept, and so carried by the scripts.
const HOSTS: usize = 96;

/// What is done when a host's filters are known.
type Then = Box<dyn FnOnce()>;

/// The cosmetic side of the shields of every page of an engine. Lives on the main thread.
pub(crate) struct Cosmetics {
    mtm: MainThreadMarker,
    /// The main jar's controller and the throwaway jars' one: the same scripts in each.
    controllers: Vec<Retained<WKUserContentController>>,
    world: Retained<WKContentWorld>,
    filters: RefCell<Option<Arc<Filters>>>,
    /// Names the lists the filters are built from, `None` without lists.
    lists: RefCell<Option<String>>,
    unshielded: RefCell<Arc<[Site]>>,
    /// Counts builds; a build or lookup started for an earlier one is dropped.
    generation: Cell<u64>,
    table: RefCell<HostTable>,
    /// Documents waiting for a host's filters.
    waiting: RefCell<HashMap<String, Vec<Then>>>,
    /// The user scripts this installed last, to take them out again.
    installed: RefCell<Vec<Retained<WKUserScript>>>,
}

impl Cosmetics {
    /// Cosmetic shields for every page of each of `controllers`; nothing is hidden until
    /// [`Self::install`] has lists.
    pub(crate) fn new(
        controllers: Vec<Retained<WKUserContentController>>,
        mtm: MainThreadMarker,
    ) -> Rc<Self> {
        // SAFETY: a named world is created on first use and shared while it lives.
        let world =
            unsafe { WKContentWorld::worldWithName(&NSString::from_str("quickie-shields"), mtm) };
        let this = Rc::new(Self {
            mtm,
            controllers,
            world,
            filters: RefCell::new(None),
            lists: RefCell::new(None),
            unshielded: RefCell::new(Arc::from(Vec::new())),
            generation: Cell::new(0),
            table: RefCell::new(HostTable::new(HOSTS)),
            waiting: RefCell::new(HashMap::new()),
            installed: RefCell::new(Vec::new()),
        });
        for controller in &this.controllers {
            let handler = ShieldsHandler::new(Rc::downgrade(&this), mtm);
            // SAFETY: the controller retains the handler for as long as it lives; this is the
            // only handler under the name in the shields world, which only this module uses,
            // and the engine makes one `Cosmetics` for its controllers.
            unsafe {
                controller.addScriptMessageHandlerWithReply_contentWorld_name(
                    ProtocolObject::from_ref(&*handler),
                    &this.world,
                    &NSString::from_str(CHANNEL),
                );
            }
        }
        this
    }

    /// Whether the filters of the current lists are built (or there are no lists).
    pub(crate) fn ready(&self) -> bool {
        self.lists.borrow().is_none() || self.filters.borrow().is_some()
    }

    /// Makes the filters those of `rules`: built again when the lists changed, looked up again
    /// when the unshielded sites changed.
    pub(crate) fn install(this: &Rc<Self>, rules: &BlockerRules) {
        let unshielded: Arc<[Site]> = Arc::from(rules.unshielded.clone());
        let sites_changed = *this.unshielded.borrow() != unshielded;
        *this.unshielded.borrow_mut() = unshielded;
        let lists = lists_name(&rules.lists);
        if *this.lists.borrow() == lists {
            if sites_changed {
                this.table.borrow_mut().clear();
                this.reinstall_scripts();
            }
            return;
        }
        let generation = this.generation.get().wrapping_add(1);
        this.generation.set(generation);
        this.lists.borrow_mut().clone_from(&lists);
        *this.filters.borrow_mut() = None;
        this.table.borrow_mut().clear();
        this.reinstall_scripts();
        this.release_all();
        if lists.is_none() {
            return;
        }
        let state = MainThreadBound::new(Rc::downgrade(this), this.mtm);
        let lists = rules.lists.clone();
        let spawned = thread::Builder::new()
            .name("quickie-cosmetic-filters".to_owned())
            .spawn(move || {
                let built = catch_unwind(AssertUnwindSafe(|| Filters::build(&lists))).ok();
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let (Some(this), Some(filters)) = (state.into_inner(mtm).upgrade(), built) {
                        this.built(generation, filters);
                    }
                });
            });
        drop(spawned);
    }

    fn built(&self, generation: u64, filters: Filters) {
        if self.generation.get() != generation {
            return;
        }
        *self.filters.borrow_mut() = Some(Arc::new(filters));
        self.table.borrow_mut().clear();
        self.reinstall_scripts();
    }

    /// Starts looking up the host of a navigation that just started, so its filters are likely
    /// known by the time its document arrives.
    pub(crate) fn expect(this: &Rc<Self>, url: &Url) {
        Self::prepare(this, url, Box::new(|| {}));
    }

    /// Runs `then` once the filters of `url`'s host are in the scripts, or at once when they
    /// already are, when there are none, or when `url` is not a web page; after [`WAIT`] at
    /// most.
    pub(crate) fn prepare(this: &Rc<Self>, url: &Url, then: Then) {
        let (Some(host), true) = (url.host_str(), matches!(url.scheme(), "http" | "https")) else {
            return then();
        };
        let Some(filters) = this.filters.borrow().clone() else {
            return then();
        };
        if this.table.borrow_mut().get(host).is_some() {
            return then();
        }
        let first = {
            let mut waiting = this.waiting.borrow_mut();
            let queue = waiting.entry(host.to_owned()).or_default();
            queue.push(then);
            queue.len() == 1
        };
        if !first {
            return;
        }
        Self::look_up(this, host.to_owned(), filters);
        let state = MainThreadBound::new(Rc::downgrade(this), this.mtm);
        let late = host.to_owned();
        if let Ok(when) = DispatchTime::try_from(WAIT) {
            let _ = DispatchQueue::main().after(when, move || {
                if let Some(mtm) = MainThreadMarker::new()
                    && let Some(this) = state.into_inner(mtm).upgrade()
                {
                    this.release(&late);
                }
            });
        }
    }

    fn look_up(this: &Rc<Self>, host: String, filters: Arc<Filters>) {
        let generation = this.generation.get();
        let sites = Arc::clone(&this.unshielded.borrow());
        let state = MainThreadBound::new(Rc::downgrade(this), this.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-cosmetic-lookup".to_owned())
            .spawn(move || {
                let bundle = catch_unwind(AssertUnwindSafe(|| filters.lookup(&host, &sites)))
                    .unwrap_or_default();
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(this) = state.into_inner(mtm).upgrade() {
                        this.looked_up(generation, &host, bundle);
                    }
                });
            });
        if spawned.is_err() {
            // No worker: the documents waiting load without this host's filters.
            this.release_all();
        }
    }

    fn looked_up(&self, generation: u64, host: &str, bundle: HostBundle) {
        if self.generation.get() == generation
            && self.table.borrow_mut().insert(host, Arc::new(bundle))
        {
            self.reinstall_scripts();
        }
        self.release(host);
    }

    /// Lets the documents waiting for `host` go on.
    fn release(&self, host: &str) {
        let waiting = self.waiting.borrow_mut().remove(host).unwrap_or_default();
        for then in waiting {
            then();
        }
    }

    fn release_all(&self) {
        let waiting: Vec<Then> = self
            .waiting
            .borrow_mut()
            .drain()
            .flat_map(|(_, queue)| queue)
            .collect();
        for then in waiting {
            then();
        }
    }

    /// Answers a frame's report of classes and ids with the generic rules that hide them.
    fn hidden(
        this: &Rc<Self>,
        report: Report,
        reply: RcBlock<dyn Fn(*mut AnyObject, *mut NSString)>,
    ) {
        let answer_now =
            |reply: &RcBlock<dyn Fn(*mut AnyObject, *mut NSString)>| answer(reply, "[]");
        let Some(filters) = this.filters.borrow().clone() else {
            return answer_now(&reply);
        };
        let bundle = this.table.borrow_mut().get(&report.host);
        let reply = MainThreadBound::new(reply, this.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-cosmetic-classes".to_owned())
            .spawn(move || {
                let json = catch_unwind(AssertUnwindSafe(|| {
                    filters.answer(&report, bundle.as_deref())
                }))
                .unwrap_or_else(|_| "[]".to_owned());
                DispatchQueue::main().exec_async(move || {
                    if let Some(mtm) = MainThreadMarker::new() {
                        answer(&reply.into_inner(mtm), &json);
                    }
                });
            });
        drop(spawned);
    }

    /// Replaces this module's user scripts in the controllers with ones made from what is known
    /// now; every other script (Quickie's own, an extension's content scripts) stays, in its
    /// order: `WebKit` has no public call that removes one script, so all go and the others
    /// come back as the same objects.
    fn reinstall_scripts(&self) {
        let mut installed = self.installed.borrow_mut();
        if !installed.is_empty() {
            for controller in &self.controllers {
                // SAFETY: a getter; the array is a copy.
                let all = unsafe { controller.userScripts() };
                // SAFETY: the controller is valid, on the main thread it belongs to.
                unsafe { controller.removeAllUserScripts() };
                for script in all.to_vec() {
                    if !installed
                        .iter()
                        .any(|ours| Retained::as_ptr(ours) == Retained::as_ptr(&script))
                    {
                        // SAFETY: the script came from this controller and is still valid.
                        unsafe { controller.addUserScript(&script) };
                    }
                }
            }
            installed.clear();
        }
        let Some(filters) = self.filters.borrow().clone() else {
            return;
        };
        let table = self.table.borrow();
        // SAFETY: the page world always exists.
        let page = unsafe { WKContentWorld::pageWorld(self.mtm) };
        let scripts = page_script(&table)
            .map(|source| (source, page))
            .into_iter()
            .chain([(filters.script(&table), self.world.clone())]);
        for (source, world) in scripts {
            // SAFETY: the source is a complete script, injected at document start into every
            // frame of the given world.
            let script = unsafe {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                    WKUserScript::alloc(self.mtm),
                    &NSString::from_str(&source),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    false,
                    &world,
                )
            };
            for controller in &self.controllers {
                // SAFETY: the controller retains the script; a script may be in several.
                unsafe { controller.addUserScript(&script) };
            }
            installed.push(script);
        }
    }
}

/// Fulfils a script's message with `json`.
fn answer(reply: &RcBlock<dyn Fn(*mut AnyObject, *mut NSString)>, json: &str) {
    let text = NSString::from_str(json);
    let object: *const AnyObject = std::ptr::from_ref::<NSString>(&text).cast();
    reply.call((object.cast_mut(), std::ptr::null_mut()));
}

pub(crate) struct HandlerIvars {
    cosmetics: Weak<Cosmetics>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ShieldsHandler` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitShieldsHandler"]
    #[ivars = HandlerIvars]
    pub(crate) struct ShieldsHandler;

    unsafe impl NSObjectProtocol for ShieldsHandler {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl WKScriptMessageHandlerWithReply for ShieldsHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        fn did_receive(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
            reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>,
        ) {
            let reply = reply.copy();
            // SAFETY: a getter; the body is whatever the script posted.
            let body = unsafe { message.body() };
            let report = body
                .downcast_ref::<NSString>()
                .and_then(|text| Report::parse(&text.to_string()));
            match (report, self.ivars().cosmetics.upgrade()) {
                (Some(report), Some(cosmetics)) => Cosmetics::hidden(&cosmetics, report, reply),
                _ => answer(&reply, "[]"),
            }
        }
    }
);

impl ShieldsHandler {
    fn new(cosmetics: Weak<Cosmetics>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HandlerIvars { cosmetics });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

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
//!   Quickie's own scriptlet library ([`library`]), evaluated only on a host that has some.
//!   Scriptlets must patch the page's own `JSON.parse`, `fetch`, and globals, so they run where
//!   the page runs, as in uBlock Origin. Nothing in them talks to Quickie.
//! - **Shields world** (Quickie's own content world for shields), at document start, in every
//!   frame: the generic and the host's own hiding rules as a constructed stylesheet, the
//!   procedural filters, and a batched report of the classes and ids the page uses. Only this
//!   world has the message handler; it answers with the generic rules for those classes and
//!   ids (`hidden_class_id_selectors`).
//!
//! Lookups never block the main thread: the filters are built, and each host is looked up, on
//! worker threads; hosts are cached ([`bundle::HostTable`]). A navigation asks for its host
//! when it starts and waits, at most [`WAIT`], only between its response and its document, so
//! the scripts are in place before the page's first script runs.

mod bundle;
mod library;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};
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
use quickie_application::{BlockerRules, ListRules};
use quickie_domain_shields::{ContentHash, Site};
use serde_json::Value;
use url::Url;

use bundle::{HostBundle, HostTable, cosmetic_script, page_script, unshielded};

/// How long a document waits for its host's filters before it loads without them.
pub(crate) const WAIT: Duration = Duration::from_millis(300);

/// How many hosts' filters are kept, and so carried by the scripts.
const HOSTS: usize = 96;

/// The name the shields world's script posts its classes and ids under.
const HANDLER: &str = "quickieShields";

/// The most classes or ids one report is read for, and the longest name read: the page writes
/// them, so they are untrusted and bounded.
const MOST_NAMES: usize = 4096;
const LONGEST_NAME: usize = 256;

/// What is done when a host's filters are known.
type Then = Box<dyn FnOnce()>;

/// The cosmetic filters of the enabled lists, built off the main thread and read from any.
pub(crate) struct Filters {
    engine: Engine,
    /// The selectors every host hides, unless it excepts them.
    generic: BTreeSet<String>,
}

impl Filters {
    /// Builds the filters of `lists`: only their cosmetic rules and the `$generichide`
    /// exceptions, since `WebKit`'s content rule lists do the network blocking. Lists that may
    /// use the trusted scriptlets get their permission. Slow: a worker thread runs it.
    fn build(lists: &[ListRules]) -> Self {
        let mut set = FilterSet::new(false);
        for list in lists {
            let text = String::from_utf8_lossy(&list.body);
            let cosmetic: String = text
                .lines()
                .filter(|line| {
                    line.contains('#') || line.contains("generichide") || line.contains("ghide")
                })
                .flat_map(|line| [line, "\n"])
                .collect();
            let options = ParseOptions {
                permissions: if list.list.trusted() {
                    library::TRUSTED
                } else {
                    adblock::resources::PermissionMask::default()
                },
                ..ParseOptions::default()
            };
            set.add_filter_list(cosmetic, options);
        }
        let mut engine = Engine::new_with_filter_set(set);
        engine.use_resources(library::resources());
        let generic = engine
            .url_cosmetic_resources("https://quickie.invalid/")
            .hide_selectors
            .into_iter()
            .collect();
        Self { engine, generic }
    }

    /// What `host` gets.
    fn lookup(&self, host: &str, unshielded_sites: &[Site]) -> HostBundle {
        if unshielded(host, unshielded_sites) {
            return HostBundle::off();
        }
        let resources = self
            .engine
            .url_cosmetic_resources(&format!("https://{host}/"));
        HostBundle::of(resources, &self.generic)
    }

    /// The generic rules that hide elements with these classes or ids.
    fn hidden(
        &self,
        classes: &[String],
        ids: &[String],
        exceptions: &HashSet<String>,
    ) -> Vec<String> {
        self.engine
            .hidden_class_id_selectors(classes, ids, exceptions)
    }
}

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
                    &NSString::from_str(HANDLER),
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
        if bundle
            .as_ref()
            .is_some_and(|bundle| bundle.off || bundle.generichide)
        {
            return answer_now(&reply);
        }
        let exceptions = bundle.map_or_else(HashSet::new, |bundle| bundle.exceptions.clone());
        let reply = MainThreadBound::new(reply, this.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-cosmetic-classes".to_owned())
            .spawn(move || {
                let selectors = catch_unwind(AssertUnwindSafe(|| {
                    filters.hidden(&report.classes, &report.ids, &exceptions)
                }))
                .unwrap_or_default();
                let json = Value::from(selectors).to_string();
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
            .chain([(
                cosmetic_script(&filters.generic, &table),
                self.world.clone(),
            )]);
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

/// Names the lists filters are built from; `None` for no lists.
fn lists_name(lists: &[ListRules]) -> Option<String> {
    if lists.is_empty() {
        return None;
    }
    let text: Vec<String> = lists
        .iter()
        .map(|list| format!("{}={}", list.list, list.hash))
        .collect();
    Some(ContentHash::of(text.join(" ").as_bytes()).to_string())
}

/// What a frame reported: the classes and ids it saw since its last report.
struct Report {
    host: String,
    classes: Vec<String>,
    ids: Vec<String>,
}

impl Report {
    /// Reads a report as the shields script posts it: JSON text. Bounded, as the page wrote
    /// the names.
    fn parse(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let names = |key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|name| name.len() <= LONGEST_NAME)
                        .take(MOST_NAMES)
                        .map(str::to_owned)
                        .collect()
                })
        };
        Some(Self {
            host: value.get("host")?.as_str()?.to_owned(),
            classes: names("classes"),
            ids: names("ids"),
        })
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use quickie_application::ListRules;
    use quickie_domain_shields::{ContentHash, FilterListId, Site};

    use super::{Filters, LONGEST_NAME, Report, lists_name};

    fn list(id: FilterListId, text: &str) -> ListRules {
        ListRules {
            list: id,
            hash: ContentHash::of(text.as_bytes()),
            body: Arc::from(text.as_bytes()),
        }
    }

    const LIST: &str = "##div[data-ad]\n\
                        ##.generic-ad\n\
                        video.test##.host-ad\n\
                        video.test##+js(json-prune, adPlacements)\n\
                        video.test##+js(trusted-replace-fetch-response, ads, x)\n\
                        quiet.test#@#div[data-ad]\n\
                        quiet.test#@#.generic-ad\n\
                        @@||plain.test^$generichide\n\
                        ||tracker.test^\n";

    #[test]
    fn a_host_gets_its_own_rules_and_the_scriptlets_its_list_may_run() {
        let filters = Filters::build(&[list(FilterListId::EasyList, LIST)]);

        let bundle = filters.lookup("www.video.test", &[]);

        assert_eq!(bundle.css, [".host-ad{display:none!important}"]);
        assert!(
            bundle
                .scriptlets
                .contains("quickie_json_prune(\"adPlacements\")")
        );
        assert!(
            !bundle.scriptlets.contains("trusted"),
            "{}",
            bundle.scriptlets
        );
    }

    #[test]
    fn a_trusted_list_may_run_the_trusted_scriptlets() {
        let filters = Filters::build(&[list(FilterListId::UblockFilters, LIST)]);

        let bundle = filters.lookup("video.test", &[]);

        assert!(
            bundle
                .scriptlets
                .contains("quickie_trusted_replace_fetch_response(")
        );
    }

    #[test]
    fn generic_rules_apply_everywhere_but_where_they_are_excepted() {
        let filters = Filters::build(&[list(FilterListId::EasyList, LIST)]);

        assert!(filters.generic.contains("div[data-ad]"));
        let quiet = filters.lookup("quiet.test", &[]);
        assert_eq!(quiet.remove, ["div[data-ad]"]);
        let classes = [String::from("generic-ad")];
        assert_eq!(
            filters.hidden(&classes, &[], &HashSet::new()),
            [".generic-ad"]
        );
        assert_eq!(
            filters.hidden(&classes, &[], &quiet.exceptions),
            Vec::<String>::new()
        );
        assert!(filters.lookup("plain.test", &[]).generichide);
        assert!(!filters.lookup("other.test", &[]).differs());
    }

    #[test]
    fn a_procedural_filter_reaches_the_cosmetic_script_as_json() {
        let filters = Filters::build(&[list(
            FilterListId::EasyList,
            "localhost##div:has-text(Sponsored)\n",
        )]);

        let bundle = filters.lookup("localhost", &[]);

        assert_eq!(
            bundle.procedural,
            [
                r#"{"selector":[{"type":"css-selector","arg":"div"},{"type":"has-text","arg":"Sponsored"}]}"#
            ]
        );
    }

    // The Shorts feed's ad rules as uBlock filters and EasyList write them (November 2026).
    const SHORTS: &str = "www.youtube.com##+js(json-prune-fetch-response, \
        reelWatchSequenceResponse.entries.[-].command.reelWatchEndpoint.adClientParams.isAd \
        entries.[-].command.reelWatchEndpoint.adClientParams.isAd, , propsToMatch, \
        url:/reel_watch_sequence?)\n\
        www.youtube.com##+js(json-prune, entries.[-].command.reelWatchEndpoint.adClientParams.isAd)\n\
        youtube.com###shorts-inner-container > .ytd-shorts:has(> .ytd-reel-video-renderer > ytd-ad-slot-renderer)\n";

    #[test]
    fn the_shorts_feed_s_ad_rules_reach_the_host_intact() {
        let filters = Filters::build(&[list(FilterListId::UblockFilters, SHORTS)]);

        let bundle = filters.lookup("www.youtube.com", &[]);

        assert!(
            bundle.scriptlets.contains(
                "quickie_json_prune_fetch_response(\"reelWatchSequenceResponse.entries.[-].command.reelWatchEndpoint.adClientParams.isAd entries.[-].command.reelWatchEndpoint.adClientParams.isAd\", \"\", \"propsToMatch\", \"url:/reel_watch_sequence?\")"
            ),
            "{}",
            bundle.scriptlets
        );
        assert!(
            bundle.scriptlets.contains(
                "quickie_json_prune(\"entries.[-].command.reelWatchEndpoint.adClientParams.isAd\")"
            ),
            "{}",
            bundle.scriptlets
        );
        assert_eq!(
            bundle.css,
            [
                "#shorts-inner-container > .ytd-shorts:has(> .ytd-reel-video-renderer > ytd-ad-slot-renderer){display:none!important}"
            ],
            "{:?}",
            bundle.procedural
        );
    }

    // The extensions test relies on it: a page on 127.0.0.1 makes the shields replace their
    // scripts.
    #[test]
    fn an_ip_address_host_gets_its_own_rules() {
        let filters = Filters::build(&[list(FilterListId::EasyList, "127.0.0.1##.local-ad\n")]);

        let bundle = filters.lookup("127.0.0.1", &[]);

        assert_eq!(bundle.css, [".local-ad{display:none!important}"]);
    }

    #[test]
    fn an_unshielded_site_gets_nothing() {
        let filters = Filters::build(&[list(FilterListId::EasyList, LIST)]);

        let bundle = filters.lookup("video.test", &[Site::try_from("video.test").unwrap()]);

        assert!(bundle.off);
        assert_eq!(bundle.scriptlets, "");
    }

    #[test]
    fn classes_and_ids_find_their_generic_rules() {
        let filters = Filters::build(&[list(
            FilterListId::EasyList,
            "##.banner-ad\n###top-ad\n##.unused\n",
        )]);

        let mut hidden = filters.hidden(
            &["banner-ad".to_owned(), "article".to_owned()],
            &["top-ad".to_owned()],
            &HashSet::new(),
        );
        hidden.sort();

        assert_eq!(hidden, ["#top-ad", ".banner-ad"]);
    }

    #[test]
    fn lists_are_named_by_their_content() {
        let a = [list(FilterListId::EasyList, "##.a")];
        let b = [list(FilterListId::EasyList, "##.b")];

        assert_eq!(lists_name(&[]), None);
        assert_eq!(lists_name(&a), lists_name(&a));
        assert_ne!(lists_name(&a), lists_name(&b));
    }

    #[test]
    fn a_report_is_read_within_bounds() {
        let long = "x".repeat(LONGEST_NAME + 1);
        let text = format!(r#"{{"host":"a.test","classes":["ad","{long}",3],"ids":["top"]}}"#);

        let report = Report::parse(&text).unwrap();

        assert_eq!(report.host, "a.test");
        assert_eq!(report.classes, ["ad"]);
        assert_eq!(report.ids, ["top"]);
        assert!(Report::parse("[]").is_none());
        assert!(Report::parse("not json").is_none());
    }
}

//! Web extensions on the system `WebKit` (`WKWebExtension`, macOS 15.4 and later): the
//! application's [`ExtensionHost`] for the main profile.
//!
//! One `WKWebExtensionController` with the default, persistent configuration serves the main
//! profile: every page of the main jar gets it in its configuration, pages of throwaway jars
//! do not, and a context never has access to private data; the agent's profile is an engine
//! of its own and never gets one. Throwaway pages share the main jar's user content controller
//! (the content blocker installs its rules there), and `WebKit` injects content scripts
//! through that controller, so content scripts still reach them; giving throwaway jars a
//! controller of their own needs the content blocker to install into both.
//!
//! Each running extension is a context whose unique identifier and base URL
//! (`webkit-extension://<id>/`) are its id, so its storage survives restarts and updates.
//! What it may do is set whole from the user's grants each time it starts; nothing is granted
//! implicitly.
//!
//! What extensions ask of the browser arrives through the controller's delegate
//! ([`delegate`]) and leaves as [`ExtensionEvent`]s, drained by the shell after the waker; the
//! completion handlers wait here until [`ExtensionHost::answer`] or [`ExtensionHost::opened`].
//! Tabs and windows ([`tabs`]) are what the shell last arranged.
//!
//! - [`manifest`]: a package's manifest as the application's values.

mod delegate;
mod manifest;
mod tabs;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::{RcBlock, StackBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSDate, NSDictionary, NSError, NSRunLoop, NSSet, NSString, NSURL};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionAction, WKWebExtensionContext, WKWebExtensionController,
    WKWebExtensionControllerConfiguration, WKWebExtensionMatchPattern, WKWebExtensionTab,
    WKWebView,
};
use quickie_application::{
    Access, EngineError, ExtensionEvent, ExtensionHost, ReplyToken, WindowTabs,
};
use quickie_domain_browsing::TabId;
use quickie_domain_settings::{ExtensionId, Grants, SiteAccess, site_patterns};

use delegate::ControllerDelegate;
use tabs::{ExtensionTab, ExtensionWindow};

/// The scheme of extension pages.
const SCHEME: &str = "webkit-extension";

/// What the main profile's pages share with the extension host: the controller to put in
/// their configuration, and each tab's view, for extensions to reach.
pub(crate) struct Shared {
    pub(crate) controller: Retained<WKWebExtensionController>,
    pub(crate) views: RefCell<HashMap<TabId, Retained<WKWebView>>>,
}

impl Shared {
    /// The controller, or `None` on a macOS without `WKWebExtension` (before 15.4).
    pub(crate) fn new(user_agent: &str, mtm: MainThreadMarker) -> Option<Rc<Self>> {
        AnyClass::get(c"WKWebExtensionController")?;
        // SAFETY: the class exists; the default configuration is persistent.
        let configuration =
            unsafe { WKWebExtensionControllerConfiguration::defaultConfiguration(mtm) };
        // SAFETY: a getter; the configuration always has one.
        let pages = unsafe { configuration.webViewConfiguration() };
        // SAFETY: a plain string setter: extension pages carry Safari's tail, as pages do.
        unsafe { pages.setApplicationNameForUserAgent(Some(&NSString::from_str(user_agent))) };
        // SAFETY: the configuration copies it.
        unsafe { configuration.setWebViewConfiguration(Some(&pages)) };
        // SAFETY: a valid configuration made on the main thread.
        let controller = unsafe {
            WKWebExtensionController::initWithConfiguration(
                WKWebExtensionController::alloc(mtm),
                &configuration,
            )
        };
        Some(Rc::new(Self {
            controller,
            views: RefCell::new(HashMap::new()),
        }))
    }
}

/// A permission prompt's completion handler and what it asked, until answered.
pub(crate) enum Prompt {
    Permissions(
        RcBlock<dyn Fn(NonNull<NSSet<NSString>>, *mut NSDate)>,
        Vec<Retained<NSString>>,
    ),
    Urls(
        RcBlock<dyn Fn(NonNull<NSSet<NSURL>>, *mut NSDate)>,
        Vec<(Retained<NSURL>, String)>,
    ),
    Patterns(
        RcBlock<dyn Fn(NonNull<NSSet<WKWebExtensionMatchPattern>>, *mut NSDate)>,
        Vec<Retained<WKWebExtensionMatchPattern>>,
    ),
}

impl Prompt {
    /// Hands `WebKit` what was granted of what was asked.
    pub(crate) fn answer(self, granted: &Grants) {
        let never = std::ptr::null_mut();
        match self {
            Self::Permissions(done, asked) => {
                let given: Vec<_> = asked
                    .into_iter()
                    .filter(|name| granted.permissions.contains(&name.to_string()))
                    .collect();
                let set = NSSet::from_retained_slice(&given);
                done.call((NonNull::from(&*set), never));
            }
            Self::Urls(done, asked) => {
                let given: Vec<_> = asked
                    .into_iter()
                    .filter(|(_, pattern)| granted.patterns.contains(pattern))
                    .map(|(url, _)| url)
                    .collect();
                let set = NSSet::from_retained_slice(&given);
                done.call((NonNull::from(&*set), never));
            }
            Self::Patterns(done, asked) => {
                let given: Vec<_> = asked
                    .into_iter()
                    // SAFETY: a getter with no preconditions.
                    .filter(|pattern| granted.patterns.contains(&unsafe { pattern.string() }.to_string()))
                    .collect();
                let set = NSSet::from_retained_slice(&given);
                done.call((NonNull::from(&*set), never));
            }
        }
    }
}

/// A tab an extension opens, until the shell says which tab it became.
pub(crate) enum Opening {
    Tab(RcBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>),
    /// The options page, which answers with no tab.
    Page(RcBlock<dyn Fn(*mut NSError)>),
}

/// Everything the host, its delegate, and its tabs and windows share.
pub(crate) struct State {
    mtm: MainThreadMarker,
    pub(crate) shared: Rc<Shared>,
    delegate: RefCell<Option<Retained<ControllerDelegate>>>,
    /// Packages read and not started yet.
    read: RefCell<HashMap<ExtensionId, Retained<WKWebExtension>>>,
    running: RefCell<HashMap<ExtensionId, Retained<WKWebExtensionContext>>>,
    events: RefCell<Vec<ExtensionEvent>>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
    scheduled: Cell<bool>,
    next_token: Cell<u64>,
    pub(crate) prompts: RefCell<HashMap<ReplyToken, Prompt>>,
    pub(crate) opens: RefCell<HashMap<ReplyToken, Opening>>,
    pub(crate) tabs: RefCell<HashMap<TabId, Retained<ExtensionTab>>>,
    /// The focused window first.
    pub(crate) windows: RefCell<Vec<Retained<ExtensionWindow>>>,
    /// The action whose popup waits to be shown, and whose extension it is.
    pub(crate) popup: RefCell<Option<(ExtensionId, Retained<WKWebExtensionAction>)>>,
}

impl State {
    pub(crate) fn token(&self) -> ReplyToken {
        let next = self.next_token.get() + 1;
        self.next_token.set(next);
        ReplyToken::new(next)
    }

    /// Queues `event` and wakes the shell for it, later on the main run loop.
    pub(crate) fn report(state: &Rc<Self>, event: ExtensionEvent) {
        state.events.borrow_mut().push(event);
        Self::schedule(state);
    }

    /// The tab object `tab` is, if it is one of ours.
    pub(crate) fn tab_of(&self, tab: &ProtocolObject<dyn WKWebExtensionTab>) -> Option<TabId> {
        self.tabs
            .borrow()
            .iter()
            .find(|(_, ours)| std::ptr::addr_eq(&raw const **ours, std::ptr::from_ref(tab)))
            .map(|(&id, _)| id)
    }

    fn schedule(state: &Rc<Self>) {
        if state.scheduled.get() || state.events.borrow().is_empty() {
            return;
        }
        let Some(wake) = state.wake.borrow().clone() else {
            return;
        };
        state.scheduled.set(true);
        let weak = Rc::downgrade(state);
        let block = RcBlock::new(move || {
            if let Some(state) = weak.upgrade() {
                state.scheduled.set(false);
            }
            wake();
        });
        // SAFETY: the block captures an `Rc`, which is sound because it runs on the main run
        // loop, the thread that scheduled it and owns it.
        unsafe { NSRunLoop::mainRunLoop().performBlock(&block) };
    }
}

impl Drop for State {
    /// Requests nobody answered get nothing, so no extension waits forever.
    fn drop(&mut self) {
        for (_, prompt) in self.prompts.get_mut().drain() {
            prompt.answer(&Grants::new());
        }
        for (_, opening) in self.opens.get_mut().drain() {
            match opening {
                Opening::Tab(done) => done.call((std::ptr::null_mut(), std::ptr::null_mut())),
                Opening::Page(done) => done.call((std::ptr::null_mut(),)),
            }
        }
    }
}

/// The main profile's web extensions; see the module documentation. Lives on the main thread.
pub struct WebKitExtensions {
    state: Rc<State>,
}

impl std::fmt::Debug for WebKitExtensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebKitExtensions")
            .field("running", &self.state.running.borrow().len())
            .finish_non_exhaustive()
    }
}

impl WebKitExtensions {
    pub(crate) fn new(shared: Rc<Shared>, mtm: MainThreadMarker) -> Self {
        let state = Rc::new(State {
            mtm,
            shared,
            delegate: RefCell::new(None),
            read: RefCell::new(HashMap::new()),
            running: RefCell::new(HashMap::new()),
            events: RefCell::new(Vec::new()),
            wake: RefCell::new(None),
            scheduled: Cell::new(false),
            next_token: Cell::new(0),
            prompts: RefCell::new(HashMap::new()),
            opens: RefCell::new(HashMap::new()),
            tabs: RefCell::new(HashMap::new()),
            windows: RefCell::new(Vec::new()),
            popup: RefCell::new(None),
        });
        let delegate = ControllerDelegate::new(Rc::downgrade(&state), mtm);
        // SAFETY: the delegate is weak; the state keeps it for as long as the controller.
        unsafe {
            state
                .shared
                .controller
                .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        *state.delegate.borrow_mut() = Some(delegate);
        Self { state }
    }

    /// Sets what runs, later on the main run loop, when events wait: it should drain them.
    pub fn set_waker(&mut self, wake: impl Fn() + 'static) {
        *self.state.wake.borrow_mut() = Some(Rc::new(wake));
        State::schedule(&self.state);
    }

    /// The popup [`ExtensionEvent::Popup`] announced: its extension and its page's view, sized
    /// as the extension asked; `None` once it is gone.
    #[must_use]
    pub fn popup(&self) -> Option<(ExtensionId, Retained<WKWebView>)> {
        let popup = self.state.popup.borrow();
        let (id, action) = popup.as_ref()?;
        // SAFETY: a getter; the action makes the view when it presents a popup.
        let view = unsafe { action.popupWebView() }?;
        Some((id.clone(), view))
    }

    /// Closes the popup on screen, if any, as clicking outside it would.
    pub fn close_popup(&self) {
        let popup = self.state.popup.borrow_mut().take();
        if let Some((_, action)) = popup {
            // SAFETY: closing an action's popup has no preconditions.
            unsafe { action.closePopup() };
        }
    }

    /// Whether the extension `id` runs now.
    #[must_use]
    pub fn running(&self, id: &ExtensionId) -> bool {
        self.state.running.borrow().contains_key(id)
    }

    fn context(&self, id: &ExtensionId) -> Result<Retained<WKWebExtensionContext>, EngineError> {
        self.state
            .running
            .borrow()
            .get(id)
            .cloned()
            .ok_or_else(|| EngineError(format!("the extension {id} is not running")))
    }

    /// Makes a context for `extension` as `id`, with `access`, and loads it.
    fn load(
        &self,
        id: &ExtensionId,
        extension: &WKWebExtension,
        access: &Access,
    ) -> Result<(), EngineError> {
        // SAFETY: a fresh context for a read extension.
        let context = unsafe { WKWebExtensionContext::contextForExtension(extension) };
        // SAFETY: plain setters on a context not loaded yet.
        unsafe { context.setUniqueIdentifier(&NSString::from_str(id.as_str())) };
        let base = NSURL::URLWithString(&NSString::from_str(&format!("{SCHEME}://{id}/")));
        if let Some(base) = base {
            // SAFETY: as above; the scheme is the one `WebKit` serves extension pages on.
            unsafe { context.setBaseURL(&base) };
        }
        // SAFETY: as above.
        unsafe { context.setInspectable(true) };
        // SAFETY: as above: no access to throwaway jars' pages and data.
        unsafe { context.setHasAccessToPrivateData(false) };
        apply(&context, access, self.state.mtm);
        // SAFETY: a configured context, loaded once into the main profile's controller.
        unsafe {
            self.state
                .shared
                .controller
                .loadExtensionContext_error(&context)
        }
        .map_err(|error| EngineError(error.localizedDescription().to_string()))?;
        self.state.running.borrow_mut().insert(id.clone(), context);
        Ok(())
    }

    fn unload(&self, id: &ExtensionId) {
        let context = self.state.running.borrow_mut().remove(id);
        if let Some(context) = context {
            let mut popup = self.state.popup.borrow_mut();
            if popup.as_ref().is_some_and(|(shown, _)| shown == id) {
                *popup = None;
            }
            drop(popup);
            // SAFETY: a context this controller loaded; an error means it is already gone.
            let _ = unsafe {
                self.state
                    .shared
                    .controller
                    .unloadExtensionContext_error(&context)
            };
        }
    }
}

/// Sets what a context may do, whole: the grants, each allowed site as granted patterns, each
/// blocked site as denied ones. A pattern `WebKit` cannot read is left out.
fn apply(context: &WKWebExtensionContext, access: &Access, mtm: MainThreadMarker) {
    let forever = NSDate::distantFuture();
    let permissions: Vec<Retained<NSString>> = access
        .granted
        .permissions
        .iter()
        .map(|name| NSString::from_str(name))
        .collect();
    let mut granted: BTreeSet<String> = access.granted.patterns.clone();
    let mut denied = BTreeSet::new();
    for (site, choice) in &access.sites {
        let into = match choice {
            SiteAccess::Allowed => &mut granted,
            SiteAccess::Blocked => &mut denied,
        };
        into.extend(site_patterns(site));
    }
    let dates = |count: usize| vec![forever.clone(); count];
    let keys: Vec<&NSString> = permissions.iter().map(|name| &**name).collect();
    let values = dates(keys.len());
    let values: Vec<&NSDate> = values.iter().map(|date| &**date).collect();
    let granted_permissions = NSDictionary::from_slices(&keys, &values);
    // SAFETY: a dictionary of permission names to expiry dates, as the setter takes.
    unsafe { context.setGrantedPermissions(&granted_permissions) };
    for (patterns, grant) in [(granted, true), (denied, false)] {
        let made: Vec<Retained<WKWebExtensionMatchPattern>> = patterns
            .iter()
            .filter_map(|text| manifest::pattern(text, mtm))
            .collect();
        let keys: Vec<&WKWebExtensionMatchPattern> =
            made.iter().map(|pattern| &**pattern).collect();
        let values = dates(keys.len());
        let values: Vec<&NSDate> = values.iter().map(|date| &**date).collect();
        let dictionary = NSDictionary::from_slices(&keys, &values);
        if grant {
            // SAFETY: a dictionary of patterns to expiry dates, as the setter takes.
            unsafe { context.setGrantedPermissionMatchPatterns(&dictionary) };
        } else {
            // SAFETY: as above.
            unsafe { context.setDeniedPermissionMatchPatterns(&dictionary) };
        }
    }
}

impl ExtensionHost for WebKitExtensions {
    fn read(&mut self, id: &ExtensionId, location: &str) -> Result<(), EngineError> {
        let path = Path::new(location);
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(location), path.is_dir());
        let state = Rc::downgrade(&self.state);
        let id = id.clone();
        let block = StackBlock::new(move |extension: *mut WKWebExtension, error: *mut NSError| {
            let Some(state) = state.upgrade() else {
                return;
            };
            // SAFETY: `WebKit` hands a valid extension or nil; retaining keeps it.
            let extension = unsafe { Retained::retain(extension) };
            let info = match extension {
                Some(extension) => {
                    // SAFETY: a fresh, unloaded context, only read for its commands.
                    let context = unsafe { WKWebExtensionContext::contextForExtension(&extension) };
                    let info = manifest::info(&extension, &context);
                    if info.is_ok() {
                        state.read.borrow_mut().insert(id.clone(), extension);
                    }
                    info
                }
                // SAFETY: `WebKit` hands a valid error or nil with a nil extension.
                None => Err(unsafe { error.as_ref() }.map_or_else(
                    || "unreadable".to_owned(),
                    |error| error.localizedDescription().to_string(),
                )),
            };
            State::report(
                &state,
                ExtensionEvent::Read {
                    id: id.clone(),
                    info,
                },
            );
        });
        // SAFETY: a file URL of a directory or an archive; the completion runs on the main
        // thread.
        unsafe {
            WKWebExtension::extensionWithResourceBaseURL_completionHandler(
                &url,
                &block,
                self.state.mtm,
            );
        }
        Ok(())
    }

    fn start(&mut self, id: &ExtensionId, access: &Access) -> Result<(), EngineError> {
        let staged = self.state.read.borrow_mut().remove(id);
        if let Some(extension) = staged {
            self.unload(id);
            return self.load(id, &extension, access);
        }
        let context = self.state.running.borrow().get(id).cloned();
        if let Some(context) = context {
            apply(&context, access, self.state.mtm);
            return Ok(());
        }
        // Stopped earlier: its package was read before it ran.
        Err(EngineError(format!(
            "nothing was read for the extension {id}"
        )))
    }

    fn stop(&mut self, id: &ExtensionId) {
        let context = self.state.running.borrow().get(id).cloned();
        if let Some(context) = context {
            // SAFETY: a getter with no preconditions.
            let extension = unsafe { context.webExtension() };
            self.unload(id);
            // Kept as read, so it starts again without reading.
            self.state
                .read
                .borrow_mut()
                .entry(id.clone())
                .or_insert(extension);
        }
    }

    fn discard(&mut self, id: &ExtensionId) {
        self.state.read.borrow_mut().remove(id);
    }

    fn remove(&mut self, id: &ExtensionId) {
        self.state.read.borrow_mut().remove(id);
        self.unload(id);
    }

    fn perform_action(&mut self, id: &ExtensionId, tab: Option<TabId>) -> Result<(), EngineError> {
        let context = self.context(id)?;
        let tab = tab.and_then(|tab| self.state.tabs.borrow().get(&tab).cloned());
        let tab = tab.as_deref().map(ProtocolObject::from_ref);
        // SAFETY: a loaded context; `WebKit` presents the popup through the delegate.
        unsafe { context.performActionForTab(tab) };
        Ok(())
    }

    fn perform_command(&mut self, id: &ExtensionId, command: &str) -> Result<(), EngineError> {
        let context = self.context(id)?;
        // SAFETY: a getter on a loaded context.
        let commands = unsafe { context.commands() };
        let command = commands
            .iter()
            // SAFETY: a getter with no preconditions.
            .find(|candidate| unsafe { candidate.identifier() }.to_string() == command)
            .ok_or_else(|| EngineError(format!("{id} has no command {command}")))?;
        // SAFETY: one of the context's own commands.
        unsafe { context.performCommand(&command) };
        Ok(())
    }

    fn answer(&mut self, token: ReplyToken, granted: &Grants) {
        let prompt = self.state.prompts.borrow_mut().remove(&token);
        if let Some(prompt) = prompt {
            prompt.answer(granted);
        }
    }

    fn opened(&mut self, token: ReplyToken, tab: Option<TabId>) {
        let opening = self.state.opens.borrow_mut().remove(&token);
        match opening {
            Some(Opening::Tab(done)) => {
                let ours = tab.and_then(|tab| self.state.tabs.borrow().get(&tab).cloned());
                let pointer = ours.as_ref().map_or(std::ptr::null_mut(), |tab| {
                    let tab: &ProtocolObject<dyn WKWebExtensionTab> =
                        ProtocolObject::from_ref(&**tab);
                    std::ptr::from_ref(tab).cast_mut()
                });
                done.call((pointer, std::ptr::null_mut()));
            }
            Some(Opening::Page(done)) => done.call((std::ptr::null_mut(),)),
            None => {}
        }
    }

    fn arrange(&mut self, windows: &[WindowTabs]) {
        arrange(&self.state, windows);
    }

    fn drain(&mut self, into: &mut Vec<ExtensionEvent>) {
        into.append(&mut self.state.events.borrow_mut());
    }
}

/// Brings the tab and window objects in line with `windows` and tells the controller what
/// changed: tabs and windows opened and closed, the focused window, each window's active tab.
fn arrange(state: &Rc<State>, windows: &[WindowTabs]) {
    let controller = state.shared.controller.clone();
    let mtm = state.mtm;
    let live: BTreeSet<TabId> = windows
        .iter()
        .flat_map(|window| window.tabs.iter().copied())
        .collect();
    let closed_tabs: Vec<Retained<ExtensionTab>> = {
        let mut tabs = state.tabs.borrow_mut();
        let closed: Vec<TabId> = tabs
            .keys()
            .filter(|tab| !live.contains(tab))
            .copied()
            .collect();
        closed.iter().filter_map(|tab| tabs.remove(tab)).collect()
    };
    let opened_tabs: Vec<Retained<ExtensionTab>> = live
        .iter()
        .filter(|tab| !state.tabs.borrow().contains_key(tab))
        .map(|&tab| {
            let made = ExtensionTab::new(tab, Rc::downgrade(state), mtm);
            state.tabs.borrow_mut().insert(tab, made.clone());
            made
        })
        .collect();
    let previous: Vec<Retained<ExtensionWindow>> = state.windows.borrow().clone();
    let mut arranged = Vec::with_capacity(windows.len());
    let mut opened_windows = Vec::new();
    let mut activated = Vec::new();
    for wanted in windows {
        let window = previous
            .iter()
            .find(|window| window.ivars().window == wanted.window)
            .cloned()
            .unwrap_or_else(|| {
                let made = ExtensionWindow::new(wanted.window, Rc::downgrade(state), mtm);
                opened_windows.push(made.clone());
                made
            });
        window.ivars().tabs.replace(wanted.tabs.clone());
        let before = window.ivars().active.replace(wanted.active);
        if before != wanted.active
            && let Some(active) = wanted.active
        {
            activated.push((active, before));
        }
        arranged.push(window);
    }
    let closed_windows: Vec<Retained<ExtensionWindow>> = previous
        .into_iter()
        .filter(|window| {
            !windows
                .iter()
                .any(|wanted| wanted.window == window.ivars().window)
        })
        .collect();
    let focus_changed = state.windows.borrow().first().map(|w| w.ivars().window)
        != arranged.first().map(|w| w.ivars().window);
    *state.windows.borrow_mut() = arranged;
    // Told after the objects are in place, as the controller may ask about them at once.
    for window in &opened_windows {
        // SAFETY: a window object the delegate lists from now on.
        unsafe { controller.didOpenWindow(ProtocolObject::from_ref(&**window)) };
    }
    for tab in &opened_tabs {
        // SAFETY: a tab object its window lists from now on.
        unsafe { controller.didOpenTab(ProtocolObject::from_ref(&**tab)) };
    }
    for tab in &closed_tabs {
        // SAFETY: a tab object no window lists any more.
        unsafe { controller.didCloseTab_windowIsClosing(ProtocolObject::from_ref(&**tab), false) };
    }
    for window in &closed_windows {
        // SAFETY: a window object the delegate no longer lists.
        unsafe { controller.didCloseWindow(ProtocolObject::from_ref(&**window)) };
    }
    for (active, before) in activated {
        let tabs = state.tabs.borrow();
        let (Some(active), before) = (
            tabs.get(&active).cloned(),
            before.and_then(|tab| tabs.get(&tab).cloned()),
        ) else {
            continue;
        };
        drop(tabs);
        let before = before.as_deref().map(ProtocolObject::from_ref);
        // SAFETY: tab objects the windows list.
        unsafe {
            controller.didActivateTab_previousActiveTab(ProtocolObject::from_ref(&*active), before);
        }
    }
    if focus_changed {
        let first = state.windows.borrow().first().cloned();
        let first = first.as_deref().map(ProtocolObject::from_ref);
        // SAFETY: a window object the delegate lists, or none.
        unsafe { controller.didFocusWindow(first) };
    }
}

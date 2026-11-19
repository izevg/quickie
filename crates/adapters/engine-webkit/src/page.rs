//! One tab's page: a `WKWebView` in a data store, the observer that reports its navigations
//! and loading, the delegate that hears what it asks (pages of its own, dialogs, devices), find
//! in page, and the scripts Quickie runs in it (focus reports and link hints).

pub(crate) mod asks;
mod find;
mod observer;
pub(crate) mod scripts;

use std::cell::Cell;
use std::rc::Rc;

pub(crate) use scripts::FocusReports;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSImage, NSView};
use objc2_foundation::{NSError, NSNumber, NSRect, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKContentWorld, WKSnapshotConfiguration, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use quickie_application::EngineError;
use quickie_domain_browsing::{FindMatches, Focus};
use url::Url;

pub use find::FindStep;
pub(crate) use observer::PageEvent;
use observer::PageObserver;
use scripts::hints::{self, COLLECT_SCRIPT, Followed, LinkTarget, follow_script};
use scripts::{FOCUS_HANDLER, FOCUS_SCRIPT, FocusHandler};

use crate::scheme::InternalPages;
use asks::delegate::PageUi;
use asks::{AskKind, Reply};

/// Where a page's reports and requests go. Called on the main thread, sometimes from inside a
/// call into the view: a hook records, it never calls back into the engine.
pub(crate) struct Hooks {
    /// Navigations, titles, loading, links to open elsewhere, closing, full screen.
    pub(crate) on_event: Box<dyn Fn(PageEvent)>,
    /// A dialog or a permission, from the origin in the URL, waiting for its reply.
    pub(crate) on_ask: Box<dyn Fn(Url, AskKind, Reply)>,
    /// A page the page opens (`window.open`, `target=_blank`) in `configuration`, which
    /// `WebKit` made from this page's own: the new view must use it, so it shares the opener's
    /// process and data store. `None` refuses the page.
    #[expect(
        clippy::type_complexity,
        reason = "the one call WebKit makes to get a view back, written out where it is held"
    )]
    pub(crate) open:
        Box<dyn Fn(&WKWebViewConfiguration, Url, Focus) -> Option<Retained<WKWebView>>>,
}

/// A page and its view. Dropping it stops observation and takes the view off screen.
pub(crate) struct WebPage {
    view: Retained<WKWebView>,
    observer: Retained<PageObserver>,
    /// Keeps the UI delegate alive as long as the view refers to it.
    _ui: Retained<PageUi>,
    world: Retained<WKContentWorld>,
    /// Every page's last report of whether its focused element takes text, this one's under
    /// its view.
    focus: FocusReports,
}

impl WebPage {
    /// A new, empty page in `store` that serves `internal` pages and blocks what `content`'s
    /// rule lists block. `hooks` hear what it reports and asks; its focus reports land in
    /// `focus`.
    pub(crate) fn new(
        store: &WKWebsiteDataStore,
        content: &WKUserContentController,
        internal: &InternalPages,
        hooks: Hooks,
        focus: FocusReports,
        mtm: MainThreadMarker,
    ) -> Self {
        // SAFETY: a fresh configuration has no preconditions.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        // SAFETY: `store` is a valid data store; the configuration is copied on use.
        unsafe { configuration.setWebsiteDataStore(store) };
        // SAFETY: a valid controller; the copy keeps the same controller, so rule lists
        // installed later reach this page too.
        unsafe { configuration.setUserContentController(content) };
        for scheme in internal.schemes() {
            // SAFETY: the built-in schemes are custom (WebKit throws for its own, such as
            // `https`), and the configuration retains the handler.
            unsafe {
                configuration.setURLSchemeHandler_forURLScheme(
                    Some(ProtocolObject::from_ref(internal)),
                    &NSString::from_str(&scheme),
                );
            }
        }
        // SAFETY: preferences always exist on a configuration.
        let preferences = unsafe { configuration.preferences() };
        // Safe Browsing lookups would contact a third party on every navigation: trackerless.
        // SAFETY: a plain boolean setter.
        unsafe { preferences.setFraudulentWebsiteWarningEnabled(false) };
        // The Fullscreen API; the shell shows whose page went full screen.
        // SAFETY: a plain boolean setter.
        unsafe { preferences.setElementFullscreenEnabled(true) };
        Self::with_configuration(&configuration, hooks, focus, mtm)
    }

    /// A page from `configuration` as it is: a copy of an opener's, so the page shares the
    /// opener's data store, content rules, and internal pages.
    ///
    /// Two-finger swipes go back and forward with `WebKit`'s own interactive preview
    /// (`allowsBackForwardNavigationGestures`): it tracks the fingers on a cached snapshot of
    /// the neighbouring page in its own layer, as the architecture's swipe budget asks. Pinch
    /// does not zoom the page; it is the shell's gesture for the tab overview.
    pub(crate) fn with_configuration(
        configuration: &WKWebViewConfiguration,
        hooks: Hooks,
        focus: FocusReports,
        mtm: MainThreadMarker,
    ) -> Self {
        let world = scripts::world(mtm);
        // SAFETY: the frame is any rect and the configuration is fully set up.
        let view = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::ZERO,
                configuration,
            )
        };
        // SAFETY: a plain boolean setter.
        unsafe { view.setAllowsBackForwardNavigationGestures(true) };
        // SAFETY: as above.
        unsafe { view.setAllowsMagnification(false) };
        let hooks = Rc::new(hooks);
        let observer = PageObserver::new(Rc::clone(&hooks), mtm);
        observer.attach(&view);
        let ui = PageUi::new(hooks, mtm);
        // SAFETY: the delegate is weak; the page keeps it alive and clears it on drop.
        unsafe { view.setUIDelegate(Some(ProtocolObject::from_ref(&*ui))) };
        Self {
            view,
            observer,
            _ui: ui,
            world,
            focus,
        }
    }

    /// The view to embed; it draws the page edge to edge.
    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }

    /// A copy of the page's configuration, to open a page in the same data store.
    pub(crate) fn configuration(&self) -> Retained<WKWebViewConfiguration> {
        // SAFETY: a getter that returns a copy.
        unsafe { self.view.configuration() }
    }

    /// The URL the page shows now and its title, `None` before it has one.
    pub(crate) fn committed(&self) -> Option<(String, Option<String>)> {
        // SAFETY: getters with no preconditions.
        let url = unsafe { self.view.URL() }?.absoluteString()?.to_string();
        // SAFETY: as above.
        let title = unsafe { self.view.title() }
            .map(|title| title.to_string())
            .filter(|title| !title.is_empty());
        Some((url, title))
    }

    /// True when the page last said its focused element takes text.
    pub(crate) fn input_focused(&self) -> bool {
        self.focus
            .borrow()
            .get(&scripts::view_key(&self.view))
            .copied()
            .unwrap_or(false)
    }

    /// The web view itself, to start a download in its jar or to hand to `WebKit` for a page it
    /// asked for.
    pub(crate) fn web_view(&self) -> &WKWebView {
        &self.view
    }

    /// Navigates to `url`.
    pub(crate) fn load(&self, url: &Url) -> Result<(), EngineError> {
        let parsed = NSURL::URLWithString(&NSString::from_str(url.as_str()))
            .ok_or_else(|| EngineError(format!("not a URL WebKit can load: {url}")))?;
        let request = NSURLRequest::requestWithURL(&parsed);
        // SAFETY: `request` is a valid request for an absolute URL.
        let _navigation = unsafe { self.view.loadRequest(&request) };
        Ok(())
    }

    pub(crate) fn go_back(&self) {
        // SAFETY: a no-op when there is nothing to go back to.
        let _navigation = unsafe { self.view.goBack() };
    }

    pub(crate) fn go_forward(&self) {
        // SAFETY: a no-op when there is nothing to go forward to.
        let _navigation = unsafe { self.view.goForward() };
    }

    pub(crate) fn reload(&self) {
        // SAFETY: reloading has no preconditions.
        let _navigation = unsafe { self.view.reload() };
    }

    pub(crate) fn stop(&self) {
        // SAFETY: stopping has no preconditions.
        unsafe { self.view.stopLoading() };
    }

    /// How much of the page has loaded, `0.0 ..= 1.0`, while it loads; `None` once it is done.
    pub(crate) fn progress(&self) -> Option<f64> {
        // SAFETY: getters with no preconditions.
        let loading = unsafe { self.view.isLoading() };
        // SAFETY: as above.
        loading.then(|| unsafe { self.view.estimatedProgress() }.clamp(0.0, 1.0))
    }

    /// Selects a match of `query` as `step` says and marks all of them; `done` hears how many
    /// there are. An empty query clears the marks.
    pub(crate) fn find(&self, query: &str, step: FindStep, done: impl Fn(FindMatches) + 'static) {
        find::find(&self.view, query, step, done);
    }

    /// Renders the visible page `width` points wide, off the main thread, and hands the image
    /// to `done` on the main thread; `None` when the page cannot be drawn.
    pub(crate) fn snapshot(
        &self,
        width: f64,
        done: impl FnOnce(Option<Retained<NSImage>>) + 'static,
    ) {
        let mtm = self.view.mtm();
        // SAFETY: a fresh configuration has no preconditions.
        let configuration = unsafe { WKSnapshotConfiguration::new(mtm) };
        // SAFETY: a plain setter; the configuration copies the number.
        unsafe { configuration.setSnapshotWidth(Some(&NSNumber::new_f64(width))) };
        // SAFETY: a plain setter. Not waiting for a screen update lets a page that is not on
        // screen (just hidden by a tab switch) be drawn from its last layout.
        unsafe { configuration.setAfterScreenUpdates(false) };
        let done = Cell::new(Some(done));
        let block = RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            // SAFETY: WebKit passes a valid image or null; retaining keeps it past the call.
            let image = unsafe { Retained::retain(image) };
            if let Some(done) = done.take() {
                done(image);
            }
        });
        // SAFETY: WebKit calls the block once, on the main thread, where its captures live.
        unsafe {
            self.view
                .takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &block);
        }
    }

    /// Finds the links and controls on screen, for link hints, and hands them to `done` in
    /// document order, placed in the page view.
    pub(crate) fn link_targets(&self, done: impl FnOnce(Vec<LinkTarget>) + 'static) {
        let size = self.view.bounds().size;
        self.evaluate(COLLECT_SCRIPT, move |answer| {
            done(hints::targets(&answer, size));
        });
    }

    /// Follows target `index` of the last [`Self::link_targets`]; `background` asks a link to
    /// open in a background tab. `done` hears what is left for the browser to do.
    pub(crate) fn follow(
        &self,
        index: usize,
        background: bool,
        done: impl FnOnce(Followed) + 'static,
    ) {
        self.evaluate(&follow_script(index, background), move |answer| {
            done(hints::followed(&answer));
        });
    }

    /// Runs `script` in Quickie's world of the main frame; `done` gets its string answer, empty
    /// for anything else or an error.
    fn evaluate(&self, script: &str, done: impl FnOnce(String) + 'static) {
        let done = Cell::new(Some(done));
        let block = RcBlock::new(move |answer: *mut AnyObject, _error: *mut NSError| {
            // SAFETY: WebKit passes a valid object or null for the duration of the call.
            let answer = unsafe { answer.as_ref() }
                .and_then(|answer| answer.downcast_ref::<NSString>())
                .map_or_else(String::new, NSString::to_string);
            if let Some(done) = done.take() {
                done(answer);
            }
        });
        // SAFETY: no frame means the main frame; WebKit calls the block once on the main
        // thread, where its captures live.
        unsafe {
            self.view
                .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(script),
                    None,
                    &self.world,
                    Some(&block),
                );
        }
    }
}

impl Drop for WebPage {
    fn drop(&mut self) {
        self.observer.detach(&self.view);
        // SAFETY: clearing a weak delegate has no preconditions.
        unsafe { self.view.setUIDelegate(None) };
        self.focus
            .borrow_mut()
            .remove(&scripts::view_key(&self.view));
        // SAFETY: stopping has no preconditions; a page going away loads nothing more.
        unsafe { self.view.stopLoading() };
        self.view.removeFromSuperview();
    }
}

/// Adds the focus script to every frame of every page that uses `controller`, and one handler
/// that records each page's reports in `reports`. Once per controller: `WebKit` refuses a
/// second handler under the same name, and every page, popups included, shares it.
pub(crate) fn install_focus_script(
    controller: &WKUserContentController,
    reports: FocusReports,
    mtm: MainThreadMarker,
) {
    let world = scripts::world(mtm);
    let handler = FocusHandler::new(reports, mtm);
    // SAFETY: the source is a complete script, injected at document start into every frame.
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(FOCUS_SCRIPT),
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
            &world,
        )
    };
    // SAFETY: the controller retains the script.
    unsafe { controller.addUserScript(&script) };
    // SAFETY: the controller retains the handler; this is the only handler under the name in
    // this world, as the controller is new and this runs once for it.
    unsafe {
        controller.addScriptMessageHandler_contentWorld_name(
            ProtocolObject::from_ref(&*handler),
            &world,
            &NSString::from_str(FOCUS_HANDLER),
        );
    }
}

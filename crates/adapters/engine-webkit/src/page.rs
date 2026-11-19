//! One tab's page: a `WKWebView` in a data store, the observer that reports its navigations
//! and loading, find in page, and the scripts Quickie runs in it (focus reports and link
//! hints).

mod find;
mod observer;
pub(crate) mod scripts;

use std::cell::Cell;
use std::rc::Rc;

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
use quickie_domain_browsing::FindMatches;
use url::Url;

pub use find::FindStep;
pub(crate) use observer::PageEvent;
use observer::PageObserver;
use scripts::hints::{self, COLLECT_SCRIPT, Followed, LinkTarget, follow_script};
use scripts::{FOCUS_HANDLER, FOCUS_SCRIPT, FocusHandler};

use crate::scheme::InternalPages;

/// A page and its view. Dropping it stops observation and takes the view off screen.
pub(crate) struct WebPage {
    view: Retained<WKWebView>,
    observer: Retained<PageObserver>,
    world: Retained<WKContentWorld>,
    /// The page's last report of whether its focused element takes text.
    focused: Rc<Cell<bool>>,
    /// Keeps the focus handler alive as long as the page registers it.
    _focus: Retained<FocusHandler>,
}

impl WebPage {
    /// A new, empty page in `store` that serves `internal` pages and blocks what `content`'s
    /// rule lists block. `on_event` hears its committed navigations and title changes.
    ///
    /// Two-finger swipes go back and forward with `WebKit`'s own interactive preview
    /// (`allowsBackForwardNavigationGestures`): it tracks the fingers on a cached snapshot of
    /// the neighbouring page in its own layer, as the architecture's swipe budget asks. Pinch
    /// does not zoom the page; it is the shell's gesture for the tab overview.
    pub(crate) fn new(
        store: &WKWebsiteDataStore,
        content: &WKUserContentController,
        internal: &InternalPages,
        on_event: impl Fn(PageEvent) + 'static,
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
        let world = scripts::world(mtm);
        let focused = Rc::new(Cell::new(false));
        let focus = FocusHandler::new(Rc::clone(&focused), mtm);
        install_focus_script(&configuration, &world, &focus, mtm);
        // SAFETY: the frame is any rect and the configuration is fully set up.
        let view = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::ZERO,
                &configuration,
            )
        };
        // SAFETY: a plain boolean setter.
        unsafe { view.setAllowsBackForwardNavigationGestures(true) };
        // SAFETY: as above.
        unsafe { view.setAllowsMagnification(false) };
        let observer = PageObserver::new(Box::new(on_event), mtm);
        observer.attach(&view);
        Self {
            view,
            observer,
            world,
            focused,
            _focus: focus,
        }
    }

    /// The view to embed; it draws the page edge to edge.
    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }

    /// True when the page last said its focused element takes text.
    pub(crate) fn input_focused(&self) -> bool {
        self.focused.get()
    }

    /// The web view itself, to start a download in its jar.
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
        // SAFETY: a getter; the configuration lives as long as the view.
        let configuration = unsafe { self.view.configuration() };
        // SAFETY: a getter; a configuration always has a controller.
        let controller = unsafe { configuration.userContentController() };
        // SAFETY: the handler was added under this name in this world.
        unsafe {
            controller.removeScriptMessageHandlerForName_contentWorld(
                &NSString::from_str(FOCUS_HANDLER),
                &self.world,
            );
        }
        // SAFETY: stopping has no preconditions; a page going away loads nothing more.
        unsafe { self.view.stopLoading() };
        self.view.removeFromSuperview();
    }
}

/// Adds the focus script to every frame of pages made from `configuration`, and `handler` to
/// hear it, both in `world`.
fn install_focus_script(
    configuration: &WKWebViewConfiguration,
    world: &WKContentWorld,
    handler: &FocusHandler,
    mtm: MainThreadMarker,
) {
    // SAFETY: a getter; a configuration always has a controller.
    let controller = unsafe { configuration.userContentController() };
    // SAFETY: the source is a complete script, injected at document start into every frame.
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(FOCUS_SCRIPT),
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
            world,
        )
    };
    // SAFETY: the controller retains the script.
    unsafe { controller.addUserScript(&script) };
    // SAFETY: the controller retains the handler; the name is unique to it in this world.
    unsafe {
        controller.addScriptMessageHandler_contentWorld_name(
            ProtocolObject::from_ref(handler),
            world,
            &NSString::from_str(FOCUS_HANDLER),
        );
    }
}

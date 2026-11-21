//! One tab's page: a `WKWebView` in a data store, inside a frame view of its own that it shares
//! only with its docked Web Inspector, the observer that reports its navigations and loading,
//! the delegate that hears what it asks (pages of its own, dialogs, devices), find in page, Web
//! Inspector, and the scripts Quickie runs in it (focus reports, link hints, and full screen in
//! the tab's own window).

pub(crate) mod asks;
mod find;
pub(crate) mod inspector;
mod observer;
pub(crate) mod scripts;
pub mod trace;

use std::cell::Cell;
use std::rc::Rc;

pub(crate) use scripts::FocusReports;
use scripts::fullscreen::{self, FullscreenHandler, FullscreenPages};
use scripts::video::VideoFrames;

/// Where every page's scripts report, shared by all pages: whether a frame's focused element
/// takes text, the hooks that hear a page go full screen, and the frames that report on the
/// page's video, each under its view.
#[derive(Clone, Default)]
pub(crate) struct Reports {
    pub(crate) focus: FocusReports,
    pub(crate) fullscreen: FullscreenPages,
    pub(crate) videos: VideoFrames,
}

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSImage, NSPrintInfo, NSPrintingPaginationMode, NSView,
};
use objc2_foundation::{
    NSData, NSError, NSNumber, NSObjectProtocol, NSRect, NSSize, NSString, NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKContentWorld, WKFullscreenState, WKScriptMessageHandler, WKSnapshotConfiguration,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebExtensionController,
    WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use quickie_application::{AskKind, DevTools, EngineError, FindStep, Followed, LinkTarget};
use quickie_domain_browsing::{FindMatches, Focus};
use url::Url;

pub(crate) use observer::PageEvent;
use observer::PageObserver;
use scripts::hints::{self, COLLECT_SCRIPT, follow_script};
pub(crate) use scripts::video::{OnReport, Report};
use scripts::video::{VIDEO_HANDLER, VIDEO_SCRIPT, VideoHandler};
use scripts::{FOCUS_HANDLER, FOCUS_SCRIPT, FocusHandler, text_script};

use crate::scheme::InternalPages;
use asks::Reply;
use asks::delegate::PageUi;

/// Where a page's reports and requests go. Called on the main thread, sometimes from inside a
/// call into the view: a hook records, it never calls back into the engine.
pub(crate) struct Hooks {
    /// A navigation of the page or of one of its frames is about to start, to this URL.
    pub(crate) on_navigation: Box<dyn Fn(&Url)>,
    /// A frame's response arrived; its document loads once the shields have its host's
    /// filters, which the second argument is called for.
    #[expect(
        clippy::type_complexity,
        reason = "a hook and the continuation it calls, written out where it is held"
    )]
    pub(crate) before_document: Box<dyn Fn(&Url, Box<dyn FnOnce()>)>,
    /// Navigations, titles, loading, links to open elsewhere, closing, full screen, crashes,
    /// blocked loads.
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
    /// What a window shows of the page: the view, filling it, and the page's Web Inspector
    /// when it docks, which `WebKit` puts next to the view in the view's superview and makes
    /// room for. A tab's inspector leaves and comes back with the tab.
    frame: Retained<NSView>,
    observer: Retained<PageObserver>,
    /// Keeps the UI delegate alive as long as the view refers to it.
    _ui: Retained<PageUi>,
    world: Retained<WKContentWorld>,
    /// Every page's script reports, this one's under its view.
    reports: Reports,
    /// How many times the page floated or docked its video: a float's chain of frames stops
    /// once a later call came.
    floats: Rc<Cell<u64>>,
}

impl WebPage {
    /// A new, empty page in `store` that serves `internal` pages and blocks what `content`'s
    /// rule lists block. `hooks` hear what it reports and asks; its scripts report into
    /// `reports`.
    pub(crate) fn new(
        store: &WKWebsiteDataStore,
        content: &WKUserContentController,
        internal: &InternalPages,
        extensions: Option<&WKWebExtensionController>,
        hooks: Hooks,
        reports: Reports,
        mtm: MainThreadMarker,
    ) -> Self {
        // SAFETY: a fresh configuration has no preconditions.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        // Web extensions run in the main jar's pages only; a page it opens copies this. The
        // setter exists only where the controller does (macOS 15.4).
        if let Some(controller) = extensions {
            // SAFETY: a controller made on the main thread; the configuration retains it.
            unsafe { configuration.setWebExtensionController(Some(controller)) };
        }
        // SAFETY: `store` is a valid data store; the configuration is copied on use.
        unsafe { configuration.setWebsiteDataStore(store) };
        // SAFETY: a valid controller; the copy keeps the same controller, so rule lists
        // installed later reach this page too.
        unsafe { configuration.setUserContentController(content) };
        // Sites sniff for Safari's `Version/… Safari/…` tail and serve a degraded page without
        // it (Outlook on the web falls back to its 2007 "light" version). Matching the installed
        // Safari exactly also blends us into Safari's crowd instead of a rare user agent.
        // SAFETY: a plain string setter; the configuration copies it.
        unsafe {
            configuration.setApplicationNameForUserAgent(Some(&NSString::from_str(
                &crate::user_agent::safari_tail(),
            )));
        }
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
        // The Fullscreen API. Our page script takes its calls into the tab's own window
        // ([`scripts::fullscreen`]); `WebKit`'s own stays on for what the script cannot reach
        // (the full-screen button of its media controls) and for whether a frame may ask at
        // all (`allowfullscreen`), which the script reads from it.
        // SAFETY: a plain boolean setter.
        unsafe { preferences.setElementFullscreenEnabled(true) };
        // Web Inspector, on our fork; a page it opens copies the preferences.
        inspector::allow(&preferences);
        // EXPERIMENT (not for commit): QUICKIE_WEBKIT_OFF=Key1,Key2 turns those fork features off.
        if let Ok(off) = std::env::var("QUICKIE_WEBKIT_OFF")
            && let Some(class) = objc2::runtime::AnyClass::get(c"WKPreferences")
        {
            {
                // SAFETY: experiment; `_features` returns an array of `_WKFeature`.
                let features: Option<
                    objc2::rc::Retained<objc2_foundation::NSArray<objc2::runtime::AnyObject>>,
                > = unsafe { objc2::msg_send![class, _features] };
                for feature in features.iter().flat_map(|all| all.iter()) {
                    // SAFETY: `_WKFeature` has a `key` string.
                    let key: objc2::rc::Retained<NSString> =
                        unsafe { objc2::msg_send![&*feature, key] };
                    if off.split(',').any(|name| name == key.to_string()) {
                        // SAFETY: the setter takes a bool and the feature.
                        unsafe {
                            let (): () = objc2::msg_send![&*preferences, _setEnabled: false, forFeature: &*feature];
                        }
                    }
                }
            }
        }
        Self::with_configuration(&configuration, hooks, reports, mtm)
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
        reports: Reports,
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
        // Web Inspector can also attach from Safari's Develop menu, the only way on the system
        // `WebKit` ([`inspector`]): public API since macOS 13.3, so asked for before it is
        // called on an older system.
        if view.respondsToSelector(sel!(setInspectable:)) {
            // SAFETY: a plain boolean setter the view answers to.
            unsafe { view.setInspectable(true) };
        }
        let frame = NSView::new(mtm);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        frame.addSubview(&view);
        let hooks = Rc::new(hooks);
        reports
            .fullscreen
            .borrow_mut()
            .insert(scripts::view_key(&view), Rc::downgrade(&hooks));
        let observer = PageObserver::new(Rc::clone(&hooks), mtm);
        observer.attach(&view);
        let ui = PageUi::new(hooks, mtm);
        // SAFETY: the delegate is weak; the page keeps it alive and clears it on drop.
        unsafe { view.setUIDelegate(Some(ProtocolObject::from_ref(&*ui))) };
        Self {
            view,
            frame,
            observer,
            _ui: ui,
            world,
            reports,
            floats: Rc::default(),
        }
    }

    /// The view the page draws in, edge to edge; it takes the keyboard.
    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }

    /// The view to embed in a window: the page, and its Web Inspector when it docks.
    pub(crate) fn frame(&self) -> &NSView {
        &self.frame
    }

    /// Puts the page's view back into its frame, filling it, after it was shown elsewhere
    /// alone (the mini player); `WebKit` makes room for a docked inspector again as the
    /// view's frame changes.
    pub(crate) fn reframe(&self) {
        // SAFETY: a getter; the superview is retained for the duration of the comparison.
        let parent = unsafe { self.view.superview() };
        if parent.is_some_and(|parent| parent == self.frame) {
            return;
        }
        self.view.removeFromSuperview();
        self.view.setFrame(self.frame.bounds());
        self.view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        self.frame.addSubview(&self.view);
    }

    /// Does what `asked` says with the page's Web Inspector ([`inspector`]).
    pub(crate) fn inspect(&self, asked: DevTools) -> Result<(), EngineError> {
        inspector::open(&self.view, asked)
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

    /// True while one of the page's frames says it holds the keyboard in an element that
    /// takes text.
    pub(crate) fn input_focused(&self) -> bool {
        self.reports
            .focus
            .borrow()
            .get(&scripts::view_key(&self.view))
            .is_some_and(|frames| !frames.is_empty())
    }

    /// The web view itself, to start a download in its jar or to hand to `WebKit` for a page it
    /// asked for.
    pub(crate) fn web_view(&self) -> &WKWebView {
        &self.view
    }

    /// Navigates to `url`. A local file may read the files beside it, as a saved page needs
    /// for its images and styles, and nothing else on disk.
    pub(crate) fn load(&self, url: &Url) -> Result<(), EngineError> {
        let ns_url = |url: &Url| {
            NSURL::URLWithString(&NSString::from_str(url.as_str()))
                .ok_or_else(|| EngineError(format!("not a URL WebKit can load: {url}")))
        };
        let parsed = ns_url(url)?;
        if url.scheme() == "file" {
            let folder = url
                .join(".")
                .map_err(|error| EngineError(error.to_string()))?;
            let folder = ns_url(&folder)?;
            // SAFETY: both are absolute file URLs, the second the folder of the first.
            let _navigation = unsafe {
                self.view
                    .loadFileURL_allowingReadAccessToURL(&parsed, &folder)
            };
            return Ok(());
        }
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

    pub(crate) fn reload_from_origin(&self) {
        // SAFETY: reloading has no preconditions.
        let _navigation = unsafe { self.view.reloadFromOrigin() };
    }

    pub(crate) fn stop(&self) {
        // SAFETY: stopping has no preconditions.
        unsafe { self.view.stopLoading() };
    }

    /// The page's zoom: `1.0` for actual size.
    pub(crate) fn zoom(&self) -> f64 {
        // SAFETY: a plain getter.
        unsafe { self.view.pageZoom() }
    }

    /// Zooms the page's content (text and layout, not a magnified picture) by `factor`. The
    /// view keeps it across navigations, so the caller sets it on every commit.
    pub(crate) fn set_zoom(&self, factor: f64) {
        if (self.zoom() - factor).abs() > f64::EPSILON {
            // SAFETY: a plain setter; any positive factor is valid.
            unsafe { self.view.setPageZoom(factor) };
        }
    }

    /// Opens the print panel as a sheet on the page's window; `WebKit` lays the page out in
    /// pages and prints it, or saves it as a PDF from the panel.
    pub(crate) fn print(&self) -> Result<(), EngineError> {
        let window = self
            .view
            .window()
            .ok_or_else(|| EngineError("the page is not in a window".to_owned()))?;
        // SAFETY: the shared print info always exists; the operation copies it.
        let operation = unsafe {
            self.view
                .printOperationWithPrintInfo(&NSPrintInfo::sharedPrintInfo())
        };
        operation.setShowsPrintPanel(true);
        operation.setShowsProgressPanel(true);
        let info = operation.printInfo();
        info.setHorizontalPagination(NSPrintingPaginationMode::Automatic);
        info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
        info.setVerticallyCentered(false);
        // WebKit's print view starts with an empty frame and prints blank pages unless it has
        // the page's size.
        if let Some(printed) = operation.view() {
            printed.setFrame(self.view.bounds());
        }
        // SAFETY: no delegate and no selector, so nothing is called back; a null context is
        // never read.
        unsafe {
            operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                &window,
                None,
                None,
                std::ptr::null_mut(),
            );
        }
        Ok(())
    }

    /// Renders the whole page as a PDF, off the main thread; `done` gets its bytes, or why it
    /// could not, on the main thread.
    pub(crate) fn pdf(&self, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
        let done = Cell::new(Some(done));
        let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
            // SAFETY: WebKit passes valid data or null for the duration of the call.
            let data = unsafe { data.as_ref() };
            // SAFETY: as above, an error or null.
            let error = unsafe { error.as_ref() };
            let made = match (data, error) {
                (Some(data), None) => Ok(data.to_vec()),
                (_, Some(error)) => Err(error.localizedDescription().to_string()),
                (None, None) => Err("WebKit made no PDF".to_owned()),
            };
            if let Some(done) = done.take() {
                done(made);
            }
        });
        // SAFETY: no configuration means the whole page; WebKit calls the block once on the
        // main thread, where its captures live.
        unsafe {
            self.view
                .createPDFWithConfiguration_completionHandler(None, &block);
        }
    }

    /// How much of the page has loaded, `0.0 ..= 1.0`, while it loads; `None` once it is done.
    pub(crate) fn progress(&self) -> Option<f64> {
        observer::progress_of(&self.view)
    }

    /// Selects a match of `query` as `step` says and marks all of them; `done` hears how many
    /// there are. An empty query clears the marks.
    pub(crate) fn find(&self, query: &str, step: FindStep, done: impl Fn(FindMatches) + 'static) {
        find::find(&self.view, query, step, done);
    }

    /// The size of the page view, in points.
    pub(crate) fn size(&self) -> NSSize {
        self.view.bounds().size
    }

    /// Renders `rect` of the visible page (in the view's flipped coordinates, origin top left)
    /// `width` points wide, at the screen's scale, and hands the image to `done` on the main
    /// thread; `None` when the page cannot be drawn. The web content process paints it, so the
    /// main thread only asks; a page just taken off screen is drawn from its last layout.
    pub(crate) fn snapshot(
        &self,
        rect: NSRect,
        width: f64,
        done: impl FnOnce(Option<Retained<NSImage>>) + 'static,
    ) {
        let mtm = self.view.mtm();
        // SAFETY: a fresh configuration has no preconditions.
        let configuration = unsafe { WKSnapshotConfiguration::new(mtm) };
        // SAFETY: a plain setter; any rect in the view's coordinates is valid.
        unsafe { configuration.setRect(rect) };
        // SAFETY: a plain setter; the configuration copies the number.
        unsafe { configuration.setSnapshotWidth(Some(&NSNumber::new_f64(width))) };
        // SAFETY: a plain setter. On macOS `WebKit` paints the snapshot in the web content
        // process either way; not waiting keeps a page just hidden from waiting for a screen
        // update it will never get.
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

    /// Reads at most `max_chars` characters of the main frame's rendered text for the agent;
    /// `done` gets them, empty when the page has none or the script failed.
    pub(crate) fn text(&self, max_chars: usize, done: impl FnOnce(String) + 'static) {
        self.evaluate(&text_script(max_chars), done);
    }

    /// Makes the page's video fill the view alone, for the mini player, in whichever frame it
    /// plays; the view is resized to the player after.
    pub(crate) fn float_video(&self) {
        self.floats.set(self.floats.get() + 1);
        let videos = self.reports.videos.borrow();
        let frames = videos.get(&scripts::view_key(&self.view));
        scripts::video::float(&self.view, &self.world, frames, &self.floats);
    }

    /// Puts the page back as it was before [`Self::float_video`], scrolled where it was.
    pub(crate) fn dock_video(&self) {
        self.floats.set(self.floats.get() + 1);
        let videos = self.reports.videos.borrow();
        let frames = videos.get(&scripts::view_key(&self.view));
        scripts::video::dock(&self.view, &self.world, frames);
    }

    /// Plays the page's video, or pauses it; the video script reports the change.
    pub(crate) fn play_video(&self, play: bool) {
        let videos = self.reports.videos.borrow();
        let frames = videos.get(&scripts::view_key(&self.view));
        scripts::video::play(&self.view, &self.world, frames, play);
    }

    /// Takes the page out of full screen: out of the page view through our Fullscreen API, and
    /// out of `WebKit`'s own full-screen window when its media controls put it there. The page
    /// reports leaving as it does on its own.
    pub(crate) fn exit_fullscreen(&self) {
        self.evaluate(fullscreen::EXIT_SCRIPT, |_| {});
        // SAFETY: a plain getter.
        let state = unsafe { self.view.fullscreenState() };
        if state != WKFullscreenState::NotInFullscreen {
            // SAFETY: no completion handler; it closes full screen and picture in picture,
            // which only the user's own controls open on this page.
            unsafe {
                self.view
                    .closeAllMediaPresentationsWithCompletionHandler(None);
            }
        }
    }

    /// Runs `script` in Quickie's world of the main frame; `done` gets its string answer, empty
    /// for anything else or an error.
    fn evaluate(&self, script: &str, done: impl FnOnce(String) + 'static) {
        scripts::evaluate(&self.view, &self.world, None, script, done);
    }
}

impl Drop for WebPage {
    fn drop(&mut self) {
        self.observer.detach(&self.view);
        // SAFETY: clearing a weak delegate has no preconditions.
        unsafe { self.view.setUIDelegate(None) };
        let key = scripts::view_key(&self.view);
        self.reports.focus.borrow_mut().remove(&key);
        self.reports.fullscreen.borrow_mut().remove(&key);
        self.reports.videos.borrow_mut().remove(&key);
        // SAFETY: stopping has no preconditions; a page going away loads nothing more.
        unsafe { self.view.stopLoading() };
        self.view.removeFromSuperview();
        self.frame.removeFromSuperview();
    }
}

/// Adds the video script to every frame of every page that uses `controller`, and one handler
/// that keeps each page's frames in `frames` and hands each change of a page's video to
/// `on_report`. Once per controller, as [`install_scripts`].
pub(crate) fn install_video_script(
    controller: &WKUserContentController,
    frames: &VideoFrames,
    on_report: OnReport,
    mtm: MainThreadMarker,
) {
    let world = scripts::world(mtm);
    let handler = VideoHandler::new(Rc::clone(frames), on_report, mtm);
    // SAFETY: the source is a complete script, injected at document start into every frame.
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(VIDEO_SCRIPT),
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
            &world,
        )
    };
    // SAFETY: the controller retains the script.
    unsafe { controller.addUserScript(&script) };
    // SAFETY: the controller retains the handler; this is the only handler under the name in
    // this world, as this runs once per controller.
    unsafe {
        controller.addScriptMessageHandler_contentWorld_name(
            ProtocolObject::from_ref(&*handler),
            &world,
            &NSString::from_str(VIDEO_HANDLER),
        );
    }
}

/// Adds the scripts every page of `controller` runs and reports through: the focus script and
/// the Fullscreen API, with one handler each that passes every page's reports into
/// `reports`. Once per controller: `WebKit` refuses a second handler under the same name, and
/// every page, popups included, shares them.
pub(crate) fn install_scripts(
    controller: &WKUserContentController,
    reports: &Reports,
    mtm: MainThreadMarker,
) {
    let world = scripts::world(mtm);
    // SAFETY: the page's own world always exists.
    let page_world = unsafe { WKContentWorld::pageWorld(mtm) };
    for (source, main_frame_only, in_world) in [
        (FOCUS_SCRIPT, false, &world),
        (fullscreen::PAGE_SCRIPT, false, &page_world),
        (fullscreen::RELAY_SCRIPT, true, &world),
    ] {
        // SAFETY: the source is a complete script, injected at document start.
        let script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                WKUserScript::alloc(mtm),
                &NSString::from_str(source),
                WKUserScriptInjectionTime::AtDocumentStart,
                main_frame_only,
                in_world,
            )
        };
        // SAFETY: the controller retains the script.
        unsafe { controller.addUserScript(&script) };
    }
    let focus = FocusHandler::new(Rc::clone(&reports.focus), mtm);
    let full = FullscreenHandler::new(Rc::clone(&reports.fullscreen), mtm);
    let handlers: [(&ProtocolObject<dyn WKScriptMessageHandler>, &str); 2] = [
        (ProtocolObject::from_ref(&*focus), FOCUS_HANDLER),
        (ProtocolObject::from_ref(&*full), fullscreen::HANDLER),
    ];
    for (handler, name) in handlers {
        // SAFETY: the controller retains the handler; it is the only one under its name in
        // this world, as the controller is new and this runs once for it.
        unsafe {
            controller.addScriptMessageHandler_contentWorld_name(
                handler,
                &world,
                &NSString::from_str(name),
            );
        }
    }
}

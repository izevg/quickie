//! One tab's page: a `WKWebView` in a data store, the observer that reports its navigations
//! and loading, and find in page.

mod find;
mod observer;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSView;
use objc2_foundation::{NSRect, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration, WKWebsiteDataStore};
use quickie_application::EngineError;
use quickie_domain_browsing::FindMatches;
use url::Url;

pub use find::FindStep;
pub(crate) use observer::PageEvent;
use observer::PageObserver;

use crate::scheme::InternalPages;

/// A page and its view. Dropping it stops observation and takes the view off screen.
pub(crate) struct WebPage {
    view: Retained<WKWebView>,
    observer: Retained<PageObserver>,
}

impl WebPage {
    /// A new, empty page in `store` that serves `internal` pages. `on_event` hears its committed
    /// navigations and title changes.
    pub(crate) fn new(
        store: &WKWebsiteDataStore,
        internal: &InternalPages,
        on_event: impl Fn(PageEvent) + 'static,
        mtm: MainThreadMarker,
    ) -> Self {
        // SAFETY: a fresh configuration has no preconditions.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        // SAFETY: `store` is a valid data store; the configuration is copied on use.
        unsafe { configuration.setWebsiteDataStore(store) };
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
        // SAFETY: the frame is any rect and the configuration is fully set up.
        let view = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::ZERO,
                &configuration,
            )
        };
        let observer = PageObserver::new(Box::new(on_event), mtm);
        observer.attach(&view);
        Self { view, observer }
    }

    /// The view to embed; it draws the page edge to edge.
    pub(crate) fn view(&self) -> &NSView {
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
}

impl Drop for WebPage {
    fn drop(&mut self) {
        self.observer.detach(&self.view);
        // SAFETY: stopping has no preconditions; a page going away loads nothing more.
        unsafe { self.view.stopLoading() };
        self.view.removeFromSuperview();
    }
}

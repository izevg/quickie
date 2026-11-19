//! Development engine of Quickie: pages on `WKWebView`, so UI work never waits for a Chromium
//! build (`docs/architecture.md` §3, P0).
//!
//! The surface is engine-neutral on purpose: it is what the `EnginePort` of `application` asks
//! of an engine (create a page and embed its view, navigate, read URL and title, hear about
//! changes), and `engine-chromium` offers the same.

#![expect(
    unsafe_code,
    reason = "WebKit through objc2: its bindings mark every method unsafe, and KVO needs a class"
)]

mod observer;

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSView;
use objc2_foundation::{NSRect, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration, WKWebsiteDataStore};

use observer::PageObserver;

/// Where a page keeps cookies, caches, and storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Storage {
    /// The profile's persistent store.
    #[default]
    Persistent,
    /// A throwaway jar: in memory only, gone with the page.
    Ephemeral,
}

/// What a page reports while it lives. Delivered on the main thread.
#[derive(Clone, Debug, PartialEq)]
pub enum PageEvent {
    TitleChanged(Option<String>),
    UrlChanged(Option<String>),
    /// Load progress from 0.0 to 1.0.
    ProgressChanged(f64),
}

/// A navigation the engine refused before it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    InvalidUrl(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "not a URL the engine can load: {url}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Creates pages. Lives on the main thread, like every view it hands out.
#[derive(Debug)]
pub struct WebKitEngine {
    mtm: MainThreadMarker,
}

impl WebKitEngine {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self { mtm }
    }

    /// A new, empty page. `on_event` hears its title, URL, and progress changes.
    #[must_use]
    pub fn create_page(&self, storage: Storage, on_event: impl Fn(PageEvent) + 'static) -> WebPage {
        let mtm = self.mtm;
        // SAFETY: a fresh configuration has no preconditions.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        let store = match storage {
            // SAFETY: the shared default store exists for the life of the process.
            Storage::Persistent => unsafe { WKWebsiteDataStore::defaultDataStore(mtm) },
            // SAFETY: creating a non-persistent store has no preconditions.
            Storage::Ephemeral => unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) },
        };
        // SAFETY: `store` is a valid data store; the configuration is copied on use.
        unsafe { configuration.setWebsiteDataStore(&store) };
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
        WebPage { view, observer }
    }
}

/// One page and its view. Dropping it stops observation; the view lives on while its superview
/// holds it.
pub struct WebPage {
    view: Retained<WKWebView>,
    observer: Retained<PageObserver>,
}

impl WebPage {
    /// The view to embed; it draws the page edge to edge.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Navigates to `url`, which must be absolute.
    ///
    /// # Errors
    ///
    /// [`LoadError::InvalidUrl`] when Foundation cannot parse `url`.
    pub fn load(&self, url: &str) -> Result<(), LoadError> {
        let invalid = || LoadError::InvalidUrl(url.to_owned());
        let parsed = NSURL::URLWithString(&NSString::from_str(url)).ok_or_else(invalid)?;
        if parsed.scheme().is_none() {
            return Err(invalid());
        }
        let request = NSURLRequest::requestWithURL(&parsed);
        // SAFETY: `request` is a valid request for an absolute URL.
        let _navigation = unsafe { self.view.loadRequest(&request) };
        Ok(())
    }

    /// Shows `html` as a page of its own with no base URL, so it can load nothing relative.
    pub fn load_html(&self, html: &str) {
        // SAFETY: any string is valid HTML input; a missing base URL is allowed.
        let _navigation = unsafe {
            self.view
                .loadHTMLString_baseURL(&NSString::from_str(html), None)
        };
    }

    pub fn go_back(&self) {
        // SAFETY: a no-op when there is nothing to go back to.
        let _navigation = unsafe { self.view.goBack() };
    }

    pub fn go_forward(&self) {
        // SAFETY: a no-op when there is nothing to go forward to.
        let _navigation = unsafe { self.view.goForward() };
    }

    pub fn reload(&self) {
        // SAFETY: reloading has no preconditions.
        let _navigation = unsafe { self.view.reload() };
    }

    pub fn stop(&self) {
        // SAFETY: stopping has no preconditions.
        unsafe { self.view.stopLoading() };
    }

    /// The committed URL, if any.
    #[must_use]
    pub fn url(&self) -> Option<String> {
        url_of(&self.view)
    }

    #[must_use]
    pub fn title(&self) -> Option<String> {
        title_of(&self.view)
    }

    /// Load progress from 0.0 to 1.0.
    #[must_use]
    pub fn progress(&self) -> f64 {
        progress_of(&self.view)
    }
}

impl Drop for WebPage {
    fn drop(&mut self) {
        self.observer.detach(&self.view);
    }
}

fn url_of(view: &WKWebView) -> Option<String> {
    // SAFETY: a getter with no preconditions.
    let url = unsafe { view.URL() }?;
    url.absoluteString().map(|s| s.to_string())
}

fn title_of(view: &WKWebView) -> Option<String> {
    // SAFETY: a getter with no preconditions.
    let title = unsafe { view.title() }?;
    Some(title.to_string()).filter(|t| !t.is_empty())
}

fn progress_of(view: &WKWebView) -> f64 {
    // SAFETY: a getter with no preconditions.
    unsafe { view.estimatedProgress() }
}

#[cfg(test)]
mod tests {
    use super::{LoadError, Storage};

    #[test]
    fn pages_persist_unless_asked_otherwise() {
        assert_eq!(Storage::default(), Storage::Persistent);
    }

    #[test]
    fn load_errors_name_the_url() {
        let error = LoadError::InvalidUrl("not a url".to_owned());
        assert_eq!(
            error.to_string(),
            "not a URL the engine can load: not a url"
        );
    }
}

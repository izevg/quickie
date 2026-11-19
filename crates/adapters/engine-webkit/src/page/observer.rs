//! Watches one `WKWebView`: its committed navigations and the navigations that become
//! downloads (as its navigation delegate) and its title (by key-value observation), turned into
//! [`PageEvent`]s.

use std::ffi::c_void;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{
    NSHTTPURLResponse, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSString,
};
use objc2_web_kit::{
    WKDownload, WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationResponse, WKNavigationResponsePolicy, WKWebView,
};

use crate::downloads::wants_download;

/// What a page reports while it lives. Delivered on the main thread, sometimes from inside a
/// call into the view: a handler must only record it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PageEvent {
    /// The main frame committed a navigation: the URL is the page's now.
    Committed {
        url: Option<String>,
        title: Option<String>,
    },
    TitleChanged(Option<String>),
    /// The page started or stopped loading, or loaded more of it.
    Progress,
    /// A navigation became a download; it needs a delegate before this call returns.
    Download(Retained<WKDownload>),
}

/// The title is observed by its key path.
const TITLE: &str = "title";

/// Loading is observed by these key paths.
const PROGRESS: [&str; 2] = ["estimatedProgress", "loading"];

pub(crate) struct Ivars {
    on_event: Box<dyn Fn(PageEvent)>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `PageObserver` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitPageObserver"]
    #[ivars = Ivars]
    pub(crate) struct PageObserver;

    impl PageObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            let (Some(key_path), Some(view)) =
                (key_path, object.and_then(AnyObject::downcast_ref::<WKWebView>))
            else {
                return;
            };
            let key_path = key_path.to_string();
            if key_path == TITLE {
                (self.ivars().on_event)(PageEvent::TitleChanged(title_of(view)));
            } else if PROGRESS.contains(&key_path.as_str()) {
                (self.ivars().on_event)(PageEvent::Progress);
            }
        }
    }

    unsafe impl NSObjectProtocol for PageObserver {}

    // SAFETY: each method matches the protocol's signature; the rest are optional.
    unsafe impl WKNavigationDelegate for PageObserver {
        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            (self.ivars().on_event)(PageEvent::Committed {
                url: url_of(view),
                title: title_of(view),
            });
        }

        /// A link with a `download` attribute downloads; everything else loads.
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            decide: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: a getter with no preconditions.
            let policy = if unsafe { action.shouldPerformDownload() } {
                WKNavigationActionPolicy::Download
            } else {
                WKNavigationActionPolicy::Allow
            };
            decide.call((policy,));
        }

        /// A response `WebKit` cannot show, or an attachment, downloads.
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _view: &WKWebView,
            response: &WKNavigationResponse,
            decide: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            // SAFETY: a getter with no preconditions.
            let can_show = unsafe { response.canShowMIMEType() };
            // SAFETY: as above.
            let answer = unsafe { response.response() };
            let disposition = answer
                .downcast_ref::<NSHTTPURLResponse>()
                .and_then(|http| {
                    http.valueForHTTPHeaderField(&NSString::from_str("Content-Disposition"))
                })
                .map(|value| value.to_string());
            let policy = if wants_download(can_show, disposition.as_deref()) {
                WKNavigationResponsePolicy::Download
            } else {
                WKNavigationResponsePolicy::Allow
            };
            decide.call((policy,));
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_became_download(
            &self,
            _view: &WKWebView,
            _action: &WKNavigationAction,
            download: &WKDownload,
        ) {
            (self.ivars().on_event)(PageEvent::Download(download.retain()));
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_became_download(
            &self,
            _view: &WKWebView,
            _response: &WKNavigationResponse,
            download: &WKDownload,
        ) {
            (self.ivars().on_event)(PageEvent::Download(download.retain()));
        }
    }
);

impl PageObserver {
    pub(crate) fn new(on_event: Box<dyn Fn(PageEvent)>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { on_event });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// Starts watching `view`; [`PageObserver::detach`] must run before either is released.
    pub(crate) fn attach(&self, view: &WKWebView) {
        // SAFETY: the delegate is weak; the page that owns both keeps the observer alive and
        // clears the delegate in `detach`.
        unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(self))) };
        for key_path in key_paths() {
            // SAFETY: the observer is an `NSObject` that implements
            // `observeValueForKeyPath:ofObject:change:context:`, and the null context is valid.
            unsafe {
                view.addObserver_forKeyPath_options_context(
                    self,
                    &NSString::from_str(key_path),
                    NSKeyValueObservingOptions::New,
                    std::ptr::null_mut(),
                );
            }
        }
    }

    pub(crate) fn detach(&self, view: &WKWebView) {
        // SAFETY: clearing a weak delegate has no preconditions.
        unsafe { view.setNavigationDelegate(None) };
        for key_path in key_paths() {
            // SAFETY: `attach` registered this observer for exactly this key path with a null
            // context, and it is removed once.
            unsafe {
                view.removeObserver_forKeyPath_context(
                    self,
                    &NSString::from_str(key_path),
                    std::ptr::null_mut(),
                );
            }
        }
    }
}

/// Every key path the observer watches.
fn key_paths() -> impl Iterator<Item = &'static str> {
    std::iter::once(TITLE).chain(PROGRESS)
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

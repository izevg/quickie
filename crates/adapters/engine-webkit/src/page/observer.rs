//! Watches one `WKWebView`: its committed navigations, the navigations that become downloads,
//! the links it opens elsewhere, and the end of its web content process (as its navigation
//! delegate), its title, loading, and full screen (by key-value observation), turned into
//! [`PageEvent`]s.

use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Rc;

use block2::DynBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{
    NSHTTPURLResponse, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration,
    NSString, NSURL,
};
use objc2_web_kit::{
    WKDownload, WKFullscreenState, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationDelegate, WKNavigationResponse, WKNavigationResponsePolicy, WKNavigationType,
    WKWebView,
};
use quickie_domain_browsing::Focus;
use url::Url;

use super::Hooks;
use super::asks::link_focus;
use crate::downloads::wants_download;
use crate::shields;

/// What a page reports while it lives. Delivered on the main thread, sometimes from inside a
/// call into the view: a handler must only record it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PageEvent {
    /// The main frame committed a navigation: the URL is the page's now.
    Committed {
        url: Option<String>,
        title: Option<String>,
    },
    TitleChanged(Option<String>),
    /// The page started or stopped loading, or loaded more of it: how much, `0.0 ..= 1.0`, or
    /// `None` once it stopped.
    Progress(Option<f64>),
    /// A navigation became a download; it needs a delegate before this call returns.
    Download(Retained<WKDownload>),
    /// A link the user opened in another tab (`⌘`-click, middle-click); the page stays.
    OpenLink {
        url: String,
        focus: Focus,
    },
    /// The page closed itself (`window.close()`).
    Closed,
    /// `WebKit` showed an element of the page in its own full-screen window (`true`), or took
    /// it back (`false`).
    Fullscreen(bool),
    /// An element of the page filled the page view (`true`), or stopped (`false`), as our
    /// Fullscreen API reports it.
    FullscreenInWindow(bool),
    /// `WebKit` opened (`true`) or closed (`false`) native picture-in-picture.
    PictureInPicture(bool),
    /// The native player's return button needs the original page to become visible.
    PictureInPictureReturnRequested,
    /// The web content process that rendered the page terminated: the view is blank until it
    /// loads again.
    Crashed,
    /// One of Quickie's content rule lists kept a load of the page from happening.
    Blocked,
}

/// The title is observed by its key path.
const TITLE: &str = "title";

/// Loading is observed by these key paths.
const PROGRESS: [&str; 2] = ["estimatedProgress", "loading"];

/// Full screen is observed by its key path.
const FULLSCREEN: &str = "fullscreenState";

pub(crate) struct Ivars {
    hooks: Rc<Hooks>,
    frame: Weak<NSView>,
    document: Cell<(u64, bool)>,
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
            let on_event = &self.ivars().hooks.on_event;
            if key_path == TITLE {
                on_event(PageEvent::TitleChanged(title_of(view)));
            } else if PROGRESS.contains(&key_path.as_str()) {
                on_event(PageEvent::Progress(progress_of(view)));
            } else if key_path == FULLSCREEN {
                // SAFETY: a getter with no preconditions.
                match unsafe { view.fullscreenState() } {
                    WKFullscreenState::InFullscreen => {
                        // SAFETY: the view is live on the main thread; this getter does not mutate it.
                        let host = unsafe { view.superview() };
                        if self.ivars().frame.load().is_some_and(|frame| host.as_deref() == Some(&*frame)) {
                            on_event(PageEvent::FullscreenInWindow(true));
                        } else {
                            on_event(PageEvent::Fullscreen(true));
                        }
                    }
                    WKFullscreenState::NotInFullscreen => on_event(PageEvent::Fullscreen(false)),
                    _ => {}
                }
            }
        }

        /// `WebKit`'s private `WKNavigationDelegatePrivate` callback, the one Safari and other
        /// content blockers count with: a content rule list acted on one of the page's loads.
        /// `WebKit` calls it only because this delegate responds to it; an older or newer
        /// `WebKit` without it simply never calls, and the counts stay at zero. The action is a
        /// private `_WKContentRuleListAction`, asked for `blockedLoad` only when it responds to
        /// it. Only Quickie's own rule lists count.
        #[unsafe(method(_webView:contentRuleListWithIdentifier:performedAction:forURL:))]
        fn rule_list_acted(
            &self,
            _view: &WKWebView,
            identifier: Option<&NSString>,
            action: Option<&AnyObject>,
            _url: Option<&NSURL>,
        ) {
            let ours = identifier.is_some_and(|id| shields::is_ours(&id.to_string()));
            if ours && action.is_some_and(blocked_load) {
                (self.ivars().hooks.on_event)(PageEvent::Blocked);
            }
        }
    }

    unsafe impl NSObjectProtocol for PageObserver {}

    // SAFETY: each method matches the protocol's signature; the rest are optional.
    unsafe impl WKNavigationDelegate for PageObserver {
        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.document_changed(true);
            let hooks = &self.ivars().hooks;
            hooks.reset_picture_in_picture();
            (hooks.on_event)(PageEvent::Committed {
                url: url_of(view),
                title: title_of(view),
            });
        }

        /// A link opened with `⌘` or the middle button goes to another tab; a link with a
        /// `download` attribute downloads; everything else loads.
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            decide: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            if let Some((url, focus)) = elsewhere(action) {
                (self.ivars().hooks.on_event)(PageEvent::OpenLink { url, focus });
                decide.call((WKNavigationActionPolicy::Cancel,));
                return;
            }
            // SAFETY: a getter with no preconditions.
            let policy = if unsafe { action.shouldPerformDownload() } {
                WKNavigationActionPolicy::Download
            } else {
                // SAFETY: getters with no preconditions.
                let url = unsafe { action.request() }.URL();
                if let Some(url) = url.as_deref().and_then(parsed) {
                    (self.ivars().hooks.on_navigation)(&url);
                }
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
            if wants_download(can_show, disposition.as_deref()) {
                return decide.call((WKNavigationResponsePolicy::Download,));
            }
            // The document waits for its shields' scripts: they must be in place before the
            // page's first script runs.
            let Some(url) = answer.URL().as_deref().and_then(parsed) else {
                return decide.call((WKNavigationResponsePolicy::Allow,));
            };
            let decide = decide.copy();
            (self.ivars().hooks.before_document)(
                &url,
                Box::new(move || decide.call((WKNavigationResponsePolicy::Allow,))),
            );
        }

        /// `WebKit` does not reload a page whose process died: it stays blank until it is told.
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn content_process_terminated(&self, _view: &WKWebView) {
            self.document_changed(false);
            let hooks = &self.ivars().hooks;
            hooks.reset_picture_in_picture();
            (hooks.on_event)(PageEvent::Crashed);
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_became_download(
            &self,
            _view: &WKWebView,
            _action: &WKNavigationAction,
            download: &WKDownload,
        ) {
            (self.ivars().hooks.on_event)(PageEvent::Download(download.retain()));
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_became_download(
            &self,
            _view: &WKWebView,
            _response: &WKNavigationResponse,
            download: &WKDownload,
        ) {
            (self.ivars().hooks.on_event)(PageEvent::Download(download.retain()));
        }
    }
);

/// A URL `WebKit` handed over, when it parses.
fn parsed(url: &NSURL) -> Option<Url> {
    Url::parse(&url.absoluteString()?.to_string()).ok()
}

/// Whether a private `_WKContentRuleListAction` says a load was blocked; false for an object
/// that does not answer `blockedLoad`.
fn blocked_load(action: &AnyObject) -> bool {
    let selector = sel!(blockedLoad);
    // SAFETY: `respondsToSelector:` is `NSObject`'s and takes any selector.
    let responds: bool = unsafe { msg_send![action, respondsToSelector: selector] };
    // SAFETY: the object responds to `blockedLoad`, a `BOOL` getter without arguments.
    responds && unsafe { msg_send![action, blockedLoad] }
}

/// The link and the tab it goes to, when the user opened it in another tab.
fn elsewhere(action: &WKNavigationAction) -> Option<(String, Focus)> {
    // SAFETY: a getter with no preconditions.
    if unsafe { action.navigationType() } != WKNavigationType::LinkActivated {
        return None;
    }
    // SAFETY: as above.
    let modifiers = unsafe { action.modifierFlags() };
    // SAFETY: as above.
    let focus = link_focus(modifiers, unsafe { action.buttonNumber() })?;
    // SAFETY: a getter; every action has a request.
    let url = unsafe { action.request() }.URL()?.absoluteString()?;
    Some((url.to_string(), focus))
}

impl PageObserver {
    pub(crate) fn new(
        hooks: Rc<Hooks>,
        frame: &Retained<NSView>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            hooks,
            frame: Weak::from_retained(frame),
            document: Cell::default(),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    pub(crate) fn document_revision(&self) -> Option<u64> {
        let (revision, live) = self.ivars().document.get();
        live.then_some(revision)
    }

    fn document_changed(&self, live: bool) {
        let (revision, _) = self.ivars().document.get();
        self.ivars().document.set((revision.wrapping_add(1), live));
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
    [TITLE, FULLSCREEN].into_iter().chain(PROGRESS)
}

/// How much of the view's page has loaded, `0.0 ..= 1.0`, while it loads; `None` once it is
/// done.
pub(crate) fn progress_of(view: &WKWebView) -> Option<f64> {
    // SAFETY: getters with no preconditions.
    let loading = unsafe { view.isLoading() };
    // SAFETY: as above.
    loading.then(|| unsafe { view.estimatedProgress() }.clamp(0.0, 1.0))
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

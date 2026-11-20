//! What Quickie runs inside a page, and what the page tells it back.
//!
//! Every script runs in Quickie's own content world: it reads the same DOM as the site, but the
//! site's JavaScript cannot see its globals or replace the functions it calls, and it adds
//! nothing to the page's own world. The scripts only read the DOM and listen; the one that
//! follows a link hint focuses or clicks the element the user picked.
//!
//! Here: the world, the focus reports, and the page text the agent reads. [`hints`]: the
//! link-hint scripts and their answers.

pub(crate) mod hints;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSNumber, NSString};
use objc2_web_kit::{
    WKContentWorld, WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKWebView,
};

/// The content world every Quickie script runs in.
pub(crate) fn world(mtm: MainThreadMarker) -> Retained<WKContentWorld> {
    // SAFETY: a named world is created on first use and shared while it lives.
    unsafe { WKContentWorld::worldWithName(&NSString::from_str("quickie"), mtm) }
}

/// Reads the main frame's rendered text, as the user sees it (`innerText`: no hidden elements,
/// no markup, no script or style text), for the agent to answer about: at most `max_chars`
/// UTF-16 units, so at most that many characters. Runs in Quickie's world, so the page cannot
/// replace `innerText` or see that it was read.
pub(crate) fn text_script(max_chars: usize) -> String {
    format!(
        r"(() => {{
  const root = document.body || document.documentElement;
  const text = root ? root.innerText : '';
  return String(text || '').slice(0, {max_chars});
}})()"
    )
}

/// The name the focus script posts under.
pub(crate) const FOCUS_HANDLER: &str = "quickieFocus";

/// Installed in every frame at document start: tells whether the focused element takes text,
/// on every focus change. A frame whose focus is inside a child frame stays silent; the child
/// reports. The last report wins, and focus leaving one frame for another reports `false`
/// before the other's `true`. On load (and on a return from the back-forward cache) only the
/// main frame and a frame that has focus report: a frame added or loaded while a field
/// elsewhere has focus would otherwise report `false` over the field's `true`, and keys typed
/// into the field would become the shell's.
pub(crate) const FOCUS_SCRIPT: &str = r"(() => {
  const textless = new Set(['button', 'checkbox', 'color', 'file', 'hidden', 'image', 'radio',
    'range', 'reset', 'submit']);
  const editable = (el) => !!el && (el.isContentEditable || el.tagName === 'TEXTAREA' ||
    el.tagName === 'SELECT' ||
    (el.tagName === 'INPUT' && !textless.has(String(el.type).toLowerCase())));
  const post = () => {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    if (el && (el.tagName === 'IFRAME' || el.tagName === 'FRAME')) return;
    try { window.webkit.messageHandlers.quickieFocus.postMessage(editable(el)); } catch (_) {}
  };
  const loaded = () => { if (window === window.top || document.hasFocus()) post(); };
  addEventListener('focusin', post, true);
  addEventListener('focusout', () => setTimeout(post, 0), true);
  addEventListener('pageshow', loaded, true);
  loaded();
})();";

/// Each page's last report of whether its focused element takes text, by [`view_key`].
pub(crate) type FocusReports = Rc<RefCell<HashMap<usize, bool>>>;

/// The key of a page's view in [`FocusReports`]: its address, unique while it lives.
pub(crate) fn view_key(view: &WKWebView) -> usize {
    std::ptr::from_ref(view) as usize
}

pub(crate) struct Ivars {
    reports: FocusReports,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `FocusHandler` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitFocusHandler"]
    #[ivars = Ivars]
    pub(crate) struct FocusHandler;

    unsafe impl NSObjectProtocol for FocusHandler {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl WKScriptMessageHandler for FocusHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: a getter; the body is whatever the script posted.
            let body = unsafe { message.body() };
            let focused = body
                .downcast_ref::<NSNumber>()
                .is_some_and(NSNumber::as_bool);
            // SAFETY: a getter; the page that posted, `None` once it is gone.
            if let Some(view) = unsafe { message.webView() } {
                self.ivars()
                    .reports
                    .borrow_mut()
                    .insert(view_key(&view), focused);
            }
        }
    }
);

impl FocusHandler {
    /// A handler that keeps `reports` up to date with every page's reports.
    pub(crate) fn new(reports: FocusReports, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { reports });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

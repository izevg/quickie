//! Full screen in the tab's own window. `WebKit` on macOS shows an element that asks for full
//! screen in a full-screen window of its own, on a Space of its own, and has no setting for
//! anything else. So every page gets the Fullscreen API from a script of ours instead
//! (`fullscreen.js`): the element fills the view in the tab's existing window.
//!
//! The script must replace the page's own functions, so it runs in the page's world, where the
//! page could post to a handler as well. It reports to Quickie's world instead, with an event
//! on the document; [`RELAY_SCRIPT`] there checks that the user's click allowed it and posts
//! to [`FullscreenHandler`], which tells the page's hooks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSString;
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};

use super::view_key;
use crate::page::{Hooks, PageEvent};

/// The Fullscreen API in the page's view: in the page's own world, in every frame, at
/// document start.
pub(crate) const PAGE_SCRIPT: &str = include_str!("fullscreen.js");

/// The name the relay posts under.
pub(crate) const HANDLER: &str = "quickieFullscreen";

/// In Quickie's world of the main frame, at document start: passes the page script's reports
/// on to [`HANDLER`]. Entering counts only while the user's click is fresh, so a page cannot
/// enter fullscreen by itself; leaving always counts, and a page that goes away
/// while full screen leaves.
pub(crate) const RELAY_SCRIPT: &str = r"(() => {
  let entered = false;
  const post = (state) => {
    try { window.webkit.messageHandlers.quickieFullscreen.postMessage(state); } catch (_) {}
  };
  const leave = () => { if (entered) { entered = false; post('exit'); } };
  document.addEventListener('quickie-fullscreen-enter', () => {
    const user = navigator.userActivation;
    if (user && !user.isActive) return;
    entered = true;
    post('enter');
  }, true);
  document.addEventListener('quickie-fullscreen-exit', leave, true);
  addEventListener('pagehide', leave, true);
})();";

/// In Quickie's world of the main frame: takes the page out of full screen.
pub(crate) const EXIT_SCRIPT: &str =
    "document.dispatchEvent(new Event('quickie-fullscreen-leave')); ''";

/// The pages whose reports the handler passes on, by [`view_key`].
pub(crate) type FullscreenPages = Rc<RefCell<HashMap<usize, Weak<Hooks>>>>;

/// What a report of the relay says: `Some(true)` entered, `Some(false)` left, `None` for
/// anything that is not one.
pub(crate) fn report(body: &str) -> Option<bool> {
    match body {
        "enter" => Some(true),
        "exit" => Some(false),
        _ => None,
    }
}

pub(crate) struct Ivars {
    pages: FullscreenPages,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `FullscreenHandler` implements
    // no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitFullscreenHandler"]
    #[ivars = Ivars]
    pub(crate) struct FullscreenHandler;

    unsafe impl NSObjectProtocol for FullscreenHandler {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl WKScriptMessageHandler for FullscreenHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: a getter; the frame that posted is always there.
            let frame = unsafe { message.frameInfo() };
            // SAFETY: a plain getter.
            let main = unsafe { frame.isMainFrame() };
            // SAFETY: a getter; the body is whatever the relay posted.
            let body = unsafe { message.body() };
            let state = body
                .downcast_ref::<NSString>()
                .and_then(|body| report(&body.to_string()));
            // SAFETY: a getter; the page that posted, `None` once it is gone.
            let view = unsafe { message.webView() };
            let (true, Some(entered), Some(view)) = (main, state, view) else {
                return;
            };
            let hooks = self
                .ivars()
                .pages
                .borrow()
                .get(&view_key(&view))
                .and_then(Weak::upgrade);
            if let Some(hooks) = hooks {
                (hooks.on_event)(PageEvent::FullscreenInWindow(entered));
            }
        }
    }
);

impl FullscreenHandler {
    /// A handler that passes each page's reports on to its hooks in `pages`.
    pub(crate) fn new(pages: FullscreenPages, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { pages });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::report;

    #[test]
    fn a_report_is_entering_or_leaving() {
        assert_eq!(report("enter"), Some(true));
        assert_eq!(report("exit"), Some(false));
        for junk in ["", "Enter", "enter ", "1"] {
            assert_eq!(report(junk), None, "{junk:?}");
        }
    }
}

//! What Quickie runs inside a page, and what the page tells it back.
//!
//! Every script runs in Quickie's own content world: it reads the same DOM as the site, but the
//! site's JavaScript cannot see its globals or replace the functions it calls, and it adds
//! nothing to the page's own world. The scripts only read the DOM and listen; the one that
//! follows a link hint focuses or clicks the element the user picked, and while the mini player
//! shows a page's video, the page (and each frame on the way to the video) carries a style
//! sheet and marked elements that make the video fill it, taken away when it docks.
//!
//! Here: the world, running a script in a frame ([`evaluate`]), the focus reports, and the
//! page text the agent reads. [`hints`]: the link-hint scripts and their answers.
//! [`fullscreen`]: the Fullscreen API in the tab's own window, the one script that runs in the
//! page's world. [`video`]: what the page's video does, in any of its frames, and floating it
//! for the mini player.

pub(crate) mod fullscreen;
pub(crate) mod hints;
pub(crate) mod video;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKScriptMessage, WKScriptMessageHandler, WKUserContentController,
    WKWebView,
};

/// The content world every Quickie script runs in.
pub(crate) fn world(mtm: MainThreadMarker) -> Retained<WKContentWorld> {
    // SAFETY: a named world is created on first use and shared while it lives.
    unsafe { WKContentWorld::worldWithName(&NSString::from_str("quickie"), mtm) }
}

/// Runs `script` in `world` of `view`'s `frame`, the main frame when `None`; `done` gets its
/// string answer, empty for anything else or an error (a frame that went away).
pub(crate) fn evaluate(
    view: &WKWebView,
    world: &WKContentWorld,
    frame: Option<&WKFrameInfo>,
    script: &str,
    done: impl FnOnce(String) + 'static,
) {
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
    // SAFETY: a frame of this view or none (the main frame); WebKit calls the block once on
    // the main thread, where its captures live, with an error for a frame that is gone.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(script),
            frame,
            world,
            Some(&block),
        );
    }
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

/// Installed in every frame at document start. Each frame names itself with a random token and
/// says whether its focused element (through open shadow roots) takes text: a text field, a
/// text area, a select or editable content. A frame that focus leaves loses its focused
/// element, so at most one frame says yes. It says so again whenever that changes: on focus
/// moving in or out of it, on a return from the back-forward cache, and `false` when it goes
/// away. Whether the page itself has the keyboard is the shell's to know, not the page's.
///
/// A report is a string: `R` when the main frame starts a new document (forget every frame
/// of the old one) or `-`, then `1` or `0`, then the frame's token. The page takes text while
/// any of its frames says `1`, so the order in which frames report does not matter: a frame
/// that loads while a field elsewhere has focus, or focus moving from one frame to another,
/// cannot hide the field.
pub(crate) const FOCUS_SCRIPT: &str = r"(() => {
  const textless = new Set(['button', 'checkbox', 'color', 'file', 'hidden', 'image', 'radio',
    'range', 'reset', 'submit']);
  const editable = (el) => !!el && (el.isContentEditable || el.tagName === 'TEXTAREA' ||
    el.tagName === 'SELECT' ||
    (el.tagName === 'INPUT' && !textless.has(String(el.type).toLowerCase())));
  const frame = Math.random().toString(36).slice(2) + Date.now().toString(36);
  let last;
  const send = (focused, reset) => {
    last = focused;
    try {
      window.webkit.messageHandlers.quickieFocus.postMessage(
        (reset ? 'R' : '-') + (focused ? '1' : '0') + frame);
    } catch (_) {}
  };
  const focused = () => {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    return editable(el);
  };
  const post = () => { const now = focused(); if (now !== last) send(now, false); };
  addEventListener('focusin', post, true);
  addEventListener('focusout', () => setTimeout(post, 0), true);
  addEventListener('pageshow', () => send(focused(), false), true);
  addEventListener('pagehide', () => send(false, false), true);
  send(focused(), window === window.top);
})();";

/// For each page, by [`view_key`], the tokens of its frames that hold the keyboard in an
/// element that takes text.
pub(crate) type FocusReports = Rc<RefCell<HashMap<usize, HashSet<String>>>>;

/// Applies one report of the focus script to a page's frames. A report that is not the
/// script's changes nothing.
pub(crate) fn apply_focus_report(frames: &mut HashSet<String>, report: &str) {
    let mut chars = report.chars();
    let (Some(reset), Some(focused)) = (chars.next(), chars.next()) else {
        return;
    };
    let frame = chars.as_str();
    if !matches!(reset, 'R' | '-') || !matches!(focused, '0' | '1') || frame.is_empty() {
        return;
    }
    if reset == 'R' {
        frames.clear();
    }
    if focused == '1' {
        frames.insert(frame.to_owned());
    } else {
        frames.remove(frame);
    }
}

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
            let Some(report) = body.downcast_ref::<NSString>().map(NSString::to_string) else {
                return;
            };
            // SAFETY: a getter; the page that posted, `None` once it is gone.
            if let Some(view) = unsafe { message.webView() } {
                let mut reports = self.ivars().reports.borrow_mut();
                apply_focus_report(reports.entry(view_key(&view)).or_default(), &report);
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::apply_focus_report;

    fn after(reports: &[&str]) -> HashSet<String> {
        let mut frames = HashSet::new();
        for report in reports {
            apply_focus_report(&mut frames, report);
        }
        frames
    }

    fn frames(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_frame_that_takes_text_is_kept_until_it_says_otherwise() {
        assert_eq!(after(&["-1a"]), frames(&["a"]));
        assert_eq!(after(&["-1a", "-0a"]), frames(&[]));
    }

    #[test]
    fn another_frame_saying_no_does_not_hide_a_field() {
        assert_eq!(after(&["-1a", "-0b"]), frames(&["a"]));
    }

    #[test]
    fn focus_moving_between_frames_ends_in_the_new_one_in_either_order() {
        assert_eq!(after(&["-1a", "-1b", "-0a"]), frames(&["b"]));
        assert_eq!(after(&["-1a", "-0a", "-1b"]), frames(&["b"]));
    }

    #[test]
    fn a_new_main_document_forgets_the_old_frames() {
        assert_eq!(after(&["-1a", "-1b", "R0c"]), frames(&[]));
        assert_eq!(after(&["-1a", "R1c"]), frames(&["c"]));
    }

    #[test]
    fn a_report_not_from_the_script_changes_nothing() {
        for junk in ["", "-", "-1", "x1a", "-2a", "R"] {
            assert_eq!(after(&["-1a", junk]), frames(&["a"]), "{junk:?}");
        }
    }
}

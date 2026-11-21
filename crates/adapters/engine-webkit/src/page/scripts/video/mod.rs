//! The video script ([`video.js`](video.js)) and its reports: what a page's video does, in any
//! of its frames and open shadow roots, for
//! [`EngineEvent::Video`](quickie_application::EngineEvent::Video), and the calls that float
//! the video for the mini player, dock it, and play or pause it.
//!
//! [`frames`]: which frame's video is the page's. [`chain`]: floating a video in a frame, one
//! enclosing `<iframe>` at a time up to the main frame.
//!
//! Limits, all of the public API:
//! - A video in a closed shadow root is never seen. An open shadow root attached after its host
//!   joined the document is found on the next load, key press or pointer press, which is how
//!   a video with sound starts anyway.
//! - A frame of another origin is matched to its `<iframe>` by its address and its size, since
//!   `WebKit` names a frame but not its parent; two frames alike in both are told apart by
//!   order only. When no `<iframe>` matches, the video fills only its own frame, where that
//!   frame sits in the page.

mod chain;
mod frames;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::NSString;
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKScriptMessage, WKScriptMessageHandler, WKUserContentController,
    WKWebView,
};
use quickie_domain_browsing::Video;

use super::{evaluate, view_key};
use chain::{Chain, Round};
use frames::{FrameReport, Frames};

/// Installed in every frame of every page at document start.
pub(crate) const VIDEO_SCRIPT: &str = include_str!("video.js");

/// The name the video script posts under.
pub(crate) const VIDEO_HANDLER: &str = "quickieVideo";

/// Makes the frame's video fill its viewport alone; answers the frame's description.
const FLOAT_SCRIPT: &str = "quickieVideo.float()";

/// Puts the frame back as it was before it floated anything.
const DOCK_SCRIPT: &str = "quickieVideo.dock()";

/// Makes the `<iframe>` that holds the frame `description` describes fill this frame's
/// viewport alone; answers this frame's description, or nothing when it holds no such frame.
fn hold_script(description: &str) -> String {
    let literal = serde_json::Value::String(description.to_owned()).to_string();
    format!("quickieVideo.hold({literal})")
}

/// Plays the frame's video, or pauses it.
fn play_script(play: bool) -> String {
    format!("quickieVideo.play({play})")
}

/// What `WebKit` runs a frame's scripts by: its `WKFrameInfo`.
type Handle = Retained<WKFrameInfo>;

/// For each page, by [`view_key`], its frames as their video scripts reported.
pub(crate) type VideoFrames = Rc<RefCell<HashMap<usize, Frames<Handle>>>>;

/// What a page's video does now, `None` once it has none: a change of the frame that shows it
/// or of what that frame's video does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Report(pub(crate) Option<Video>);

/// Where a page's video reports go: the key of the page's view ([`view_key`]) and what it said.
pub(crate) type OnReport = Box<dyn Fn(usize, Report)>;

pub(crate) struct Ivars {
    frames: VideoFrames,
    on_report: OnReport,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `VideoHandler` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitVideoHandler"]
    #[ivars = Ivars]
    pub(crate) struct VideoHandler;

    unsafe impl NSObjectProtocol for VideoHandler {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl WKScriptMessageHandler for VideoHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: a getter; the body is whatever the script posted.
            let body = unsafe { message.body() };
            let Some(text) = body.downcast_ref::<NSString>().map(NSString::to_string) else {
                return;
            };
            let Some(report) = FrameReport::parse(&text) else {
                return;
            };
            // SAFETY: a getter; the page that posted, `None` once it is gone.
            let Some(view) = (unsafe { message.webView() }) else {
                return;
            };
            // SAFETY: a getter; the frame the script that posted runs in.
            let frame = unsafe { message.frameInfo() };
            // SAFETY: a plain getter.
            let handle = (!unsafe { frame.isMainFrame() }).then_some(frame);
            let key = view_key(&view);
            let changed = self
                .ivars()
                .frames
                .borrow_mut()
                .entry(key)
                .or_default()
                .heard(report.token, handle, report.said);
            if let Some(report) = changed {
                (self.ivars().on_report)(key, report);
            }
        }
    }
);

impl VideoHandler {
    /// A handler that keeps `frames` up to date and hands every change of a page's video to
    /// `on_report`.
    pub(crate) fn new(
        frames: VideoFrames,
        on_report: OnReport,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { frames, on_report });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// A frame to run a script in: its token and its handle, `None` for the main frame.
#[derive(Clone)]
struct Target {
    token: String,
    handle: Option<Handle>,
}

/// Every frame of a page that was heard from, and the main frame even if it was not.
fn targets(frames: Option<&Frames<Handle>>) -> Vec<Target> {
    let mut targets: Vec<Target> = frames
        .map_or(&[][..], Frames::all)
        .iter()
        .map(|frame| Target {
            token: frame.token.clone(),
            handle: frame.handle.clone(),
        })
        .collect();
    if targets.iter().all(|target| target.handle.is_some()) {
        targets.push(Target {
            token: String::new(),
            handle: None,
        });
    }
    targets
}

/// A video being floated from a frame: its chain, sent one round at a time while the float
/// that started it is the page's latest.
struct Floating {
    view: Weak<WKWebView>,
    world: Retained<WKContentWorld>,
    targets: Vec<Target>,
    chain: RefCell<Chain>,
    /// The page's count of floats and docks, and its value when this float started.
    floats: Rc<Cell<u64>>,
    started: u64,
}

impl Floating {
    fn send(self: &Rc<Self>, round: Option<Round>) {
        let Some(round) = round else { return };
        if self.floats.get() != self.started {
            return;
        }
        let Some(view) = self.view.load() else { return };
        let script = hold_script(&round.description);
        for token in round.tokens {
            let Some(target) = self.targets.iter().find(|target| target.token == token) else {
                continue;
            };
            let this = Rc::clone(self);
            let number = round.number;
            evaluate(
                &view,
                &self.world,
                target.handle.as_deref(),
                &script,
                move |answer| {
                    let next = this.chain.borrow_mut().answered(number, &token, &answer);
                    this.send(next);
                },
            );
        }
    }
}

/// Makes the page's video fill the view alone: the video fills its frame, and each `<iframe>`
/// on the way up fills its parent's, as far as the chain is found. `floats` counts the page's
/// floats and docks; a dock after this one stops its chain.
pub(crate) fn float(
    view: &WKWebView,
    world: &WKContentWorld,
    frames: Option<&Frames<Handle>>,
    floats: &Rc<Cell<u64>>,
) {
    let Some(showing) = frames.and_then(Frames::showing) else {
        evaluate(view, world, None, FLOAT_SCRIPT, drop);
        return;
    };
    let targets = targets(frames);
    let chain = Chain::new(
        targets
            .iter()
            .map(|target| (target.token.clone(), target.handle.is_none())),
        &showing.token,
    );
    let main = showing.handle.is_none();
    let floating = Rc::new(Floating {
        view: Weak::from_retained(&view.retain()),
        world: world.retain(),
        targets,
        chain: RefCell::new(chain),
        floats: Rc::clone(floats),
        started: floats.get(),
    });
    evaluate(
        view,
        world,
        showing.handle.as_deref(),
        FLOAT_SCRIPT,
        move |answer| {
            let round = floating.chain.borrow_mut().floated(main, &answer);
            floating.send(round);
        },
    );
}

/// Puts every frame of the page back as it was before it floated anything.
pub(crate) fn dock(view: &WKWebView, world: &WKContentWorld, frames: Option<&Frames<Handle>>) {
    for target in targets(frames) {
        evaluate(view, world, target.handle.as_deref(), DOCK_SCRIPT, drop);
    }
}

/// Plays the page's video, or pauses it, in the frame that shows it; the frame's script
/// reports the change.
pub(crate) fn play(
    view: &WKWebView,
    world: &WKContentWorld,
    frames: Option<&Frames<Handle>>,
    play: bool,
) {
    let handle = frames
        .and_then(Frames::showing)
        .and_then(|frame| frame.handle.as_deref());
    evaluate(view, world, handle, &play_script(play), drop);
}

#[cfg(test)]
mod tests {
    use super::{hold_script, play_script};

    #[test]
    fn playing_and_pausing_call_the_script() {
        assert_eq!(play_script(true), "quickieVideo.play(true)");
        assert_eq!(play_script(false), "quickieVideo.play(false)");
    }

    #[test]
    fn a_description_goes_to_the_script_as_one_string() {
        assert_eq!(
            hold_script(r#"{"href":"https://a.test/it's\"</script>"}"#),
            r#"quickieVideo.hold("{\"href\":\"https://a.test/it's\\\"</script>\"}")"#
        );
        assert_eq!(
            hold_script("line\u{2028}\n"),
            "quickieVideo.hold(\"line\u{2028}\\n\")"
        );
    }
}

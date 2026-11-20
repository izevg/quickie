//! The video script ([`video.js`](video.js)) and its reports: what a page's video does, for
//! [`EngineEvent::Video`](quickie_application::EngineEvent::Video), and the calls that float
//! the video for the mini player, dock it, and play or pause it.

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSString;
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};
use quickie_domain_browsing::Video;

use super::view_key;

/// Installed in every page's main frame at document start.
pub(crate) const VIDEO_SCRIPT: &str = include_str!("video.js");

/// The name the video script posts under.
pub(crate) const VIDEO_HANDLER: &str = "quickieVideo";

/// Makes the page's video fill the viewport alone.
pub(crate) const FLOAT_SCRIPT: &str = "quickieVideo.float()";

/// Puts the page back as it was before [`FLOAT_SCRIPT`].
pub(crate) const DOCK_SCRIPT: &str = "quickieVideo.dock()";

/// Plays the page's video, or pauses it.
pub(crate) fn play_script(play: bool) -> String {
    format!("quickieVideo.play({play})")
}

/// What a report of the video script says: `Some` with the video, `None` for a report that is
/// not the script's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Report(pub(crate) Option<Video>);

impl Report {
    /// Reads one report: `A`, `M`, `P`, or `N`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let video = match text {
            "A" => Some(Video::Playing { audible: true }),
            "M" => Some(Video::Playing { audible: false }),
            "P" => Some(Video::Paused),
            "N" => None,
            _ => return None,
        };
        Some(Self(video))
    }
}

/// Where a page's video reports go: the key of the page's view ([`view_key`]) and what it said.
pub(crate) type OnReport = Box<dyn Fn(usize, Report)>;

pub(crate) struct Ivars {
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
            let Some(report) = body
                .downcast_ref::<NSString>()
                .and_then(|text| Report::parse(&text.to_string()))
            else {
                return;
            };
            // SAFETY: a getter; the page that posted, `None` once it is gone.
            if let Some(view) = unsafe { message.webView() } {
                (self.ivars().on_report)(view_key(&view), report);
            }
        }
    }
);

impl VideoHandler {
    /// A handler that hands every page's reports to `on_report`.
    pub(crate) fn new(on_report: OnReport, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { on_report });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::Video;

    use super::{Report, play_script};

    #[test]
    fn each_report_names_what_the_video_does() {
        assert_eq!(
            Report::parse("A"),
            Some(Report(Some(Video::Playing { audible: true })))
        );
        assert_eq!(
            Report::parse("M"),
            Some(Report(Some(Video::Playing { audible: false })))
        );
        assert_eq!(Report::parse("P"), Some(Report(Some(Video::Paused))));
        assert_eq!(Report::parse("N"), Some(Report(None)));
    }

    #[test]
    fn a_report_not_from_the_script_is_dropped() {
        for junk in ["", "a", "AP", "X", " N"] {
            assert_eq!(Report::parse(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn playing_and_pausing_call_the_script() {
        assert_eq!(play_script(true), "quickieVideo.play(true)");
        assert_eq!(play_script(false), "quickieVideo.play(false)");
    }
}

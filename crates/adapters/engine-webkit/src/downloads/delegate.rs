//! The delegate of every `WKDownload`: it hands `WebKit`'s questions and news to the download
//! center as [`DelegateEvent`]s.

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{NSData, NSError, NSString, NSURL, NSURLResponse};
use objc2_web_kit::{WKDownload, WKDownloadDelegate};

/// What `WebKit` tells or asks about a download. Delivered on the main thread.
pub(crate) enum DelegateEvent {
    /// Where should the file go? `decide` gets the file URL, or null to cancel; it may be
    /// called later, from the main thread.
    Destination {
        download: Retained<WKDownload>,
        /// The address the bytes come from, after redirects.
        url: Option<String>,
        /// The file name the server or the page suggested.
        suggested: String,
        decide: RcBlock<dyn Fn(*mut NSURL)>,
    },
    /// Every byte is in the file.
    Finished(Retained<WKDownload>),
    /// The download stopped, with `WebKit`'s reason.
    Failed(Retained<WKDownload>, String),
}

pub(crate) struct Ivars {
    on_event: Box<dyn Fn(DelegateEvent)>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `DownloadDelegate` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitDownloadDelegate"]
    #[ivars = Ivars]
    pub(crate) struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}

    // SAFETY: each method matches the protocol's signature.
    unsafe impl WKDownloadDelegate for DownloadDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide_destination(
            &self,
            download: &WKDownload,
            response: &NSURLResponse,
            suggested: &NSString,
            decide: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let url = response
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string());
            (self.ivars().on_event)(DelegateEvent::Destination {
                download: download.retain(),
                url,
                suggested: suggested.to_string(),
                decide: decide.copy(),
            });
        }

        #[unsafe(method(downloadDidFinish:))]
        fn did_finish(&self, download: &WKDownload) {
            (self.ivars().on_event)(DelegateEvent::Finished(download.retain()));
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn did_fail(&self, download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            let reason = error.localizedDescription().to_string();
            (self.ivars().on_event)(DelegateEvent::Failed(download.retain(), reason));
        }
    }
);

impl DownloadDelegate {
    pub(crate) fn new(
        on_event: Box<dyn Fn(DelegateEvent)>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { on_event });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

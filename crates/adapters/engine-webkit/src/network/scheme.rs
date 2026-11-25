//! Built-in pages such as the start page, served from memory under their own URL scheme, so
//! they reload and go back and forward like any page and never touch the network.

use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_foundation::{NSData, NSError, NSString, NSURLResponse};
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView};
use url::Url;

/// Error domain of a built-in page that does not exist.
const NOT_FOUND_DOMAIN: &str = "QuickieInternalPage";

pub(crate) struct Ivars {
    /// Page HTML by its normalized URL.
    pages: HashMap<String, String>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `InternalPages` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitInternalPages"]
    #[ivars = Ivars]
    pub(crate) struct InternalPages;

    unsafe impl NSObjectProtocol for InternalPages {}

    // SAFETY: both methods match the protocol's signatures.
    unsafe impl WKURLSchemeHandler for InternalPages {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.serve(task);
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _view: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // Every task finishes inside `start`; there is nothing left to stop.
        }
    }
);

impl InternalPages {
    /// Serves each `(url, html)`; every URL must use a custom scheme.
    pub(crate) fn new(pages: &[(Url, String)], mtm: MainThreadMarker) -> Retained<Self> {
        let pages = pages
            .iter()
            .map(|(url, html)| (url.as_str().to_owned(), html.clone()))
            .collect();
        let this = Self::alloc(mtm).set_ivars(Ivars { pages });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// The schemes to register this handler for.
    pub(crate) fn schemes(&self) -> Vec<String> {
        let mut schemes: Vec<String> = self
            .ivars()
            .pages
            .keys()
            .filter_map(|url| Url::parse(url).ok())
            .map(|url| url.scheme().to_owned())
            .collect();
        schemes.sort();
        schemes.dedup();
        schemes
    }

    fn serve(&self, task: &ProtocolObject<dyn WKURLSchemeTask>) {
        // SAFETY: a task always carries its request.
        let request = unsafe { task.request() };
        let Some(ns_url) = request.URL() else {
            return fail(task);
        };
        let html = ns_url
            .absoluteString()
            .and_then(|url| Url::parse(&url.to_string()).ok())
            .and_then(|url| self.ivars().pages.get(url.as_str()));
        let Some(html) = html else {
            return fail(task);
        };
        let length = isize::try_from(html.len()).unwrap_or(isize::MAX);
        let response = NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
            NSURLResponse::alloc(),
            &ns_url,
            Some(&NSString::from_str("text/html")),
            length,
            Some(&NSString::from_str("utf-8")),
        );
        // SAFETY: the task is live until `didFinish`, and these are called once each, in order.
        unsafe {
            task.didReceiveResponse(&response);
        }
        // SAFETY: as above, after the response.
        unsafe { task.didReceiveData(&NSData::with_bytes(html.as_bytes())) };
        // SAFETY: as above, last.
        unsafe { task.didFinish() };
    }
}

fn fail(task: &ProtocolObject<dyn WKURLSchemeTask>) {
    // SAFETY: a domain and a code are all an error needs.
    let error = unsafe {
        NSError::errorWithDomain_code_userInfo(&NSString::from_str(NOT_FOUND_DOMAIN), 404, None)
    };
    // SAFETY: the task is live and has not finished.
    unsafe { task.didFailWithError(&error) };
}

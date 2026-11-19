//! Key-value observation of a `WKWebView`: turns changes of `title`, `URL`, and
//! `estimatedProgress` into [`PageEvent`]s.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{
    NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSString,
};
use objc2_web_kit::WKWebView;

use crate::PageEvent;

/// Properties observed on every page, by their key paths.
pub(crate) const KEY_PATHS: [&str; 3] = ["title", "URL", "estimatedProgress"];

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
            if let Some(event) = event_for(&key_path.to_string(), view) {
                (self.ivars().on_event)(event);
            }
        }
    }
);

impl PageObserver {
    pub(crate) fn new(on_event: Box<dyn Fn(PageEvent)>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { on_event });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// Starts observing `view`; [`PageObserver::detach`] must run before either is released.
    pub(crate) fn attach(&self, view: &WKWebView) {
        for key in KEY_PATHS {
            // SAFETY: the observer is an `NSObject` that implements
            // `observeValueForKeyPath:ofObject:change:context:`, and the null context is valid.
            unsafe {
                view.addObserver_forKeyPath_options_context(
                    self,
                    &NSString::from_str(key),
                    NSKeyValueObservingOptions::New,
                    std::ptr::null_mut(),
                );
            }
        }
    }

    pub(crate) fn detach(&self, view: &WKWebView) {
        for key in KEY_PATHS {
            // SAFETY: `attach` registered this observer for exactly these key paths with a null
            // context, and each one is removed once.
            unsafe {
                view.removeObserver_forKeyPath_context(
                    self,
                    &NSString::from_str(key),
                    std::ptr::null_mut(),
                );
            }
        }
    }
}

fn event_for(key_path: &str, view: &WKWebView) -> Option<PageEvent> {
    match key_path {
        "title" => Some(PageEvent::TitleChanged(crate::title_of(view))),
        "URL" => Some(PageEvent::UrlChanged(crate::url_of(view))),
        "estimatedProgress" => Some(PageEvent::ProgressChanged(crate::progress_of(view))),
        _ => None,
    }
}

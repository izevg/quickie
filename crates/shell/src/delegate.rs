//! The app delegate: the URLs and files other apps ask Quickie to open, and the target of the
//! menu items that run commands.

use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSMenuItem,
};
use objc2_foundation::{NSArray, NSString, NSURL};
use url::Url;

use crate::browser::Browser;

pub struct Ivars {
    browser: Weak<Browser>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `AppDelegate` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAppDelegate"]
    #[ivars = Ivars]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        /// Links from other apps, `open -a Quickie <url>`, and files opened with Quickie.
        #[unsafe(method(application:openURLs:))]
        fn application_open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            let urls: Vec<Url> = urls
                .iter()
                .filter_map(|url| url.absoluteString())
                .filter_map(|text| Url::parse(&text.to_string()).ok())
                .collect();
            if let Some(browser) = self.ivars().browser.upgrade()
                && !browser.updater.terminating()
            {
                browser.open_urls(urls);
            }
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _application: &NSApplication) -> NSApplicationTerminateReply {
            self.ivars().browser.upgrade().map_or(
                NSApplicationTerminateReply::TerminateNow,
                |browser| browser.prepare_termination(),
            )
        }
    }

    impl AppDelegate {
        #[unsafe(method(checkForAppUpdates:))]
        fn check_for_app_updates(&self, _sender: &NSMenuItem) {
            if let Some(browser) = self.ivars().browser.upgrade() {
                browser.updater.check(true);
            }
        }

        /// A menu item whose represented object is a command id.
        #[unsafe(method(runCommand:))]
        fn run_command(&self, sender: &NSMenuItem) {
            let Some(id) = sender
                .representedObject()
                .and_then(|object| object.downcast::<NSString>().ok())
            else {
                return;
            };
            if let Some(browser) = self.ivars().browser.upgrade() {
                browser.run_command(&id.to_string());
            }
        }
    }
);

impl AppDelegate {
    pub fn new(browser: &Rc<Browser>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            browser: Rc::downgrade(browser),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

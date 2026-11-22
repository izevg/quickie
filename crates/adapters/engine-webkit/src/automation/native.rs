#![expect(
    deprecated,
    reason = "the fork uses WKProcessPool and honors the pool on an automation configuration"
)]

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_foundation::{NSError, NSRect, NSString};
use objc2_web_kit::{
    WKNavigation, WKNavigationDelegate, WKProcessPool, WKWebView, WKWebViewConfiguration,
    WKWebsiteDataStore,
};

use super::{AutomationError, Reply};

static SESSION_COUNT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Session {
    view: Retained<WKWebView>,
    pool: Retained<WKProcessPool>,
    automation: Retained<AnyObject>,
    observer: Retained<NavigationObserver>,
    connected: bool,
}

impl Session {
    pub(super) fn new(
        mtm: MainThreadMarker,
        reply: Rc<RefCell<Reply>>,
    ) -> Result<Self, AutomationError> {
        let session = make_session()?;
        let observer = NavigationObserver::new(mtm);
        let id = SESSION_COUNT.fetch_add(1, Ordering::Relaxed);
        let name = NSString::from_str(&format!("quickie-{}-{id}", std::process::id()));
        // SAFETY: the fork session accepts an NSString and copies it.
        unsafe { msg_send![&*session, setSessionIdentifier: &*name] }
        // All methods of the automation delegate are optional. Installing an object that
        // implements only NSObject's methods gives the native client its default behavior for
        // dialog checks during navigation and timeouts.
        // SAFETY: the fork installs a weak reference, and only calls optional methods after
        // checking for their selectors. `Session` holds the observer until after disconnect.
        unsafe { msg_send![&*session, setDelegate: &*observer] }

        let handler = RcBlock::new(move |message: NonNull<NSString>| {
            // SAFETY: the fork passes a live NSString for this main-thread callback.
            let message = unsafe { message.as_ref() };
            reply.borrow_mut().receive(&message.to_string());
        });
        // SAFETY: the selector and `void (^)(NSString *)` signature match the fork API. It
        // copies the block and invokes it on the main thread.
        let connected: Bool = unsafe { msg_send![&*session, connectLocalClient: &*handler] };
        if !connected.as_bool() {
            return Err(AutomationError::Transport);
        }

        // SAFETY: a new process pool on the main thread, dedicated to this session.
        let pool = unsafe { WKProcessPool::new(mtm) };
        // SAFETY: the selector matches the fork API and the local client is already connected.
        unsafe { msg_send![&*pool, _setAutomationSession: &*session] }

        let view = make_view(&pool, mtm);
        // SAFETY: the view holds a weak navigation delegate owned by this session.
        unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*observer))) };
        // Start the content process that owns the initial browsing context without a request.
        // SAFETY: the HTML string is valid and has no base URL or external resources.
        unsafe { view.loadHTMLString_baseURL(&NSString::from_str("<!doctype html>"), None) };

        Ok(Self {
            view,
            pool,
            automation: session,
            observer,
            connected: true,
        })
    }

    pub(super) fn dispatch(&self, message: &str) -> Result<(), AutomationError> {
        if !self.connected {
            return Err(AutomationError::Transport);
        }
        let message = NSString::from_str(message);
        // SAFETY: the connected fork session accepts a JSON NSString on the main thread.
        let accepted: Bool = unsafe { msg_send![&*self.automation, dispatchMessage: &*message] };
        if accepted.as_bool() {
            Ok(())
        } else {
            Err(AutomationError::Transport)
        }
    }

    pub(super) fn is_loading(&self) -> bool {
        // SAFETY: a getter on the session's main-thread view.
        unsafe { self.view.isLoading() }
    }

    pub(super) fn reset_navigation_failure(&self) {
        self.observer.ivars().set(false);
    }

    pub(super) fn navigation_failed(&self) -> bool {
        self.observer.ivars().get()
    }

    pub(super) fn stop_loading(&self) {
        // SAFETY: the private view is live and this session owns its navigation.
        unsafe { self.view.stopLoading() }
    }

    pub(super) fn close(&mut self) {
        if !self.connected {
            return;
        }
        self.connected = false;
        // SAFETY: the local connection was installed on this main-thread session.
        unsafe { msg_send![&*self.automation, disconnectLocalClient] }
        // SAFETY: the private pool accepts nil to release the disconnected session.
        unsafe { msg_send![&*self.pool, _setAutomationSession: None::<&AnyObject>] }
        self.stop_loading();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

fn make_session() -> Result<Retained<AnyObject>, AutomationError> {
    let session = AnyClass::get(c"_WKAutomationSession").ok_or(AutomationError::Unsupported)?;
    let configuration =
        AnyClass::get(c"_WKAutomationSessionConfiguration").ok_or(AutomationError::Unsupported)?;
    if [
        sel!(initWithConfiguration:),
        sel!(setSessionIdentifier:),
        sel!(setDelegate:),
        sel!(connectLocalClient:),
        sel!(dispatchMessage:),
        sel!(disconnectLocalClient),
    ]
    .into_iter()
    .any(|selector| session.instance_method(selector).is_none())
        || [
            sel!(setInspectorPassthroughEnabled:),
            sel!(setAllowsInsecureMediaCapture:),
        ]
        .into_iter()
        .any(|selector| configuration.instance_method(selector).is_none())
        || WKProcessPool::class()
            .instance_method(sel!(_setAutomationSession:))
            .is_none()
        || WKWebViewConfiguration::class()
            .instance_method(sel!(_setControlledByAutomation:))
            .is_none()
    {
        return Err(AutomationError::Unsupported);
    }

    // SAFETY: the configuration class is a present NSObject subclass with a normal initializer.
    let config: Option<Retained<AnyObject>> = unsafe { msg_send![configuration, new] };
    let config = config.ok_or(AutomationError::Transport)?;
    // SAFETY: both setters accept BOOL and are available on this configuration class.
    unsafe { msg_send![&*config, setInspectorPassthroughEnabled: Bool::NO] }
    // SAFETY: as above; keep the standard secure-context requirements for media capture.
    unsafe { msg_send![&*config, setAllowsInsecureMediaCapture: Bool::NO] }
    // SAFETY: this is the loaded Objective-C session class; allocation does not run its init.
    let allocated: Allocated<AnyObject> = unsafe { msg_send![session, alloc] };
    // SAFETY: the live configuration is the type required by the fork session initializer.
    let session: Option<Retained<AnyObject>> =
        unsafe { msg_send![allocated, initWithConfiguration: &*config] };
    session.ok_or(AutomationError::Transport)
}

fn make_view(pool: &WKProcessPool, mtm: MainThreadMarker) -> Retained<WKWebView> {
    // SAFETY: a fresh configuration on the main thread has no preconditions.
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    // SAFETY: the pool belongs solely to this session and has its local client attached.
    unsafe { configuration.setProcessPool(pool) };
    // SAFETY: a fresh nonpersistent store, allocated on the main thread.
    let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
    // SAFETY: WebKit retains the store in the view's configuration.
    unsafe { configuration.setWebsiteDataStore(&store) };
    // SAFETY: preferences always exist on a fresh configuration.
    let preferences = unsafe { configuration.preferences() };
    // SAFETY: a boolean setter. Avoid third-party lookups, as in the browser's normal views.
    unsafe { preferences.setFraudulentWebsiteWarningEnabled(false) };
    // SAFETY: support was checked for the fork's boolean setter.
    unsafe { msg_send![&*configuration, _setControlledByAutomation: Bool::YES] }
    // SAFETY: the allocated view and its isolated configuration are on the main thread.
    unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::ZERO, &configuration)
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the observer implements no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitAutomationNavigationObserver"]
    #[ivars = Cell<bool>]
    struct NavigationObserver;

    unsafe impl NSObjectProtocol for NavigationObserver {}

    // SAFETY: both optional methods match the WebKit navigation delegate protocol.
    unsafe impl WKNavigationDelegate for NavigationObserver {
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_failed(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>, _error: &NSError) {
            self.ivars().set(true);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>, _error: &NSError) {
            self.ivars().set(true);
        }
    }
);

impl NavigationObserver {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let observer = Self::alloc(mtm).set_ivars(Cell::new(false));
        // SAFETY: ivars are initialized and NSObject's init has no further requirements.
        unsafe { msg_send![super(observer), init] }
    }
}

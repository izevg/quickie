//! A page's `WKUIDelegate`: turns what the page asks into calls on its [`Hooks`].
//! Geolocation has no delegate method on macOS, so `WebKit` keeps refusing it.

use std::cell::RefCell;
use std::rc::Rc;

use block2::DynBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSView;
use objc2_foundation::NSString;
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKNavigationAction, WKPermissionDecision, WKSecurityOrigin,
    WKUIDelegate, WKWebView, WKWebViewConfiguration, WKWindowFeatures,
};
use quickie_application::{AskKind, Dialog, Permission};
use quickie_domain_browsing::Focus;
use url::Url;

use super::security_key::{self, Authentication};
use super::{Reply, link_focus};
use crate::page::{Hooks, PageEvent};

/// Where a page without a URL of its own is: never an empty address bar.
pub(crate) const BLANK: &str = "about:blank";

pub(crate) struct Ivars {
    hooks: Rc<Hooks>,
    frame: Weak<NSView>,
    authentication: RefCell<Option<Retained<Authentication>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `PageUi` implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitPageUi"]
    #[ivars = Ivars]
    pub(crate) struct PageUi;

    unsafe impl NSObjectProtocol for PageUi {}

    impl PageUi {
        #[unsafe(method(_webView:hasVideoInPictureInPictureDidChange:))]
        fn picture_in_picture(&self, _view: &WKWebView, active: Bool) {
            let active = active.as_bool();
            let hooks = &self.ivars().hooks;
            hooks.native_pip_active.set(active);
            (hooks.on_event)(PageEvent::PictureInPicture(active));
        }

        #[unsafe(method(_webViewFullscreenMayReturnToInline:))]
        fn fullscreen_may_return_to_inline(&self, _view: &WKWebView) {
            let hooks = &self.ivars().hooks;
            if hooks.native_pip_active.get() {
                (hooks.on_event)(PageEvent::PictureInPictureReturnRequested);
            }
        }

        #[unsafe(method(_webView:runWebAuthenticationPanel:initiatedByFrame:completionHandler:))]
        fn authenticate(
            &self,
            view: &WKWebView,
            panel: &AnyObject,
            initiating_frame: &WKFrameInfo,
            reply: &DynBlock<dyn Fn(isize)>,
        ) {
            let Some(presenter) = self.ivars().hooks.security_key_presenter.as_ref()
                .filter(|_| security_key::is_usb_sign_in(panel))
            else {
                reply.call((security_key::UNAVAILABLE,));
                return;
            };
            self.cancel_authentication();
            let session = self.ivars().frame.load().and_then(|frame| {
                let origin = frame_origin(initiating_frame)?.origin().ascii_serialization();
                Authentication::begin(view, &frame, panel, origin, Rc::clone(presenter), self.mtm())
            });
            if let Some(session) = session {
                self.ivars().authentication.replace(Some(session));
                reply.call((security_key::PRESENTED,));
            } else {
                reply.call((security_key::DID_NOT_PRESENT,));
                security_key::cancel(panel);
            }
        }
    }

    // SAFETY: every method matches the protocol's signature; the rest are optional.
    unsafe impl WKUIDelegate for PageUi {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create(
            &self,
            _view: &WKWebView,
            configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            // SAFETY: a getter with no preconditions.
            let modifiers = unsafe { action.modifierFlags() };
            // SAFETY: as above.
            let button = unsafe { action.buttonNumber() };
            let focus = link_focus(modifiers, button).unwrap_or(Focus::Foreground);
            // SAFETY: a getter; every action has a request.
            let request = unsafe { action.request() };
            let url = request
                .URL()
                .and_then(|url| url.absoluteString())
                .and_then(|url| Url::parse(&url.to_string()).ok())
                .or_else(blank);
            url.and_then(|url| (self.ivars().hooks.open)(configuration, url, focus))
        }

        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, _view: &WKWebView) {
            (self.ivars().hooks.on_event)(PageEvent::Closed);
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(
            &self,
            _view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            reply: &DynBlock<dyn Fn()>,
        ) {
            let dialog = Dialog::Alert {
                message: message.to_string(),
            };
            self.ask(frame_origin(frame), dialog, Reply::alert(reply.copy()));
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(
            &self,
            _view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            reply: &DynBlock<dyn Fn(Bool)>,
        ) {
            let dialog = Dialog::Confirm {
                message: message.to_string(),
            };
            self.ask(frame_origin(frame), dialog, Reply::confirm(reply.copy()));
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(
            &self,
            _view: &WKWebView,
            message: &NSString,
            default: Option<&NSString>,
            frame: &WKFrameInfo,
            reply: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let dialog = Dialog::Prompt {
                message: message.to_string(),
                default: default.map_or_else(String::new, NSString::to_string),
            };
            self.ask(frame_origin(frame), dialog, Reply::prompt(reply.copy()));
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_capture(
            &self,
            _view: &WKWebView,
            origin: &WKSecurityOrigin,
            frame: &WKFrameInfo,
            kind: WKMediaCaptureType,
            reply: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let permission = match kind {
                WKMediaCaptureType::Camera => Permission::Camera,
                WKMediaCaptureType::Microphone => Permission::Microphone,
                _ => Permission::CameraAndMicrophone,
            };
            let reply = Reply::permission(reply.copy());
            // An origin that cannot be named is declined: the sheet must say who asks.
            if let Some(origin) = origin_url(origin).or_else(|| frame_origin(frame)) {
                (self.ivars().hooks.on_ask)(origin, AskKind::Permission(permission), reply);
            }
        }
    }
);

impl PageUi {
    pub(crate) fn new(
        hooks: Rc<Hooks>,
        frame: &Retained<NSView>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            hooks,
            frame: Weak::from_retained(frame),
            authentication: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    pub(crate) fn cancel_authentication(&self) {
        let session = self.ivars().authentication.borrow_mut().take();
        if let Some(session) = session {
            session.cancel();
        }
    }

    pub(crate) fn native_pip_active(&self) -> bool {
        self.ivars().hooks.native_pip_active.get()
    }

    /// Asks the user, on behalf of the frame's origin; a dialog whose origin cannot be named
    /// is cancelled.
    fn ask(&self, origin: Option<Url>, dialog: Dialog, reply: Reply) {
        if let Some(origin) = origin {
            (self.ivars().hooks.on_ask)(origin, AskKind::Dialog(dialog), reply);
        }
    }
}

fn blank() -> Option<Url> {
    Url::parse(BLANK).ok()
}

/// The origin of the frame that asks, as a URL: who the sheet says is asking. A frame without
/// a host (`about:blank`, `data:`) is `about:blank`.
fn frame_origin(frame: &WKFrameInfo) -> Option<Url> {
    // SAFETY: a getter; every frame has an origin.
    let origin = unsafe { frame.securityOrigin() };
    origin_url(&origin).or_else(blank)
}

/// `https://example.com:8443/` for an origin; `None` for one without a host (`about:blank`,
/// `data:`).
fn origin_url(origin: &WKSecurityOrigin) -> Option<Url> {
    // SAFETY: a getter with no preconditions.
    let scheme = unsafe { origin.protocol() };
    // SAFETY: as above.
    let host = unsafe { origin.host() };
    // SAFETY: as above.
    let port = unsafe { origin.port() };
    origin_text(&scheme.to_string(), &host.to_string(), port)
        .and_then(|text| Url::parse(&text).ok())
}

/// The URL text of an origin's parts; `None` without a scheme or a host. A port of 0 is the
/// scheme's default; an IPv6 host gets its brackets.
fn origin_text(scheme: &str, host: &str, port: isize) -> Option<String> {
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let port = if port > 0 {
        format!(":{port}")
    } else {
        String::new()
    };
    Some(format!("{scheme}://{host}{port}/"))
}

#[cfg(test)]
mod tests {
    use super::origin_text;

    #[test]
    fn an_origin_is_written_as_a_url() {
        assert_eq!(
            origin_text("https", "meet.example", 0).as_deref(),
            Some("https://meet.example/")
        );
        assert_eq!(
            origin_text("http", "127.0.0.1", 8080).as_deref(),
            Some("http://127.0.0.1:8080/")
        );
        assert_eq!(
            origin_text("http", "::1", 80).as_deref(),
            Some("http://[::1]:80/")
        );
        assert_eq!(
            origin_text("http", "[::1]", 0).as_deref(),
            Some("http://[::1]/")
        );
    }

    #[test]
    fn an_origin_without_a_scheme_or_host_has_no_url() {
        assert_eq!(origin_text("", "", 0), None);
        assert_eq!(origin_text("about", "", 0), None);
    }
}

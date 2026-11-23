//! USB `WebAuthn` presentation for the fork's own authenticator manager.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::DynBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::{NSNumber, NSSet, NSString};
use objc2_web_kit::WKWebView;

mod reply;

use reply::PinReply;

pub(super) const UNAVAILABLE: isize = 0;
pub(super) const PRESENTED: isize = 1;
pub(super) const DID_NOT_PRESENT: isize = 2;
const USB: isize = 0;
const GET: isize = 1;
const PIN_INVALID: isize = 4;

/// What the key needs next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityKeyStage {
    Touch,
    Pin { remaining: usize, incorrect: bool },
}

/// Information that can be shown to the user. The PIN never travels through this type.
pub struct SecurityKeyPrompt {
    pub origin: String,
    pub relying_party: String,
    pub stage: SecurityKeyStage,
}

/// A submitted PIN or a cancellation. The callback is called at most once, on the main thread.
pub type SecurityKeyCompletion = Box<dyn FnOnce(Option<Retained<NSString>>)>;
/// Removes a superseded prompt and discards its completion without calling it.
pub type DismissSecurityKeyPrompt = Box<dyn FnOnce()>;
/// The host presents a browser-owned prompt on the supplied window.
pub type SecurityKeyPresenter =
    dyn Fn(&NSWindow, &SecurityKeyPrompt, SecurityKeyCompletion) -> DismissSecurityKeyPrompt;

struct Shown {
    identity: Rc<()>,
    dismiss: Option<DismissSecurityKeyPrompt>,
}

pub(super) struct Ivars {
    view: Weak<WKWebView>,
    frame: Weak<NSView>,
    panel: Weak<AnyObject>,
    parent: Retained<NSWindow>,
    presenter: Rc<SecurityKeyPresenter>,
    prompt: RefCell<Option<Shown>>,
    done: Cell<bool>,
    incorrect: Cell<bool>,
    origin: String,
    relying_party: String,
}

define_class!(
    // SAFETY: the authentication panel holds a weak delegate; its optional selectors match
    // the pinned _WKWebAuthenticationPanelDelegate ABI.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieUsbAuthentication"]
    #[ivars = Ivars]
    pub(super) struct Authentication;

    unsafe impl NSObjectProtocol for Authentication {}

    impl Authentication {
        #[unsafe(method(panel:requestPINWithRemainingRetries:completionHandler:))]
        fn request_pin(
            &self,
            panel: &AnyObject,
            remaining: usize,
            reply: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let reply = PinReply::new(reply.copy());
            if self.owns(panel) {
                let incorrect = self.ivars().incorrect.replace(false);
                self.show(SecurityKeyStage::Pin { remaining, incorrect }, Some(reply));
            }
        }

        #[unsafe(method(panel:updateWebAuthenticationPanel:))]
        fn updated(&self, panel: &AnyObject, status: isize) {
            if self.owns(panel) && status == PIN_INVALID {
                self.ivars().incorrect.set(true);
            }
        }

        #[unsafe(method(panel:dismissWebAuthenticationPanelWithResult:))]
        fn dismissed(&self, panel: &AnyObject, _result: isize) {
            if self.owns(panel) {
                self.ivars().done.set(true);
                self.hide();
            }
        }
    }
);

impl Authentication {
    pub(super) fn begin(
        view: &WKWebView,
        frame: &NSView,
        panel: &AnyObject,
        origin: String,
        presenter: Rc<SecurityKeyPresenter>,
        mtm: MainThreadMarker,
    ) -> Option<Retained<Self>> {
        let parent = owner(view, frame)?;
        if origin == "null" || !parent.isKeyWindow() || parent.attachedSheet().is_some() {
            return None;
        }
        // SAFETY: a getter on the _WKWebAuthenticationPanel supplied by WebKit.
        let relying_party: Retained<NSString> = unsafe { msg_send![panel, relyingPartyID] };
        let this = Self::alloc(mtm).set_ivars(Ivars {
            view: Weak::from_retained(&view.retain()),
            frame: Weak::from_retained(&frame.retain()),
            panel: Weak::from_retained(&panel.retain()),
            parent,
            presenter,
            prompt: RefCell::new(None),
            done: Cell::new(false),
            incorrect: Cell::new(false),
            origin,
            relying_party: relying_party.to_string(),
        });
        // SAFETY: NSObject's designated initializer with initialized ivars.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        // SAFETY: this delegate implements the optional callback ABI and the page retains it.
        let (): () = unsafe { msg_send![panel, setDelegate: &*this] };
        this.show(SecurityKeyStage::Touch, None);
        (!this.ivars().done.get()).then_some(this)
    }

    pub(super) fn cancel(&self) {
        if self.ivars().done.replace(true) {
            return;
        }
        if let Some(panel) = self.ivars().panel.load() {
            cancel(&panel);
        }
        self.hide();
    }

    fn owns(&self, panel: &AnyObject) -> bool {
        !self.ivars().done.get()
            && self
                .ivars()
                .panel
                .load()
                .as_deref()
                .is_some_and(|own| std::ptr::eq(own, panel))
    }

    fn still_visible(&self) -> bool {
        let ivars = self.ivars();
        let (Some(view), Some(frame)) = (ivars.view.load(), ivars.frame.load()) else {
            return false;
        };
        owner(&view, &frame).as_deref() == Some(&*ivars.parent)
    }

    fn hide(&self) {
        let prompt = self.ivars().prompt.borrow_mut().take();
        if let Some(dismiss) = prompt.and_then(|prompt| prompt.dismiss) {
            dismiss();
        }
    }

    fn show(&self, stage: SecurityKeyStage, reply: Option<PinReply>) {
        if !self.still_visible() {
            self.cancel();
            return;
        }
        self.hide();
        let ivars = self.ivars();
        let prompt = SecurityKeyPrompt {
            origin: ivars.origin.clone(),
            relying_party: ivars.relying_party.clone(),
            stage,
        };
        let identity = Rc::new(());
        let expected = Rc::clone(&identity);
        let session = Weak::from_retained(&self.retain());
        ivars.prompt.replace(Some(Shown {
            identity: Rc::clone(&identity),
            dismiss: None,
        }));
        let dismiss = (ivars.presenter)(
            &ivars.parent,
            &prompt,
            Box::new(move |pin| {
                if let Some(session) = session.load() {
                    session.answered(&expected, stage, reply, pin);
                }
            }),
        );
        let mut shown = ivars.prompt.borrow_mut();
        if let Some(shown) = shown
            .as_mut()
            .filter(|shown| Rc::ptr_eq(&identity, &shown.identity))
        {
            shown.dismiss = Some(dismiss);
        } else {
            drop(shown);
            dismiss();
        }
    }

    fn answered(
        &self,
        identity: &Rc<()>,
        stage: SecurityKeyStage,
        reply: Option<PinReply>,
        pin: Option<Retained<NSString>>,
    ) {
        let shown = self
            .ivars()
            .prompt
            .borrow_mut()
            .take_if(|shown| Rc::ptr_eq(identity, &shown.identity));
        if shown.is_none() || self.ivars().done.get() {
            return;
        }
        let (Some(reply), Some(pin)) = (reply, pin) else {
            self.cancel();
            return;
        };
        if !self.still_visible() {
            self.cancel();
            return;
        }
        if pin.length() == 0 {
            self.show(stage, Some(reply));
            return;
        }
        self.show(SecurityKeyStage::Touch, None);
        if !self.ivars().done.get() {
            reply.send(&pin);
        }
    }
}

fn owner(view: &WKWebView, frame: &NSView) -> Option<Retained<NSWindow>> {
    // SAFETY: the view is live on the main thread. The normal page frame is its direct parent.
    let attached = unsafe { view.superview() }.as_deref() == Some(frame);
    if !attached || view.isHiddenOrHasHiddenAncestor() {
        return None;
    }
    view.window().filter(|window| window.isVisible())
}

pub(super) fn is_usb_sign_in(panel: &AnyObject) -> bool {
    // SAFETY: a getter on the _WKWebAuthenticationPanel supplied by WebKit.
    let kind: isize = unsafe { msg_send![panel, type] };
    // SAFETY: the panel's transports property returns a set of NSNumber values.
    let transports: Retained<NSSet<NSNumber>> = unsafe { msg_send![panel, transports] };
    kind == GET && transports.containsObject(&NSNumber::new_isize(USB))
}

pub(super) fn cancel(panel: &AnyObject) {
    // SAFETY: WebKit's panel cancels only the request it represents.
    let (): () = unsafe { msg_send![panel, cancel] };
}

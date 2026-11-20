//! The query field's delegate reports each edit, so an answer follows every keystroke.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSControlTextEditingDelegate, NSTextFieldDelegate};
use objc2_foundation::NSNotification;

/// What an edit runs.
type Changed = Rc<dyn Fn()>;

pub struct Ivars {
    changed: RefCell<Option<Changed>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `FieldDelegate` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieQueryFieldDelegate"]
    #[ivars = Ivars]
    pub struct FieldDelegate;

    unsafe impl NSObjectProtocol for FieldDelegate {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSControlTextEditingDelegate for FieldDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, _notification: &NSNotification) {
            let changed = self.ivars().changed.borrow().clone();
            if let Some(changed) = changed {
                changed();
            }
        }
    }

    unsafe impl NSTextFieldDelegate for FieldDelegate {}
);

impl FieldDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            changed: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// Runs `changed` after every edit the user makes in the field.
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        *self.ivars().changed.borrow_mut() = Some(Rc::new(changed));
    }
}

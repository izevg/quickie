//! The query field's delegate reports each edit, so an answer follows every keystroke, and a
//! `⌫` in the empty field, which the palette uses to leave the mode its prompt shows.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSControl, NSControlTextEditingDelegate, NSTextFieldDelegate, NSTextView};
use objc2_foundation::NSNotification;

/// What an edit runs; what `⌫` in the empty field runs, which answers whether it took the key.
type Changed = Rc<dyn Fn()>;
type Erased = Rc<dyn Fn() -> bool>;

pub struct Ivars {
    absorb: RefCell<Option<Changed>>,
    changed: RefCell<Option<Changed>>,
    erased: RefCell<Option<Erased>>,
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

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl NSControlTextEditingDelegate for FieldDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, _notification: &NSNotification) {
            let absorb = self.ivars().absorb.borrow().clone();
            if let Some(absorb) = absorb {
                absorb();
            }
            self.changed();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, editor: &NSTextView, command: Sel) -> Bool {
            if command != sel!(deleteBackward:) || editor.string().length() > 0 {
                return Bool::NO;
            }
            let erased = self.ivars().erased.borrow().clone();
            let took = erased.is_some_and(|erased| erased());
            if took {
                self.changed();
            }
            Bool::new(took)
        }
    }

    unsafe impl NSTextFieldDelegate for FieldDelegate {}
);

impl FieldDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            absorb: RefCell::new(None),
            changed: RefCell::new(None),
            erased: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// Runs `changed` after every edit the user makes in the field.
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        *self.ivars().changed.borrow_mut() = Some(Rc::new(changed));
    }

    /// Runs `erased` when `⌫` is pressed in the empty field; it returns true when it used the
    /// key, which then counts as an edit.
    pub fn on_erase_empty(&self, erased: impl Fn() -> bool + 'static) {
        *self.ivars().erased.borrow_mut() = Some(Rc::new(erased));
    }

    /// Runs `absorb` after every edit, before the edit is reported.
    pub fn on_absorb(&self, absorb: impl Fn() + 'static) {
        *self.ivars().absorb.borrow_mut() = Some(Rc::new(absorb));
    }

    fn changed(&self) {
        let changed = self.ivars().changed.borrow().clone();
        if let Some(changed) = changed {
            changed();
        }
    }
}

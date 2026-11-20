//! A toggle row: a label, a detail line under it, and a switch at the trailing edge
//! (`docs/design/inventory.md`, shared `toggle_row`). On, off, or disabled; the switch is
//! `AppKit`'s own, so it keeps the system's look, accent color, and motion.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSControlSize, NSControlStateValueOff, NSControlStateValueOn, NSLineBreakMode,
    NSSwitch, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::tokens::{TextRole, color, space};

type Handler = Rc<dyn Fn(bool)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ToggleTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieToggleRowTarget"]
    #[ivars = Ivars]
    pub struct ToggleTarget;

    unsafe impl NSObjectProtocol for ToggleTarget {}

    impl ToggleTarget {
        #[unsafe(method(changed:))]
        fn changed(&self, sender: Option<&NSSwitch>) {
            if let Some(switch) = sender {
                self.tell(is_on(switch));
            }
        }
    }
);

impl ToggleTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn tell(&self, on: bool) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(on);
        }
    }
}

/// A label and its detail with a switch, and an optional accessory before the switch (a
/// refresh button on a filter list).
pub struct ToggleRow {
    view: Retained<NSView>,
    title: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    switch: Retained<NSSwitch>,
    accessory: RefCell<Option<Retained<NSView>>>,
    target: Retained<ToggleTarget>,
}

impl ToggleRow {
    /// The row's height: two lines of text and the space around them.
    pub const HEIGHT: CGFloat = 44.0;

    #[must_use]
    pub fn new(title: &str, detail: &str, mtm: MainThreadMarker) -> Self {
        let view = NSView::new(mtm);
        let title = label(title, TextRole::RowTitle, &color::label(), mtm);
        let detail = label(detail, TextRole::RowDetail, &color::secondary_label(), mtm);
        let switch = NSSwitch::new(mtm);
        switch.setControlSize(NSControlSize::Small);
        switch.sizeToFit();
        let target = ToggleTarget::new(mtm);
        // SAFETY: the target is weak; the row keeps it alive for as long as the switch.
        unsafe { switch.setTarget(Some(&target)) };
        // SAFETY: the target implements `changed:`, which takes the sender.
        unsafe { switch.setAction(Some(sel!(changed:))) };
        switch.setAccessibilityLabel(Some(&title.stringValue()));
        view.addSubview(&title);
        view.addSubview(&detail);
        view.addSubview(&switch);
        Self {
            view,
            title,
            detail,
            switch,
            accessory: RefCell::new(None),
            target,
        }
    }

    /// Puts `accessory`, sized already, between the text and the switch.
    pub fn set_accessory(&self, accessory: &NSView) {
        if let Some(old) = self.accessory.replace(Some(Retained::from(accessory))) {
            old.removeFromSuperview();
        }
        self.view.addSubview(accessory);
    }

    /// Runs `handler` with the new state each time the user flips the switch.
    pub fn on_change(&self, handler: impl Fn(bool) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// Shows the state without telling the handler.
    pub fn set_on(&self, on: bool) {
        let state = if on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        self.switch.setState(state);
    }

    #[must_use]
    pub fn is_on(&self) -> bool {
        is_on(&self.switch)
    }

    /// A disabled row shows its state and takes no clicks.
    pub fn set_enabled(&self, enabled: bool) {
        self.switch.setEnabled(enabled);
        let tint = if enabled {
            color::label()
        } else {
            color::secondary_label()
        };
        self.title.setTextColor(Some(&tint));
    }

    pub fn set_detail(&self, detail: &str) {
        self.detail.setStringValue(&NSString::from_str(detail));
    }

    /// Flips the switch as a click does, telling the handler; nothing when disabled.
    pub fn toggle(&self) {
        if !self.switch.isEnabled() {
            return;
        }
        let on = !self.is_on();
        self.set_on(on);
        self.target.tell(on);
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Places the row at `origin` in its superview, `width` wide: the text on the leading
    /// side, the switch centered on the trailing edge.
    pub fn place(&self, origin: NSPoint, width: CGFloat) {
        self.view
            .setFrame(NSRect::new(origin, NSSize::new(width, Self::HEIGHT)));
        let switch = self.switch.frame().size;
        self.switch.setFrameOrigin(NSPoint::new(
            width - switch.width,
            ((Self::HEIGHT - switch.height) / 2.0).round(),
        ));
        let mut text_width = width - switch.width - space::MD;
        if let Some(accessory) = self.accessory.borrow().as_ref() {
            let size = accessory.frame().size;
            accessory.setFrameOrigin(NSPoint::new(
                text_width - size.width,
                ((Self::HEIGHT - size.height) / 2.0).round(),
            ));
            text_width -= size.width + space::SM;
        }
        let title_height = self.title.frame().size.height;
        let detail_height = self.detail.frame().size.height;
        let gap = 2.0;
        let bottom = ((Self::HEIGHT - title_height - detail_height - gap) / 2.0).round();
        self.detail.setFrame(NSRect::new(
            NSPoint::new(0.0, bottom),
            NSSize::new(text_width, detail_height),
        ));
        self.title.setFrame(NSRect::new(
            NSPoint::new(0.0, bottom + detail_height + gap),
            NSSize::new(text_width, title_height),
        ));
    }
}

fn is_on(switch: &NSSwitch) -> bool {
    switch.state() == NSControlStateValueOn
}

fn label(
    text: &str,
    role: TextRole,
    tint: &objc2_app_kit::NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

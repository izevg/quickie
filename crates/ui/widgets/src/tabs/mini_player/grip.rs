//! The grip: a clear view over the player's video that takes every press, drag, and scroll on
//! it, so the floating page never sees the pointer, and hands each press, drag, and release
//! to the player. It reacts to nothing else: no hover. To `VoiceOver` it is a button, the
//! video, whose press is a click on it.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAccessibility, NSAccessibilityButtonRole, NSEvent, NSView};
use objc2_foundation::NSPoint;

/// What the pointer did on the grip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Press,
    Drag,
    Release,
    /// `VoiceOver` pressed it: a click without a pointer.
    Activate,
}

/// Hears each phase, where the pointer is in the window, and when, in seconds.
pub(super) type OnPointer = Box<dyn Fn(Phase, NSPoint, f64)>;

pub(super) struct Ivars {
    on_pointer: RefCell<Option<OnPointer>>,
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements, and `Grip` implements no `Drop`.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieMiniPlayerGrip"]
    #[ivars = Ivars]
    pub(super) struct Grip;

    unsafe impl NSObjectProtocol for Grip {}

    impl Grip {
        /// The first click on an inactive window already grabs the player.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// A press moves the player, never the window.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.tell(Phase::Press, event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.tell(Phase::Drag, event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.tell(Phase::Release, event);
        }

        /// A scroll over the player scrolls neither it nor the page under it.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, _event: &NSEvent) {}

        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            if let Some(on_pointer) = self.ivars().on_pointer.borrow().as_ref() {
                on_pointer(Phase::Activate, NSPoint::ZERO, 0.0);
            }
            true
        }
    }
);

impl Grip {
    pub(super) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            on_pointer: RefCell::new(None),
        });
        // SAFETY: `init` is `NSView`'s initializer and the ivars are set.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.setAccessibilityElement(true);
        // SAFETY: an immutable string exported by AppKit.
        this.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        this
    }

    /// Hands every press, drag, and release to `on_pointer` from now on.
    pub(super) fn on_pointer(&self, on_pointer: OnPointer) {
        *self.ivars().on_pointer.borrow_mut() = Some(on_pointer);
    }

    fn tell(&self, phase: Phase, event: &NSEvent) {
        if let Some(on_pointer) = self.ivars().on_pointer.borrow().as_ref() {
            on_pointer(phase, event.locationInWindow(), event.timestamp());
        }
    }
}

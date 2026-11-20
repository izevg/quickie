//! The agent frame (`docs/design/mocks/agent/session/`): a 2 pt border in the agent color around
//! the window's content while the agent acts, so it is plain at a glance whose turn it is. It
//! never takes a click (the page under it works as before), draws nothing inside, and comes and
//! goes with the badge.

use std::cell::Cell;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAppearanceCustomization, NSAutoresizingMaskOptions, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::NSPoint;
use quickie_ui_shared::tokens::{color, increased_contrast};
use quickie_ui_shared::{Trigger, motion};

/// The border's width; 3 pt with Increase Contrast.
pub const WIDTH: CGFloat = 2.0;

define_class!(
    // SAFETY: `NSView` has no subclassing requirements beyond the main thread, and `FrameView`
    // implements no `Drop`; `hitTest:`, `wantsUpdateLayer`, and `updateLayer` are public API
    // meant to be overridden.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAgentFrameView"]
    struct FrameView;

    impl FrameView {
        /// Clicks go through to the page.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            None
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        /// The agent color resolves as the layer is drawn, so it follows light and dark.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            let Some(layer) = self.layer() else {
                return;
            };
            let width = if increased_contrast(self) {
                WIDTH + 1.0
            } else {
                WIDTH
            };
            let apply = RcBlock::new(move || {
                layer.setBorderColor(Some(&color::agent().CGColor()));
                layer.setBorderWidth(width);
            });
            self.effectiveAppearance()
                .performAsCurrentDrawingAppearance(&apply);
        }
    }
);

pub struct AgentFrame {
    view: Retained<FrameView>,
    shown: Cell<bool>,
}

impl AgentFrame {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        // SAFETY: `init` is `NSView`'s designated initializer.
        let view: Retained<FrameView> = unsafe { msg_send![FrameView::alloc(mtm), init] };
        view.setWantsLayer(true);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        Self {
            view,
            shown: Cell::new(false),
        }
    }

    #[must_use]
    pub fn is_shown(&self) -> bool {
        self.shown.get()
    }

    /// The frame's view, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Frames `host` (the window's content); the same host again costs nothing.
    pub fn show(&self, host: &NSView) {
        // SAFETY: a getter; the superview is retained for the comparison.
        let parent = unsafe { self.view.superview() };
        if self.shown.get() && parent.is_some_and(|parent| &*parent == host) {
            return;
        }
        self.view.removeFromSuperview();
        self.view.setFrame(host.bounds());
        host.addSubview(&self.view);
        motion::show(&self.view, motion::FADE_FAST, Trigger::App);
        self.shown.set(true);
    }

    pub fn hide(&self) {
        if !self.shown.replace(false) {
            return;
        }
        self.view.removeFromSuperview();
    }
}

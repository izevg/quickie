//! The page frame: where the page sits in its window. By default it fills the window edge to
//! edge (*Content first*). With *Inset page* on it is a card over the window's background: an
//! even [`INSET`] at the sides and the bottom, the titlebar's height at the top so the window
//! buttons sit in the band above it, rounded corners ([`RADIUS`]), a soft shadow, and a hairline
//! ring. The band around the card is the window's chrome: the shell's drag area lies under it,
//! and the card itself never moves the window.
//!
//! - The window's full screen keeps the card, with an even [`INSET`] on top as well, since the
//!   window buttons are gone there; the frame follows it on its own. A page's element full screen
//!   drops the inset, which returns with it ([`PageFrame::set_page_full_screen`]).
//! - Core Animation only: the card's layer casts the shadow along a path, and the page's host
//!   inside it clips to the rounded corners. Resizing keeps the inset exact in the same frame:
//!   the card lays itself out from the window's size each time the window resizes
//!   (`resizeWithOldSuperviewSize:`), never by a second layout pass.
//! - [`glow`]: while the page loads, a light runs along its edge: the window's inner edge, or
//!   around the card as its box shadow.

mod glow;

use std::cell::Cell;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAutoresizingMaskOptions, NSView,
    NSWindowDidExitFullScreenNotification, NSWindowOrderingMode, NSWindowStyleMask,
    NSWindowWillEnterFullScreenNotification,
};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::CGPath;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize};
use objc2_quartz_core::kCACornerCurveContinuous;
use quickie_ui_shared::tokens::{color, increased_contrast};

/// The card's distance from the window's sides and bottom, in points.
pub const INSET: CGFloat = 8.0;

/// The card's corner radius: the window's own, less the inset, so the corners run parallel.
pub const RADIUS: CGFloat = 12.0;

/// The corner radius of a window with a titlebar and no toolbar, which the glow follows edge to
/// edge. `AppKit` has no public call for it.
const WINDOW_RADIUS: CGFloat = 16.0;

/// The card's shadow: soft and close, it lifts the page off the window without a frame.
const SHADOW_OPACITY: f32 = 0.16;
const SHADOW_RADIUS: CGFloat = 6.0;
const SHADOW_OFFSET: CGFloat = 1.0;

/// What decides the card's frame, and the glow it places with it.
struct Ivars {
    /// *Inset page* is on.
    inset: Cell<bool>,
    /// The window is in full screen or entering it.
    window_full_screen: Cell<bool>,
    /// The page shows an element full screen.
    page_full_screen: Cell<bool>,
    glow: Retained<glow::GlowView>,
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements beyond the main thread, and `Card`
    // implements no `Drop`; the methods it overrides are public API meant to be overridden.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickiePageCard"]
    #[ivars = Ivars]
    struct Card;

    unsafe impl NSObjectProtocol for Card {}

    impl Card {
        /// Follows its window in and out of full screen. The center drops a selector observer
        /// when it is deallocated, so a closed window's card leaves nothing behind.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            // SAFETY: `viewDidMoveToWindow` is `NSView`'s, called by AppKit on the main thread.
            let () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            let center = NSNotificationCenter::defaultCenter();
            // SAFETY: `self` is a live object; removing an observer that is not there is a no-op.
            unsafe { center.removeObserver(self) };
            let Some(window) = self.window() else {
                return;
            };
            // SAFETY: an immutable AppKit constant, read after AppKit has loaded.
            let will_enter = unsafe { NSWindowWillEnterFullScreenNotification };
            // SAFETY: as above.
            let did_exit = unsafe { NSWindowDidExitFullScreenNotification };
            for (selector, name) in [
                (sel!(fullScreenWillEnter:), will_enter),
                (sel!(fullScreenDidExit:), did_exit),
            ] {
                // SAFETY: the selector is a method of this class that takes one
                // `NSNotification`, and the object is the window the notification is about.
                unsafe {
                    center.addObserver_selector_name_object(self, selector, Some(name), Some(&window));
                }
            }
            self.ivars()
                .window_full_screen
                .set(window.styleMask().contains(NSWindowStyleMask::FullScreen));
            self.lay_out();
        }

        #[unsafe(method(fullScreenWillEnter:))]
        fn full_screen_will_enter(&self, _notification: &NSNotification) {
            self.ivars().window_full_screen.set(true);
            self.lay_out();
        }

        #[unsafe(method(fullScreenDidExit:))]
        fn full_screen_did_exit(&self, _notification: &NSNotification) {
            self.ivars().window_full_screen.set(false);
            self.lay_out();
        }

        /// The card never moves the window, even where the page under a click would let it.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        /// The shadow's path follows the card's size as the window resizes.
        /// Lays the card out from the window's new size, in place of the autoresizing mask:
        /// the full-screen transition resizes the window through sizes where the mask's kept
        /// margins come out wrong, and the card would keep them.
        #[unsafe(method(resizeWithOldSuperviewSize:))]
        fn resize_with_old_superview_size(&self, _old: NSSize) {
            self.place(false);
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: `setFrameSize:` is `NSView`'s and takes the new size.
            let () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        /// The corners, the shadow, and the ring, for the frame as it is now; the ring's color
        /// resolves under the view's appearance, so it follows light and dark.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            self.paint();
        }
    }
);

impl Card {
    fn new(glow: Retained<glow::GlowView>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            inset: Cell::new(false),
            window_full_screen: Cell::new(false),
            page_full_screen: Cell::new(false),
            glow,
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer and the ivars are set.
        let card: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        card.setWantsLayer(true);
        card.setAutoresizingMask(fill());
        card
    }

    /// True while the card stands inset.
    fn is_inset(&self) -> bool {
        let ivars = self.ivars();
        ivars.inset.get() && !ivars.page_full_screen.get()
    }

    /// Puts the card where its state says, in its superview, and paints it; the glow goes with
    /// it: around the card and under it when inset, else over the page along the window's
    /// edge.
    fn lay_out(&self) {
        self.place(true);
    }

    /// [`Card::lay_out`]; `restack` also moves the glow above or below the card, which only a
    /// change of state needs, never a resize.
    fn place(&self, restack: bool) {
        // SAFETY: a getter; the superview is retained for the duration of the layout.
        let Some(parent) = (unsafe { self.superview() }) else {
            return;
        };
        let bounds = parent.bounds();
        let inset = self.is_inset();
        let frame = if inset {
            let top = match self.window() {
                Some(window) if !self.ivars().window_full_screen.get() => {
                    let layout = window.contentLayoutRect();
                    (bounds.size.height - layout.origin.y - layout.size.height).max(INSET)
                }
                _ => INSET,
            };
            card_frame(bounds.size, top)
        } else {
            bounds
        };
        self.setFrame(frame);
        self.setNeedsDisplay(true);

        let glow = &self.ivars().glow;
        let (order, edge) = if inset {
            (
                NSWindowOrderingMode::Below,
                glow::Edge {
                    radius: RADIUS,
                    around: true,
                },
            )
        } else {
            let radius = if self.ivars().window_full_screen.get() {
                0.0
            } else {
                WINDOW_RADIUS
            };
            (
                NSWindowOrderingMode::Above,
                glow::Edge {
                    radius,
                    around: false,
                },
            )
        };
        if restack {
            parent.addSubview_positioned_relativeTo(glow, order, Some(self));
        }
        glow.place(frame, edge);
    }

    fn paint(&self) {
        let (Some(layer), Some(clip)) = (self.layer(), self.subviews().firstObject()) else {
            return;
        };
        let Some(clip_layer) = clip.layer() else {
            return;
        };
        let inset = self.is_inset();
        let radius = if inset { RADIUS } else { 0.0 };
        let ring = if !inset {
            0.0
        } else if increased_contrast(self) {
            2.0
        } else {
            1.0
        };
        let bounds = self.bounds();
        let apply = RcBlock::new(move || {
            for layer in [&layer, &clip_layer] {
                layer.setCornerRadius(radius);
                // SAFETY: an immutable string exported by Core Animation.
                layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
            }
            clip_layer.setMasksToBounds(true);
            clip_layer.setBorderWidth(ring);
            clip_layer.setBorderColor(Some(&color::separator().CGColor()));
            layer.setShadowOpacity(if inset { SHADOW_OPACITY } else { 0.0 });
            layer.setShadowRadius(SHADOW_RADIUS);
            layer.setShadowOffset(NSSize::new(0.0, -SHADOW_OFFSET));
            // SAFETY: a null transform means identity; the radius fits the card, which is
            // never smaller than twice the radius in a window of the browser's least size.
            let path =
                unsafe { CGPath::with_rounded_rect(bounds, radius, radius, std::ptr::null()) };
            layer.setShadowPath(inset.then_some(&*path));
        });
        self.effectiveAppearance()
            .performAsCurrentDrawingAppearance(&apply);
    }
}

/// The card's frame in a content view of `size`, `top` points below its top edge and
/// [`INSET`] from the other three.
fn card_frame(size: NSSize, top: CGFloat) -> NSRect {
    NSRect::new(
        NSPoint::new(INSET, INSET),
        NSSize::new(
            (size.width - INSET * 2.0).max(0.0),
            (size.height - INSET - top).max(0.0),
        ),
    )
}

fn fill() -> NSAutoresizingMaskOptions {
    NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable
}

/// The page's place in one window: a card, inside it the view the engine fills with the
/// window's page, and the glow that shows the page loading.
pub struct PageFrame {
    card: Retained<Card>,
    host: Retained<NSView>,
    glow: glow::Glow,
}

impl PageFrame {
    /// A frame, edge to edge, not in a window yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let glow = glow::Glow::new(mtm);
        let card = Card::new(glow.view_retained(), mtm);
        let host = NSView::new(mtm);
        host.setWantsLayer(true);
        host.setAutoresizingMask(fill());
        card.addSubview(&host);
        Self { card, host, glow }
    }

    /// Shows the page's loading: `Some(fraction)` while it loads, `None` once it is done, which
    /// fades the glow out.
    pub fn set_loading(&self, progress: Option<f64>) {
        self.glow.set(progress);
    }

    /// Takes the glow away at once, as when another tab, with its own loading, comes forward.
    pub fn reset_loading(&self) {
        self.glow.reset();
    }

    /// True while the glow shows a load in progress.
    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.glow.is_loading()
    }

    /// How fast the glow's light goes round, in turns per second; zero while it stands still
    /// (no load, or Reduce Motion).
    #[must_use]
    pub fn glow_speed(&self) -> f64 {
        self.glow.speed()
    }

    /// The glow's view, for a render.
    #[must_use]
    pub fn glow_view(&self) -> &NSView {
        self.glow.view()
    }

    /// Puts the frame at the bottom of `content` (the window's content view), under everything
    /// else in it.
    pub fn install(&self, content: &NSView) {
        self.card
            .setFrame(NSRect::new(NSPoint::ZERO, content.bounds().size));
        self.host.setFrame(self.card.bounds());
        content.addSubview_positioned_relativeTo(&self.card, NSWindowOrderingMode::Below, None);
        self.card.lay_out();
    }

    /// The view the engine fills with the page.
    #[must_use]
    pub fn host(&self) -> &NSView {
        &self.host
    }

    /// The card, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.card
    }

    /// Turns *Inset page* on or off.
    pub fn set_inset(&self, on: bool) {
        if self.card.ivars().inset.replace(on) != on {
            self.card.lay_out();
        }
    }

    /// A page of this window went into an element's full screen (`true`) or came out of it.
    pub fn set_page_full_screen(&self, on: bool) {
        if self.card.ivars().page_full_screen.replace(on) != on {
            self.card.lay_out();
        }
    }

    /// True while the page stands inset as a card: *Inset page* is on and the page shows no
    /// element full screen.
    #[must_use]
    pub fn is_inset(&self) -> bool {
        self.card.is_inset()
    }
}

#[cfg(test)]
mod tests {
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    use super::{INSET, card_frame};

    #[test]
    fn the_card_keeps_an_even_inset_and_the_titlebar_band_on_top() {
        assert_eq!(
            card_frame(NSSize::new(1000.0, 600.0), 32.0),
            NSRect::new(
                NSPoint::new(INSET, INSET),
                NSSize::new(1000.0 - INSET * 2.0, 600.0 - INSET - 32.0)
            )
        );
    }

    #[test]
    fn a_window_smaller_than_the_inset_leaves_an_empty_card() {
        assert_eq!(card_frame(NSSize::new(4.0, 4.0), 32.0).size, NSSize::ZERO);
    }
}

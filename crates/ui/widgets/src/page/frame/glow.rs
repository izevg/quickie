//! The load glow: while the page loads, a soft light in the accent color runs along the page's
//! edge (E1: no glass, no dimming, never a click). It replaces a progress bar: a thin, faint ring
//! marks the edge, and a comet of light travels around it, faster as the engine's progress
//! grows. When loading ends it fades out with `fade-fast`.
//!
//! - Edge to edge it runs along the window's inner edge, over the page. Inset, it runs around
//!   the card as its box shadow: under the card, so only its outer half and its halo show.
//! - Core Animation only, all of it on the render server: a conic gradient (the comet) turns
//!   under a mask that is the rounded edge stroked, and the halo is that layer's shadow. The main
//!   thread only starts, re-times, and stops one animation; a speed change keeps the comet where
//!   it is (the layer's own time is carried over, never restarted).
//! - Reduce Motion: no comet, a static thin edge that fades in and out.

use std::cell::Cell;
use std::f64::consts::TAU;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAppearanceCustomization, NSAutoresizingMaskOptions, NSColor, NSView};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CACurrentMediaTime, CAGradientLayer, CALayer, CAMediaTiming,
    CAMediaTimingFunction, CAShapeLayer, CATransaction, kCAGradientLayerConic,
};
use quickie_ui_shared::motion::{self, FADE_FAST};
use quickie_ui_shared::tokens::color;

/// The edge's width, in points: thin.
const LINE: CGFloat = 1.5;
/// How far the halo spreads from the edge.
const HALO: CGFloat = 5.0;
/// The halo's strength: low, it lights the edge without covering the page.
const HALO_OPACITY: f32 = 0.7;
/// The faint ring under the comet; with Reduce Motion, the static edge.
const RING_OPACITY: f32 = 0.22;
const STILL_RING_OPACITY: f32 = 0.6;
/// Turns per second at the start of a load and near its end.
const SPIN_SLOW: f64 = 0.35;
const SPIN_FAST: f64 = 1.1;
const SPIN: &str = "spin";

/// How fast the comet goes round at `progress` (`0.0 ..= 1.0`), in turns per second.
#[must_use]
pub(super) fn spin_speed(progress: f64) -> f64 {
    SPIN_SLOW + (SPIN_FAST - SPIN_SLOW) * progress.clamp(0.0, 1.0)
}

/// How the ring looks while a page loads: the static ring's opacity, and whether the comet
/// runs.
#[must_use]
pub(super) fn ring(reduce_motion: bool) -> (f32, bool) {
    if reduce_motion {
        (STILL_RING_OPACITY, false)
    } else {
        (RING_OPACITY, true)
    }
}

/// Where the glow runs, in its view's bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Edge {
    /// The corners' radius.
    pub radius: CGFloat,
    /// Around a card, outside it (the inner half hides under the card); else inside the view.
    pub around: bool,
}

/// The layers, kept by the view so a resize can shape them.
pub(super) struct Ivars {
    ring: Retained<CAShapeLayer>,
    halo: Retained<CALayer>,
    mask: Retained<CAShapeLayer>,
    comet: Retained<CAGradientLayer>,
    edge: Cell<Edge>,
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements beyond the main thread, and `GlowView`
    // implements no `Drop`; `hitTest:` and `setFrameSize:` are public API meant to be
    // overridden.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieLoadGlowView"]
    #[ivars = Ivars]
    pub(super) struct GlowView;

    impl GlowView {
        /// Clicks go through to the page.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            None
        }

        /// The edge follows the view's size as the window resizes, in the same frame.
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: `setFrameSize:` is `NSView`'s and takes the new size.
            let () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.shape();
        }
    }
);

impl GlowView {
    /// Runs the glow along `edge` in a view at `frame`.
    pub(super) fn place(&self, frame: NSRect, edge: Edge) {
        self.ivars().edge.set(edge);
        self.setFrame(frame);
        self.shape();
    }

    /// Lays the layers out for the view's bounds and edge, without animating.
    fn shape(&self) {
        let ivars = self.ivars();
        let bounds = self.bounds();
        let edge = ivars.edge.get();
        let (rect, width) = if edge.around {
            (bounds, LINE * 2.0)
        } else {
            let half = LINE / 2.0;
            (
                NSRect::new(
                    NSPoint::new(half, half),
                    NSSize::new(
                        (bounds.size.width - LINE).max(0.0),
                        (bounds.size.height - LINE).max(0.0),
                    ),
                ),
                LINE,
            )
        };
        let radius = edge
            .radius
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0)
            .max(0.0);
        // SAFETY: a null transform means identity; the radius fits the rect (clamped above).
        let path = unsafe { CGPath::with_rounded_rect(rect, radius, radius, std::ptr::null()) };
        let side = bounds.size.width.hypot(bounds.size.height);
        transaction(0.0, || {
            for shape in [&ivars.ring, &ivars.mask] {
                shape.setFrame(bounds);
                shape.setPath(Some(&path));
                shape.setLineWidth(width);
            }
            ivars.halo.setFrame(bounds);
            ivars
                .comet
                .setBounds(NSRect::new(NSPoint::ZERO, NSSize::new(side, side)));
            ivars.comet.setPosition(NSPoint::new(
                bounds.size.width / 2.0,
                bounds.size.height / 2.0,
            ));
        });
    }

    /// The accent color in the ring, the halo, and the comet, resolved under the view's
    /// appearance.
    fn paint(&self) {
        let ivars = self.ivars();
        let (ring, halo, comet) = (ivars.ring.clone(), ivars.halo.clone(), ivars.comet.clone());
        let apply = RcBlock::new(move || {
            let accent = color::accent();
            let at = |alpha: CGFloat| accent.colorWithAlphaComponent(alpha).CGColor();
            transaction(0.0, || {
                ring.setStrokeColor(Some(&at(1.0)));
                halo.setShadowColor(Some(&at(1.0)));
                // The head at the start of the turn, the tail fading behind it: the gradient's
                // angle grows the other way from the clockwise turn.
                let stops = [at(0.0), at(1.0), at(0.45), at(0.0), at(0.0)];
                set_colors(&comet, &stops);
            });
        });
        self.effectiveAppearance()
            .performAsCurrentDrawingAppearance(&apply);
    }
}

/// The comet's colors, from its tail round to its head.
fn set_colors(comet: &CAGradientLayer, colors: &[Retained<CGColor>]) {
    let objects: Vec<&AnyObject> = colors.iter().map(|color| (***color).as_ref()).collect();
    let array = NSArray::from_slice(&objects);
    // SAFETY: every element is a `CGColor`, which is what the gradient layer takes.
    unsafe { comet.setColors(Some(&array)) };
}

/// The glow over (or under) one page frame's card.
pub(super) struct Glow {
    view: Retained<GlowView>,
    /// Counts each start and finish, so a fade that ends after a new load began leaves it.
    generation: Rc<Cell<u64>>,
    loading: Cell<bool>,
    /// The comet's turn is on the layer.
    spinning: Rc<Cell<bool>>,
}

impl Glow {
    pub(super) fn new(mtm: MainThreadMarker) -> Self {
        let ring = CAShapeLayer::new();
        ring.setFillColor(None);
        let mask = CAShapeLayer::new();
        mask.setFillColor(None);
        // Only the mask's alpha counts: opaque black.
        mask.setStrokeColor(Some(&NSColor::blackColor().CGColor()));
        let comet = CAGradientLayer::new();
        // SAFETY: an immutable string exported by Core Animation.
        comet.setType(unsafe { kCAGradientLayerConic });
        comet.setStartPoint(NSPoint::new(0.5, 0.5));
        comet.setEndPoint(NSPoint::new(0.5, 1.0));
        let stops: Vec<Retained<NSNumber>> = [0.0, 0.015, 0.14, 0.5, 1.0]
            .into_iter()
            .map(NSNumber::new_f64)
            .collect();
        comet.setLocations(Some(&NSArray::from_retained_slice(&stops)));
        let masked = CALayer::new();
        masked.addSublayer(&comet);
        // SAFETY: the mask is new and in no other layer tree.
        unsafe { masked.setMask(Some(&mask)) };
        let halo = CALayer::new();
        halo.addSublayer(&masked);
        halo.setShadowOpacity(HALO_OPACITY);
        halo.setShadowRadius(HALO);
        halo.setShadowOffset(NSSize::ZERO);
        let this = GlowView::alloc(mtm).set_ivars(Ivars {
            ring,
            halo,
            mask,
            comet,
            edge: Cell::new(Edge {
                radius: 0.0,
                around: false,
            }),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer and the ivars are set.
        let view: Retained<GlowView> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        view.setHidden(true);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        if let Some(layer) = view.layer() {
            layer.addSublayer(&view.ivars().ring);
            layer.addSublayer(&view.ivars().halo);
            layer.setOpacity(0.0);
        }
        Self {
            view,
            generation: Rc::new(Cell::new(0)),
            loading: Cell::new(false),
            spinning: Rc::new(Cell::new(false)),
        }
    }

    pub(super) fn view(&self) -> &NSView {
        &self.view
    }

    /// The view, for the card to place it with itself.
    pub(super) fn view_retained(&self) -> Retained<GlowView> {
        self.view.clone()
    }

    /// True while it shows a load in progress.
    pub(super) fn is_loading(&self) -> bool {
        self.loading.get()
    }

    /// The comet's speed now, in turns per second; zero while it does not run.
    pub(super) fn speed(&self) -> f64 {
        if self.spinning.get() {
            f64::from(self.view.ivars().comet.speed())
        } else {
            0.0
        }
    }

    /// The page loaded more: `Some(fraction)` while it loads, `None` once it is done, which
    /// fades the glow out.
    pub(super) fn set(&self, progress: Option<f64>) {
        match progress {
            Some(fraction) => self.advance(fraction),
            None => self.finish(),
        }
    }

    /// Takes the glow away at once, as when another tab, with its own loading, comes forward.
    pub(super) fn reset(&self) {
        self.generation.set(self.generation.get() + 1);
        self.loading.set(false);
        self.stop();
        self.view.setHidden(true);
    }

    fn advance(&self, fraction: f64) {
        let fraction = fraction.clamp(0.0, 1.0);
        let ivars = self.view.ivars();
        if !self.loading.replace(true) {
            self.generation.set(self.generation.get() + 1);
            let (ring_opacity, spins) = ring(motion::reduce_motion());
            self.view.paint();
            self.view.setHidden(false);
            transaction(0.0, || {
                ivars.ring.setOpacity(ring_opacity);
                ivars.halo.setHidden(!spins);
            });
            if spins {
                ivars.comet.setSpeed(as_speed(spin_speed(fraction)));
                ivars.comet.setTimeOffset(0.0);
                ivars.comet.setBeginTime(0.0);
                ivars
                    .comet
                    .addAnimation_forKey(&spin(), Some(&NSString::from_str(SPIN)));
            } else {
                ivars.comet.removeAnimationForKey(&NSString::from_str(SPIN));
            }
            self.spinning.set(spins);
            if let Some(layer) = self.view.layer() {
                transaction(FADE_FAST.duration, || layer.setOpacity(1.0));
            }
        } else if self.speed() > 0.0 {
            retime(&ivars.comet, spin_speed(fraction));
        }
    }

    fn finish(&self) {
        if !self.loading.replace(false) {
            return;
        }
        let Some(layer) = self.view.layer() else {
            return;
        };
        let generation = self.generation.get();
        let shared = Rc::clone(&self.generation);
        let (view, comet) = (self.view.clone(), self.view.ivars().comet.clone());
        let spinning = Rc::clone(&self.spinning);
        let hide = RcBlock::new(move || {
            if shared.get() == generation {
                comet.removeAnimationForKey(&NSString::from_str(SPIN));
                spinning.set(false);
                view.setHidden(true);
            }
        });
        CATransaction::begin();
        CATransaction::setAnimationDuration(FADE_FAST.duration);
        CATransaction::setAnimationTimingFunction(Some(&curve()));
        // SAFETY: the block takes no arguments, and a transaction begun on the main thread
        // calls it there, where the captured view lives.
        unsafe { CATransaction::setCompletionBlock(Some(&hide)) };
        layer.setOpacity(0.0);
        CATransaction::commit();
    }

    fn stop(&self) {
        self.view
            .ivars()
            .comet
            .removeAnimationForKey(&NSString::from_str(SPIN));
        self.spinning.set(false);
        if let Some(layer) = self.view.layer() {
            transaction(0.0, || layer.setOpacity(0.0));
        }
    }
}

/// The comet's turn: one turn a second at speed 1, clockwise, for ever.
fn spin() -> Retained<CABasicAnimation> {
    let spin =
        CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("transform.rotation.z")));
    let from = NSNumber::new_f64(0.0);
    let to = NSNumber::new_f64(-TAU);
    // SAFETY: a number is what a rotation animation takes.
    unsafe { spin.setFromValue(Some(&from)) };
    // SAFETY: as above.
    unsafe { spin.setToValue(Some(&to)) };
    spin.setDuration(1.0);
    spin.setRepeatCount(f32::INFINITY);
    spin
}

/// Changes how fast `layer`'s animations run without a jump: its own time now becomes the
/// offset it continues from.
fn retime(layer: &CALayer, speed: f64) {
    let now = CACurrentMediaTime();
    let local = layer.convertTime_fromLayer(now, None);
    layer.setTimeOffset(local);
    layer.setBeginTime(
        layer
            .superlayer()
            .map_or(now, |parent| parent.convertTime_fromLayer(now, None)),
    );
    layer.setSpeed(as_speed(speed));
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a few turns a second fit a float"
)]
fn as_speed(speed: f64) -> f32 {
    speed as f32
}

/// Runs `change` in a transaction of `duration` seconds; zero turns implicit animation off.
fn transaction(duration: f64, change: impl FnOnce()) {
    CATransaction::begin();
    if duration <= 0.0 {
        CATransaction::setDisableActions(true);
    } else {
        CATransaction::setAnimationDuration(duration);
        CATransaction::setAnimationTimingFunction(Some(&curve()));
    }
    change();
    CATransaction::commit();
}

/// The decelerating curve of `fade-fast`.
fn curve() -> Retained<CAMediaTimingFunction> {
    let [c1x, c1y, c2x, c2y] = FADE_FAST.curve;
    CAMediaTimingFunction::functionWithControlPoints(c1x, c1y, c2x, c2y)
}

#[cfg(test)]
mod tests {
    use super::{SPIN_FAST, SPIN_SLOW, ring, spin_speed};

    #[test]
    fn the_comet_speeds_up_as_the_page_loads() {
        assert!((spin_speed(0.0) - SPIN_SLOW).abs() < 1e-9);
        assert!((spin_speed(1.0) - SPIN_FAST).abs() < 1e-9);
        assert!(spin_speed(0.3) < spin_speed(0.7));
        assert!((spin_speed(-1.0) - SPIN_SLOW).abs() < 1e-9);
        assert!((spin_speed(2.0) - SPIN_FAST).abs() < 1e-9);
    }

    #[test]
    fn reduce_motion_keeps_a_still_edge() {
        let (still, spins) = ring(true);
        let (moving, runs) = ring(false);

        assert!(!spins);
        assert!(runs);
        assert!(still > moving);
    }
}

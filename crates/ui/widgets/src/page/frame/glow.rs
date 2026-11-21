//! The load glow: while the page loads, a thin line of turning color runs round the page's
//! edge with a soft bloom of the same colors (E1: no glass, no dimming, never a click). It
//! replaces a progress bar: the colors turn faster as the engine's progress grows. When loading
//! ends it fades out with `fade-fast`.
//!
//! - Edge to edge it runs along the window's inner edge, over the page. Inset, it runs around
//!   the card, under it, so only the line's outer half and the outer bloom show.
//! - Core Animation only, all of it on the render server. A conic gradient of the spectrum
//!   turns under a mask that is the rounded edge stroked thin: the line. A copy of it turns
//!   under the same edge stroked wider and feathered by the mask's own shadow: the bloom, which
//!   keeps the line's colors where a layer shadow would have one. The main thread only starts,
//!   re-times, and stops one animation per gradient; a speed change keeps the colors where they
//!   are (the layers' own time is carried over, never restarted).
//! - Reduce Motion: the same line and bloom, standing still, fading in and out.

use std::cell::Cell;
use std::f64::consts::TAU;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAppearanceCustomization, NSAutoresizingMaskOptions, NSColor, NSView};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::CGPath;
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CACurrentMediaTime, CAGradientLayer, CALayer, CAMediaTiming,
    CAMediaTimingFunction, CAShapeLayer, CATransaction, kCAGradientLayerConic,
};
use quickie_ui_shared::motion::{self, FADE_FAST};
use quickie_ui_shared::tokens::color;

/// The line's width, in points: thin.
const LINE: CGFloat = 1.5;
/// The bloom's core, stroked under the line, and how far it feathers out from it.
const BLOOM_CORE: CGFloat = 3.0;
const BLOOM: CGFloat = 9.0;
/// The bloom's strength. Over the page it stays low, so it lights the edge without covering
/// the page; around a card it shows only outside it, on the window's background.
const BLOOM_OVER_PAGE: f32 = 0.45;
const BLOOM_AROUND_CARD: f32 = 0.7;
/// Turns per second at the start of a load and near its end: slow enough to read as light, not
/// as a spinner.
const SPIN_SLOW: f64 = 0.2;
const SPIN_FAST: f64 = 0.55;
const SPIN: &str = "spin";

/// How fast the colors go round at `progress` (`0.0 ..= 1.0`), in turns per second.
#[must_use]
pub(super) fn spin_speed(progress: f64) -> f64 {
    SPIN_SLOW + (SPIN_FAST - SPIN_SLOW) * progress.clamp(0.0, 1.0)
}

/// How strong the bloom is along `edge`.
#[must_use]
pub(super) fn bloom_opacity(edge: Edge) -> f32 {
    if edge.around {
        BLOOM_AROUND_CARD
    } else {
        BLOOM_OVER_PAGE
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

/// One turning gradient of the spectrum under its mask.
struct Band {
    layer: Retained<CALayer>,
    mask: Retained<CAShapeLayer>,
    spectrum: Retained<CAGradientLayer>,
}

impl Band {
    fn new() -> Self {
        let mask = CAShapeLayer::new();
        mask.setFillColor(None);
        // Only the mask's alpha counts: opaque black.
        mask.setStrokeColor(Some(&NSColor::blackColor().CGColor()));
        let spectrum = CAGradientLayer::new();
        // SAFETY: an immutable string exported by Core Animation.
        spectrum.setType(unsafe { kCAGradientLayerConic });
        spectrum.setStartPoint(NSPoint::new(0.5, 0.5));
        spectrum.setEndPoint(NSPoint::new(0.5, 1.0));
        let layer = CALayer::new();
        layer.addSublayer(&spectrum);
        // SAFETY: the mask is new and in no other layer tree.
        unsafe { layer.setMask(Some(&mask)) };
        Self {
            layer,
            mask,
            spectrum,
        }
    }

    /// Lays the band out over `bounds` along `path`, stroked `width` wide.
    fn shape(&self, bounds: NSRect, path: &CGPath, width: CGFloat) {
        let side = bounds.size.width.hypot(bounds.size.height);
        self.layer.setFrame(bounds);
        self.mask.setFrame(bounds);
        self.mask.setPath(Some(path));
        self.mask.setLineWidth(width);
        self.spectrum
            .setBounds(NSRect::new(NSPoint::ZERO, NSSize::new(side, side)));
        self.spectrum.setPosition(NSPoint::new(
            bounds.size.width / 2.0,
            bounds.size.height / 2.0,
        ));
    }
}

/// The layers, kept by the view so a resize can shape them.
pub(super) struct Ivars {
    line: Band,
    bloom: Band,
    edge: Cell<Edge>,
}

impl Ivars {
    fn spectra(&self) -> [&CAGradientLayer; 2] {
        [&self.line.spectrum, &self.bloom.spectrum]
    }
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
        // Around a card the path is the card's edge and the strokes are doubled, as their
        // inner halves hide under it; inside the view the line's outer side touches its edge.
        let (rect, line) = if edge.around {
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
        let core = if edge.around {
            BLOOM_CORE * 2.0
        } else {
            BLOOM_CORE
        };
        transaction(0.0, || {
            ivars.line.shape(bounds, &path, line);
            ivars.bloom.shape(bounds, &path, core);
            ivars.bloom.layer.setOpacity(bloom_opacity(edge));
        });
    }

    /// The spectrum in both gradients, resolved under the view's appearance.
    fn paint(&self) {
        let ivars = self.ivars();
        let spectra = [ivars.line.spectrum.clone(), ivars.bloom.spectrum.clone()];
        let apply = RcBlock::new(move || {
            let colors = color::spectrum().map(|color| color.CGColor());
            let objects: Vec<&AnyObject> = colors.iter().map(|color| (**color).as_ref()).collect();
            let array = NSArray::from_slice(&objects);
            transaction(0.0, || {
                for spectrum in &spectra {
                    // SAFETY: every element is a `CGColor`, which is what the gradient takes.
                    unsafe { spectrum.setColors(Some(&array)) };
                }
            });
        });
        self.effectiveAppearance()
            .performAsCurrentDrawingAppearance(&apply);
    }
}

/// The glow over (or under) one page frame's card.
pub(super) struct Glow {
    view: Retained<GlowView>,
    /// Counts each start and finish, so a fade that ends after a new load began leaves it.
    generation: Rc<Cell<u64>>,
    loading: Cell<bool>,
    /// The gradients' turn is on their layers.
    spinning: Rc<Cell<bool>>,
}

impl Glow {
    pub(super) fn new(mtm: MainThreadMarker) -> Self {
        let line = Band::new();
        let bloom = Band::new();
        // The mask's shadow is part of its alpha: it feathers the bloom's core into a soft
        // light that keeps the spectrum's colors.
        bloom
            .mask
            .setShadowColor(Some(&NSColor::blackColor().CGColor()));
        bloom.mask.setShadowOpacity(1.0);
        bloom.mask.setShadowRadius(BLOOM);
        bloom.mask.setShadowOffset(NSSize::ZERO);
        let this = GlowView::alloc(mtm).set_ivars(Ivars {
            line,
            bloom,
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
            layer.addSublayer(&view.ivars().bloom.layer);
            layer.addSublayer(&view.ivars().line.layer);
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

    /// How fast the colors turn now, in turns per second; zero while they stand still.
    pub(super) fn speed(&self) -> f64 {
        if self.spinning.get() {
            f64::from(self.view.ivars().line.spectrum.speed())
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
            let spins = !motion::reduce_motion();
            self.view.paint();
            self.view.setHidden(false);
            let key = NSString::from_str(SPIN);
            for spectrum in ivars.spectra() {
                spectrum.removeAnimationForKey(&key);
                if spins {
                    spectrum.setSpeed(as_speed(spin_speed(fraction)));
                    spectrum.setTimeOffset(0.0);
                    spectrum.setBeginTime(0.0);
                    spectrum.addAnimation_forKey(&spin(), Some(&key));
                }
            }
            self.spinning.set(spins);
            if let Some(layer) = self.view.layer() {
                transaction(FADE_FAST.duration, || layer.setOpacity(1.0));
            }
        } else if self.speed() > 0.0 {
            let now = CACurrentMediaTime();
            for spectrum in ivars.spectra() {
                retime(spectrum, now, spin_speed(fraction));
            }
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
        let view = self.view.clone();
        let spinning = Rc::clone(&self.spinning);
        let hide = RcBlock::new(move || {
            if shared.get() == generation {
                halt(&view);
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
        halt(&self.view);
        self.spinning.set(false);
        if let Some(layer) = self.view.layer() {
            transaction(0.0, || layer.setOpacity(0.0));
        }
    }
}

/// Takes the turn off both gradients.
fn halt(view: &GlowView) {
    let key = NSString::from_str(SPIN);
    for spectrum in view.ivars().spectra() {
        spectrum.removeAnimationForKey(&key);
    }
}

/// The gradients' turn: one turn a second at speed 1, clockwise, for ever.
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

/// Changes how fast `layer`'s animations run without a jump: its own time at `now` becomes the
/// offset it continues from. Both gradients are re-timed at the same `now`, so they stay
/// together.
fn retime(layer: &CALayer, now: f64, speed: f64) {
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
    use super::spin_speed;
    use super::{BLOOM_AROUND_CARD, BLOOM_OVER_PAGE, Edge, SPIN_FAST, SPIN_SLOW, bloom_opacity};

    #[test]
    fn the_colors_turn_faster_as_the_page_loads() {
        assert!((spin_speed(0.0) - SPIN_SLOW).abs() < 1e-9);
        assert!((spin_speed(1.0) - SPIN_FAST).abs() < 1e-9);
        assert!(spin_speed(0.3) < spin_speed(0.7));
        assert!((spin_speed(-1.0) - SPIN_SLOW).abs() < 1e-9);
        assert!((spin_speed(2.0) - SPIN_FAST).abs() < 1e-9);
    }

    #[test]
    fn the_bloom_is_quieter_over_the_page_than_around_a_card() {
        let over = bloom_opacity(Edge {
            radius: 16.0,
            around: false,
        });
        let around = bloom_opacity(Edge {
            radius: 12.0,
            around: true,
        });

        assert!((over - BLOOM_OVER_PAGE).abs() < f32::EPSILON);
        assert!((around - BLOOM_AROUND_CARD).abs() < f32::EPSILON);
        assert!(over < around);
    }
}

//! The load progress line: a 2 px accent line along the top edge of the window while the page
//! loads (E1: no glass, no shadow, no dimming). It grows with the engine's progress by scaling
//! a layer, never by resizing it, reaches the far edge when loading ends, and fades out with
//! `fade-fast`. It never takes a click. Reduce Motion keeps the fade and drops the growth.

use std::cell::Cell;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAMediaTimingFunction, CATransaction, CATransform3D};
use quickie_ui_shared::motion::{self, FADE_FAST, STATE_CHANGE};
use quickie_ui_shared::tokens::color;

/// The line's thickness, in points.
pub const THICKNESS: CGFloat = 2.0;

/// The least the line shows once loading starts, so a slow first byte is still visible.
const LEAST: f64 = 0.05;

define_class!(
    // SAFETY: `NSView` has no subclassing requirements beyond the main thread, and
    // `PassThroughView` implements no `Drop`.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieProgressLineView"]
    struct PassThroughView;

    impl PassThroughView {
        /// Clicks go through to the page.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            None
        }
    }
);

pub struct LoadProgressLine {
    view: Retained<PassThroughView>,
    bar: Retained<CALayer>,
    /// Counts each start and finish, so a fade that ends after a new load began leaves it.
    generation: Rc<Cell<u64>>,
    loading: Cell<bool>,
}

impl LoadProgressLine {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        // SAFETY: `init` is `NSView`'s designated initializer.
        let view: Retained<PassThroughView> =
            unsafe { msg_send![PassThroughView::alloc(mtm), init] };
        view.setWantsLayer(true);
        view.setHidden(true);
        let bar = CALayer::new();
        bar.setAnchorPoint(NSPoint::new(0.0, 0.5));
        if let Some(layer) = view.layer() {
            layer.addSublayer(&bar);
        }
        Self {
            view,
            bar,
            generation: Rc::new(Cell::new(0)),
            loading: Cell::new(false),
        }
    }

    /// Shows a page's loading in `host`: `Some(fraction)` while it loads, `None` once it is
    /// done, which completes the line and fades it.
    pub fn set(&self, host: &NSView, progress: Option<f64>) {
        self.place(host);
        match progress {
            Some(fraction) => self.advance(fraction),
            None => self.finish(),
        }
    }

    /// Takes the line away at once, as when another tab, with its own loading, comes forward.
    pub fn reset(&self) {
        self.generation.set(self.generation.get() + 1);
        self.loading.set(false);
        self.view.setHidden(true);
    }

    /// True while it shows a load in progress.
    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.loading.get()
    }

    /// The fraction drawn now, `0.0 ..= 1.0`.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        self.bar.transform().m11
    }

    fn advance(&self, fraction: f64) {
        if !self.loading.get() {
            self.generation.set(self.generation.get() + 1);
            self.loading.set(true);
            self.view.setHidden(false);
            transaction(0.0, || {
                self.bar.setOpacity(1.0);
                self.bar.setTransform(scale_x(0.0));
            });
        }
        let duration = if motion::reduce_motion() {
            0.0
        } else {
            STATE_CHANGE.duration * 2.0
        };
        transaction(duration, || {
            self.bar.setTransform(scale_x(fraction.max(LEAST)));
        });
    }

    fn finish(&self) {
        if !self.loading.replace(false) {
            return;
        }
        let generation = self.generation.get();
        let shared = Rc::clone(&self.generation);
        let (bar, view) = (self.bar.clone(), self.view.clone());
        let fade = RcBlock::new(move || {
            if shared.get() != generation {
                return;
            }
            let (bar, view, shared) = (bar.clone(), view.clone(), Rc::clone(&shared));
            let hide = RcBlock::new(move || {
                if shared.get() == generation {
                    view.setHidden(true);
                }
            });
            CATransaction::begin();
            CATransaction::setAnimationDuration(FADE_FAST.duration);
            CATransaction::setAnimationTimingFunction(Some(&curve()));
            // SAFETY: the block takes no arguments, and a transaction begun on the main thread
            // calls it there, where the captured view lives.
            unsafe { CATransaction::setCompletionBlock(Some(&hide)) };
            bar.setOpacity(0.0);
            CATransaction::commit();
        });
        let duration = if motion::reduce_motion() {
            0.0
        } else {
            FADE_FAST.duration
        };
        CATransaction::begin();
        CATransaction::setAnimationDuration(duration);
        CATransaction::setAnimationTimingFunction(Some(&curve()));
        // SAFETY: as above, the block runs on the main thread that began the transaction.
        unsafe { CATransaction::setCompletionBlock(Some(&fade)) };
        self.bar.setTransform(scale_x(1.0));
        CATransaction::commit();
    }

    /// Lays the line along the top edge of `host`, above the page.
    fn place(&self, host: &NSView) {
        // SAFETY: a getter; the superview is retained for the duration of the comparison.
        let parent = unsafe { self.view.superview() };
        if parent.is_none_or(|parent| &*parent != host) {
            self.view.removeFromSuperview();
            host.addSubview(&self.view);
        }
        let bounds = host.bounds();
        self.view.setFrame(NSRect::new(
            NSPoint::new(0.0, bounds.size.height - THICKNESS),
            NSSize::new(bounds.size.width, THICKNESS),
        ));
        self.view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        transaction(0.0, || {
            self.bar
                .setBackgroundColor(Some(&color::accent().CGColor()));
            // The transform scales the bar; its untransformed bounds span the whole width.
            let transform = self.bar.transform();
            self.bar.setTransform(scale_x(1.0));
            self.bar.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(bounds.size.width, THICKNESS),
            ));
            self.bar.setTransform(transform);
        });
    }
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

fn scale_x(fraction: f64) -> CATransform3D {
    CATransform3D::new_scale(fraction.clamp(0.0, 1.0), 1.0, 1.0)
}

/// The decelerating curve of `fade-fast`.
fn curve() -> Retained<CAMediaTimingFunction> {
    let [c1x, c1y, c2x, c2y] = FADE_FAST.curve;
    CAMediaTimingFunction::functionWithControlPoints(c1x, c1y, c2x, c2y)
}

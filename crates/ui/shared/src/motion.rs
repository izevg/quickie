//! Motion tokens (`docs/design/system.md`, Motion) and the helpers that apply them.
//!
//! Keyboard-initiated surfaces appear and disappear instantly; clicks and gestures animate with
//! the tokens. Reduce Motion keeps the duration and drops scale and slide. Only `transform` and
//! `opacity` are animated.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, Message};
use objc2_app_kit::{NSView, NSWorkspace};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSArray, NSNumber, NSString, NSValue};
use objc2_quartz_core::{
    CAAnimation, CAAnimationGroup, CABasicAnimation, CAMediaTiming, CAMediaTimingFunction,
    CASpringAnimation, CATransaction, CATransform3D, NSValueCATransform3DAdditions,
    kCAFillModeBoth,
};

/// What started a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Keyboard,
    Pointer,
    /// Quickie itself, not a key or a gesture: a toast, a status chip. It animates.
    App,
}

/// The geometric part of an entrance, on top of the fade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entrance {
    Fade,
    /// Scales from this factor to 1 around the center.
    Scale(CGFloat),
    /// Slides down from this many points above.
    Slide(CGFloat),
}

/// A motion token: duration in seconds, cubic-bezier control points, entrance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub duration: f64,
    pub curve: [f32; 4],
    pub entrance: Entrance,
}

const EASE_OUT: [f32; 4] = [0.0, 0.0, 0.58, 1.0];
const EXPO_OUT: [f32; 4] = [0.16, 1.0, 0.3, 1.0];

/// Selection, toggles, focus changes.
pub const STATE_CHANGE: Motion = Motion {
    duration: 0.100,
    curve: EASE_OUT,
    entrance: Entrance::Fade,
};
/// E1 in and out.
pub const FADE_FAST: Motion = Motion {
    duration: 0.120,
    curve: EASE_OUT,
    entrance: Entrance::Fade,
};
/// E2: scale 0.96 → 1 and fade.
pub const OVERLAY_ENTER: Motion = Motion {
    duration: 0.200,
    curve: EXPO_OUT,
    entrance: Entrance::Scale(0.96),
};
/// E3: slide from the top edge and fade.
pub const SHEET_ENTER: Motion = Motion {
    duration: 0.250,
    curve: EXPO_OUT,
    entrance: Entrance::Slide(crate::tokens::space::XL),
};
/// How long the origin chip stays before it fades.
pub const ORIGIN_CHIP_HOLD: f64 = 2.4;
/// How long a toast stays before it fades.
pub const TOAST_HOLD: f64 = 3.0;

/// What a transition actually does once trigger and accessibility settings are known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// Zero means instant.
    pub duration: f64,
    pub entrance: Entrance,
}

impl Motion {
    #[must_use]
    pub fn plan(self, trigger: Trigger, reduce_motion: bool) -> Plan {
        let duration = match trigger {
            Trigger::Keyboard => 0.0,
            Trigger::Pointer | Trigger::App => self.duration,
        };
        let entrance = if reduce_motion {
            Entrance::Fade
        } else {
            self.entrance
        };
        Plan { duration, entrance }
    }
}

/// True when the user asked for reduced motion.
#[must_use]
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Shows `view` with `motion`; instantly when the keyboard asked for it.
pub fn show(view: &NSView, motion: Motion, trigger: Trigger) {
    view.setHidden(false);
    let plan = motion.plan(trigger, reduce_motion());
    if plan.duration <= 0.0 {
        return;
    }
    view.setWantsLayer(true);
    let Some(layer) = view.layer() else { return };
    let size = view.bounds().size;
    let from = match plan.entrance {
        Entrance::Fade => None,
        Entrance::Scale(factor) => {
            let (cx, cy) = (size.width / 2.0, size.height / 2.0);
            Some(
                CATransform3D::new_translation(-cx, -cy, 0.0)
                    .concat(CATransform3D::new_scale(factor, factor, 1.0))
                    .concat(CATransform3D::new_translation(cx, cy, 0.0)),
            )
        }
        Entrance::Slide(points) => Some(CATransform3D::new_translation(0.0, points, 0.0)),
    };
    let mut parts = vec![basic(
        "opacity",
        &NSNumber::new_f32(0.0),
        &NSNumber::new_f32(1.0),
    )];
    if let Some(from) = from {
        parts.push(basic(
            "transform",
            &transform_value(from),
            &transform_value(CATransform3D::new_scale(1.0, 1.0, 1.0)),
        ));
    }
    let group = CAAnimationGroup::new();
    let parts: Vec<&CAAnimation> = parts.iter().map(|a| &****a).collect();
    group.setAnimations(Some(&NSArray::from_slice(&parts)));
    group.setDuration(plan.duration);
    group.setTimingFunction(Some(&curve(motion.curve)));
    layer.addAnimation_forKey(&group, Some(&NSString::from_str("quickie.show")));
}

/// Hides `view` with a fade of `motion`'s duration; instantly when the keyboard asked for it.
pub fn hide(view: &NSView, motion: Motion, trigger: Trigger) {
    let plan = motion.plan(trigger, reduce_motion());
    let layer = view.layer();
    let Some(layer) = layer.filter(|_| plan.duration > 0.0) else {
        view.setHidden(true);
        return;
    };
    CATransaction::begin();
    let target: Retained<NSView> = view.retain();
    let done = RcBlock::new(move || {
        target.setHidden(true);
        if let Some(layer) = target.layer() {
            layer.setOpacity(1.0);
        }
    });
    // SAFETY: the block takes no arguments, as the completion block must, and a transaction
    // begun on the main thread calls it there, where touching the captured view is allowed.
    unsafe { CATransaction::setCompletionBlock(Some(&done)) };
    let fade = basic("opacity", &NSNumber::new_f32(1.0), &NSNumber::new_f32(0.0));
    fade.setDuration(plan.duration);
    fade.setTimingFunction(Some(&curve(motion.curve)));
    layer.setOpacity(0.0);
    layer.addAnimation_forKey(&fade, Some(&NSString::from_str("quickie.hide")));
    CATransaction::commit();
}

fn basic(key_path: &str, from: &AnyObject, to: &AnyObject) -> Retained<CABasicAnimation> {
    let animation = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str(key_path)));
    // SAFETY: `from` matches the animated property: an `NSNumber` for opacity, an `NSValue`
    // holding a `CATransform3D` for transform.
    unsafe { animation.setFromValue(Some(from)) };
    // SAFETY: as above, `to` has the property's type.
    unsafe { animation.setToValue(Some(to)) };
    animation
}

fn transform_value(transform: CATransform3D) -> Retained<NSValue> {
    // SAFETY: boxing a plain `CATransform3D` by value has no preconditions.
    unsafe { NSValue::valueWithCATransform3D(transform) }
}

fn curve(points: [f32; 4]) -> Retained<CAMediaTimingFunction> {
    let [c1x, c1y, c2x, c2y] = points;
    CAMediaTimingFunction::functionWithControlPoints(c1x, c1y, c2x, c2y)
}

/// The scale a surface grows from while a gesture reveals it: never from nothing.
pub const GESTURE_SCALE_FROM: CGFloat = 0.97;
/// `gesture-settle`: the perceptual duration of the spring that finishes a gesture, no bounce.
pub const GESTURE_SETTLE: f64 = 0.3;

/// Animation keys of a gesture-driven surface.
const FOLLOW: &str = "quickie.gesture.follow";
const SETTLE: &str = "quickie.gesture.settle";

/// How a gesture-driven surface looks at `progress` (0 hidden, 1 shown, clamped): its
/// opacity, and its scale unless Reduce Motion is on.
#[must_use]
pub fn gesture_appearance(progress: f64, reduce_motion: bool) -> (f32, CGFloat) {
    let progress = progress.clamp(0.0, 1.0);
    let scale = if reduce_motion {
        1.0
    } else {
        GESTURE_SCALE_FROM + (1.0 - GESTURE_SCALE_FROM) * progress
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an opacity in 0…1; f32 is what Core Animation stores"
    )]
    let opacity = progress as f32;
    (opacity, scale)
}

/// The spring's initial velocity for a gesture moving `velocity` progress per second, from
/// `progress` toward `target`: Core Animation measures it in whole distances per second. A
/// surface already there, or a fling away from the target, starts at rest.
#[must_use]
pub fn settle_velocity(progress: f64, target: f64, velocity: f64) -> f64 {
    let distance = target - progress;
    if distance.abs() < 1e-3 {
        return 0.0;
    }
    (velocity / distance).clamp(0.0, 20.0)
}

/// Puts `view` where the fingers are, 1:1 and without easing: a paused animation scrubbed to
/// `progress`, so the layer geometry `AppKit` owns stays untouched. A view that does not
/// `scale` (a scrim) only fades.
pub fn follow(view: &NSView, progress: f64, scale: bool) {
    view.setWantsLayer(true);
    view.setHidden(false);
    let Some(layer) = view.layer() else { return };
    layer.removeAnimationForKey(&NSString::from_str(SETTLE));
    let (opacity, scale) = gesture_appearance(progress, reduce_motion() || !scale);
    let group = CAAnimationGroup::new();
    let parts = [
        basic(
            "opacity",
            &NSNumber::new_f32(opacity),
            &NSNumber::new_f32(opacity),
        ),
        basic(
            "transform",
            &transform_value(centered_scale(view, scale)),
            &transform_value(centered_scale(view, scale)),
        ),
    ];
    let parts: Vec<&CAAnimation> = parts.iter().map(|a| &****a).collect();
    group.setAnimations(Some(&NSArray::from_slice(&parts)));
    hold(&group);
    group.setSpeed(0.0);
    layer.addAnimation_forKey(&group, Some(&NSString::from_str(FOLLOW)));
}

/// The progress `view` shows right now, mid-gesture or mid-settle: a new gesture starts from
/// here instead of jumping. `None` when no gesture moved it.
#[must_use]
pub fn presented_progress(view: &NSView) -> Option<f64> {
    let layer = view.layer()?;
    let moving = [FOLLOW, SETTLE].into_iter().any(|key| {
        // SAFETY: a lookup by key; the animation is only checked for presence, never changed.
        unsafe { layer.animationForKey(&NSString::from_str(key)) }.is_some()
    });
    if !moving {
        return None;
    }
    // SAFETY: a getter; the presentation layer is a copy for reading.
    let presented = unsafe { layer.presentationLayer() }?;
    Some(f64::from(presented.opacity()))
}

/// Finishes a gesture from `progress` to shown or hidden, carrying the gesture's `velocity`
/// (progress per second) into a `gesture-settle` spring; Reduce Motion fades with `fade-fast`
/// instead. `done` runs once it rests; for a hidden end, the caller then takes it away. A view
/// that does not `scale` only fades, as in [`follow`].
pub fn settle(
    view: &NSView,
    (progress, velocity): (f64, f64),
    shown: bool,
    scale: bool,
    done: impl FnOnce() + 'static,
) {
    let Some(layer) = view.layer() else {
        done();
        return;
    };
    let reduce = reduce_motion();
    let target = if shown { 1.0 } else { 0.0 };
    let (from_opacity, from_scale) = gesture_appearance(progress, reduce || !scale);
    let (to_opacity, to_scale) = gesture_appearance(target, reduce || !scale);
    let parts = [
        basic_or_spring(
            "opacity",
            &NSNumber::new_f32(from_opacity),
            &NSNumber::new_f32(to_opacity),
            reduce,
        ),
        basic_or_spring(
            "transform",
            &transform_value(centered_scale(view, from_scale)),
            &transform_value(centered_scale(view, to_scale)),
            reduce,
        ),
    ];
    let duration = if reduce {
        FADE_FAST.duration
    } else {
        let initial = settle_velocity(progress, target, velocity);
        for part in &parts {
            if let Some(spring) = part.downcast_ref::<CASpringAnimation>() {
                spring.setInitialVelocity(initial);
            }
        }
        parts
            .iter()
            .filter_map(|part| part.downcast_ref::<CASpringAnimation>())
            .map(CASpringAnimation::settlingDuration)
            .fold(0.0, f64::max)
    };
    let group = CAAnimationGroup::new();
    let parts: Vec<&CAAnimation> = parts.iter().map(|a| &**a).collect();
    group.setAnimations(Some(&NSArray::from_slice(&parts)));
    group.setDuration(duration);
    hold(&group);
    CATransaction::begin();
    let target_view: Retained<NSView> = view.retain();
    let done = std::cell::Cell::new(Some(done));
    let finished = RcBlock::new(move || {
        if let Some(layer) = target_view.layer() {
            layer.removeAnimationForKey(&NSString::from_str(SETTLE));
        }
        if let Some(done) = done.take() {
            done();
        }
    });
    // SAFETY: the block takes no arguments, as the completion block must, and a transaction
    // begun on the main thread calls it there, where touching the captured view is allowed.
    unsafe { CATransaction::setCompletionBlock(Some(&finished)) };
    layer.removeAnimationForKey(&NSString::from_str(FOLLOW));
    layer.addAnimation_forKey(&group, Some(&NSString::from_str(SETTLE)));
    CATransaction::commit();
}

/// Drops any gesture animation from `view`: it shows its model values again.
pub fn stop_gesture(view: &NSView) {
    if let Some(layer) = view.layer() {
        layer.removeAnimationForKey(&NSString::from_str(FOLLOW));
        layer.removeAnimationForKey(&NSString::from_str(SETTLE));
    }
}

/// Keeps an animation's end values on screen until it is removed.
fn hold(animation: &CAAnimation) {
    animation.setRemovedOnCompletion(false);
    // SAFETY: `kCAFillModeBoth` is an immutable string exported by Core Animation.
    animation.setFillMode(unsafe { kCAFillModeBoth });
}

fn basic_or_spring(
    key_path: &str,
    from: &AnyObject,
    to: &AnyObject,
    reduce: bool,
) -> Retained<CAAnimation> {
    if reduce {
        let fade = basic(key_path, from, to);
        fade.setDuration(FADE_FAST.duration);
        fade.setTimingFunction(Some(&curve(FADE_FAST.curve)));
        return Retained::into_super(Retained::into_super(fade));
    }
    let spring = CASpringAnimation::initWithPerceptualDuration_bounce(
        CASpringAnimation::alloc(),
        GESTURE_SETTLE,
        0.0,
    );
    spring.setKeyPath(Some(&NSString::from_str(key_path)));
    // SAFETY: `from` and `to` have the animated property's type, as in `basic`.
    unsafe { spring.setFromValue(Some(from)) };
    // SAFETY: as above.
    unsafe { spring.setToValue(Some(to)) };
    spring.setDuration(spring.settlingDuration());
    Retained::into_super(Retained::into_super(Retained::into_super(spring)))
}

/// `scale` around the center of `view`; layer-backed views anchor at their bottom left.
fn centered_scale(view: &NSView, scale: CGFloat) -> CATransform3D {
    let size = view.bounds().size;
    let (cx, cy) = (size.width / 2.0, size.height / 2.0);
    CATransform3D::new_translation(-cx, -cy, 0.0)
        .concat(CATransform3D::new_scale(scale, scale, 1.0))
        .concat(CATransform3D::new_translation(cx, cy, 0.0))
}

#[cfg(test)]
mod tests {
    use super::{
        Entrance, GESTURE_SCALE_FROM, OVERLAY_ENTER, SHEET_ENTER, Trigger, gesture_appearance,
        settle_velocity,
    };

    #[test]
    fn a_gesture_grows_the_surface_from_almost_full_size_as_it_fades_in() {
        let (opacity, scale) = gesture_appearance(0.0, false);
        assert!(opacity.abs() < f32::EPSILON);
        assert!((scale - GESTURE_SCALE_FROM).abs() < f64::EPSILON);

        let (opacity, scale) = gesture_appearance(0.5, false);
        assert!((opacity - 0.5).abs() < f32::EPSILON);
        assert!((scale - 0.985).abs() < 1e-9);

        let (opacity, scale) = gesture_appearance(1.0, false);
        assert!((opacity - 1.0).abs() < f32::EPSILON);
        assert!((scale - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gesture_progress_is_clamped_and_reduce_motion_only_fades() {
        let (opacity, scale) = gesture_appearance(1.7, false);
        assert!((opacity - 1.0).abs() < f32::EPSILON);
        assert!((scale - 1.0).abs() < f64::EPSILON);
        assert!(gesture_appearance(-0.3, false).0.abs() < f32::EPSILON);

        let (opacity, scale) = gesture_appearance(0.25, true);
        assert!((opacity - 0.25).abs() < f32::EPSILON);
        assert!((scale - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_spring_starts_with_the_fingers_velocity_in_distances_per_second() {
        // Half the way left, moving at two progress per second toward 1: four distances.
        assert!((settle_velocity(0.5, 1.0, 2.0) - 4.0).abs() < f64::EPSILON);
        // Closing: progress falls toward 0.
        assert!((settle_velocity(0.8, 0.0, -1.6) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_fling_away_or_no_distance_settles_from_rest_and_speed_is_capped() {
        assert!(settle_velocity(0.5, 1.0, -3.0).abs() < f64::EPSILON);
        assert!(settle_velocity(1.0, 1.0, 5.0).abs() < f64::EPSILON);
        assert!((settle_velocity(0.99, 1.0, 50.0) - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn keyboard_transitions_are_instant() {
        let plan = OVERLAY_ENTER.plan(Trigger::Keyboard, false);
        assert!(plan.duration.abs() < f64::EPSILON);
    }

    #[test]
    fn pointer_transitions_use_the_token() {
        let plan = OVERLAY_ENTER.plan(Trigger::Pointer, false);
        assert!((plan.duration - 0.2).abs() < f64::EPSILON);
        assert_eq!(plan.entrance, Entrance::Scale(0.96));
    }

    #[test]
    fn reduce_motion_keeps_the_duration_and_only_fades() {
        let plan = SHEET_ENTER.plan(Trigger::Pointer, true);
        assert!((plan.duration - 0.25).abs() < f64::EPSILON);
        assert_eq!(plan.entrance, Entrance::Fade);
    }
}

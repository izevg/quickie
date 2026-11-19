//! Motion tokens (`docs/design/system.md`, Motion) and the helpers that apply them.
//!
//! Keyboard-initiated surfaces appear and disappear instantly; clicks and gestures animate with
//! the tokens. Reduce Motion keeps the duration and drops scale and slide. Only `transform` and
//! `opacity` are animated.

use block2::RcBlock;
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSView, NSWorkspace};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSArray, NSNumber, NSString, NSValue};
use objc2_quartz_core::{
    CAAnimation, CAAnimationGroup, CABasicAnimation, CAMediaTiming, CAMediaTimingFunction,
    CATransaction, CATransform3D, NSValueCATransform3DAdditions,
};

/// What started a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Keyboard,
    Pointer,
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
            Trigger::Pointer => self.duration,
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

#[cfg(test)]
mod tests {
    use super::{Entrance, OVERLAY_ENTER, SHEET_ENTER, Trigger};

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

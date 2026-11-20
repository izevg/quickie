//! The mini player: when the user leaves a tab whose video plays, the video floats over the
//! window in a small rounded player, in a corner, as in Arc. Only the user's own action (a tab
//! switch, the switcher, a new tab) brings it up; nothing shows on hover, and its controls are
//! always there, not revealed: play or pause, back to the tab, and close.
//!
//! It is the page's own view, not a picture: the browser's engine moves the tab's page into
//! [`MiniPlayer::slot`] and makes its video fill it, so it plays on and the window server
//! composites it like any page. A clear grip over the video takes every press, so the page
//! never sees the pointer; dragging moves the player anywhere over the window, 1:1, and on
//! release it springs (`gesture-settle`) to the corner it was flung toward, or fades there
//! under Reduce Motion.
//!
//! It is content on the page, not a glass surface: a rounded video on black with a medium
//! shadow and no dimming, and its controls on one E1 pill over the video, in the theme's
//! colors as every indicator. It appears and goes at once after a key, and with `fade-fast`
//! after a click.

mod grip;
pub mod snap;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSColor, NSImage, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::kCACornerCurveContinuous;
use quickie_ui_shared::tokens::{TextRole, color, radius, space};
use quickie_ui_shared::{Elevation, GlassSurface, Trigger, cast_shadow, motion, symbol};

use grip::{Grip, Phase};
pub use snap::Corner;

/// A round control button.
const BUTTON: f64 = 28.0;

/// How far a press moves before it drags the player rather than staying a click.
const SLOP: f64 = 3.0;

/// A release this long after the pointer last moved throws nothing: the pointer had stopped.
const STILL: f64 = 0.08;

/// What the player asks of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerRequest {
    /// Pause the video when it plays, play it when it does not.
    PlayPause,
    /// Go back to the video's tab; the player ends there.
    BackToTab,
    /// Close the player; the video pauses.
    Close,
}

type Handler = Rc<dyn Fn(PlayerRequest)>;

pub struct TargetIvars {
    handler: RefCell<Option<Handler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `PlayerTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieMiniPlayerTarget"]
    #[ivars = TargetIvars]
    pub struct PlayerTarget;

    unsafe impl NSObjectProtocol for PlayerTarget {}

    impl PlayerTarget {
        #[unsafe(method(playPause:))]
        fn play_pause(&self, _sender: Option<&NSButton>) {
            self.ask(PlayerRequest::PlayPause);
        }

        #[unsafe(method(backToTab:))]
        fn back_to_tab(&self, _sender: Option<&NSButton>) {
            self.ask(PlayerRequest::BackToTab);
        }

        #[unsafe(method(close:))]
        fn close(&self, _sender: Option<&NSButton>) {
            self.ask(PlayerRequest::Close);
        }
    }
);

impl PlayerTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn ask(&self, request: PlayerRequest) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }
}

/// A drag under way, in the host's coordinates.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// Where the press was.
    start: NSPoint,
    /// How far the player was drawn from its frame at the press: a settle it interrupted.
    base: NSPoint,
    /// How far it is drawn from its frame now.
    offset: NSPoint,
    /// Where the pointer was last, and when.
    last: NSPoint,
    time: f64,
    /// How fast the pointer moved, in points per second.
    velocity: NSPoint,
    /// The press moved past the slop: it is a drag.
    moved: bool,
}

pub struct MiniPlayer {
    /// What the host holds: the shadow, the appearance, the frame in the window.
    root: Retained<NSView>,
    /// The rounded black frame the video is drawn in.
    clip: Retained<NSView>,
    slot: Retained<NSView>,
    grip: Retained<Grip>,
    controls: GlassSurface,
    play: Retained<NSButton>,
    target: Retained<PlayerTarget>,
    corner: Cell<Corner>,
    playing: Cell<bool>,
    open: Cell<bool>,
    drag: Cell<Option<Drag>>,
    mtm: MainThreadMarker,
}

impl MiniPlayer {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        let root = NSView::new(mtm);
        root.setWantsLayer(true);
        let clip = NSView::new(mtm);
        clip.setWantsLayer(true);
        clip.setAutoresizingMask(fill());
        if let Some(layer) = clip.layer() {
            layer.setBackgroundColor(Some(&NSColor::blackColor().CGColor()));
            layer.setCornerRadius(radius::CARD);
            // SAFETY: an immutable string exported by Core Animation.
            layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
            layer.setMasksToBounds(true);
        }
        root.addSubview(&clip);
        let slot = NSView::new(mtm);
        slot.setAutoresizingMask(fill());
        clip.addSubview(&slot);
        let grip = Grip::new(mtm);
        grip.setAutoresizingMask(fill());
        grip.setAccessibilityElement(true);
        clip.addSubview(&grip);
        let target = PlayerTarget::new(mtm);
        let controls = GlassSurface::new(Elevation::E1, mtm);
        let play = button("pause.fill", "Pause", sel!(playPause:), &target, mtm);
        lay_out_controls(
            &controls,
            &[
                play.clone(),
                button("pip.exit", "Back to Tab", sel!(backToTab:), &target, mtm),
                button("xmark", "Close", sel!(close:), &target, mtm),
            ],
        );
        controls.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        clip.addSubview(controls.view());
        let player = Rc::new(Self {
            root,
            clip,
            slot,
            grip,
            controls,
            play,
            target,
            corner: Cell::new(Corner::BottomRight),
            playing: Cell::new(true),
            open: Cell::new(false),
            drag: Cell::new(None),
            mtm,
        });
        let weak: Weak<Self> = Rc::downgrade(&player);
        player.grip.on_pointer(Box::new(move |phase, at, time| {
            if let Some(player) = weak.upgrade() {
                player.pointer(phase, at, time);
            }
        }));
        player
    }

    /// Runs `handler` with what the user asks of the player.
    pub fn on_request(&self, handler: impl Fn(PlayerRequest) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// Where the engine puts the floating page: it fills it, under the grip and the controls.
    #[must_use]
    pub fn slot(&self) -> &NSView {
        &self.slot
    }

    /// The player, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.root
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The corner the player rests in.
    #[must_use]
    pub fn corner(&self) -> Corner {
        self.corner.get()
    }

    /// True while its button says the video plays (it offers to pause).
    #[must_use]
    pub fn playing(&self) -> bool {
        self.playing.get()
    }

    /// Shows the player over `host`, in the corner it last rested in, for the video of the
    /// tab titled `title`; at once after a key. In another window's host, it moves there.
    pub fn show(&self, host: &NSView, title: &str, playing: bool, trigger: Trigger) {
        // SAFETY: a getter; the superview is retained for the duration of the comparison.
        let parent = unsafe { self.root.superview() };
        if parent.as_deref() != Some(host) {
            self.root.removeFromSuperview();
            host.addSubview(&self.root);
        }
        self.lay_out(host);
        self.grip
            .setAccessibilityLabel(Some(&NSString::from_str(&format!("Video of {title}"))));
        self.set_playing(playing);
        if !self.open.replace(true) {
            motion::show(&self.root, motion::FADE_FAST, trigger);
        }
    }

    /// Takes the player away: at once after a key, with `fade-fast` after a click.
    pub fn hide(&self, trigger: Trigger) {
        if !self.open.replace(false) {
            return;
        }
        self.drag.set(None);
        motion::hide(&self.root, motion::FADE_FAST, trigger);
    }

    /// Shows whether the video plays: the button offers to pause it, or to play it.
    pub fn set_playing(&self, playing: bool) {
        self.playing.set(playing);
        let (name, label) = if playing {
            ("pause.fill", "Pause")
        } else {
            ("play.fill", "Play")
        };
        if let Some(image) = symbol_image(name, label, self.mtm) {
            self.play.setImage(Some(&image));
        }
        self.play
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
    }

    /// Puts the player in `corner` at once.
    pub fn place(&self, corner: Corner) {
        self.corner.set(corner);
        // SAFETY: a getter; the superview is retained while it is laid out in.
        if let Some(host) = unsafe { self.root.superview() } {
            self.lay_out(&host);
        }
    }

    /// Lets go of the player with its center at `center` in the host, moving at `velocity`
    /// points per second: it settles in the corner it heads for.
    pub fn drop_at(&self, center: NSPoint, velocity: NSPoint) {
        // SAFETY: a getter; the superview is retained while the player settles in it.
        let Some(host) = (unsafe { self.root.superview() }) else {
            return;
        };
        let size = self.root.frame().size;
        let drawn = NSPoint::new(center.x - size.width / 2.0, center.y - size.height / 2.0);
        let heading = snap::projected(center, velocity);
        self.corner.set(Corner::nearest(
            host.bounds().size,
            heading,
            host.isFlipped(),
        ));
        self.lay_out(&host);
        let rest = self.root.frame().origin;
        let offset = NSPoint::new(drawn.x - rest.x, drawn.y - rest.y);
        motion::settle_drag(&self.root, offset, velocity);
    }

    /// Fits the player to `host` in its corner, keeping to the corner as the window resizes.
    fn lay_out(&self, host: &NSView) {
        let bounds = host.bounds().size;
        let flipped = host.isFlipped();
        let corner = self.corner.get();
        let frame = snap::frame(bounds, snap::size_in(bounds), corner, flipped);
        self.root.setFrame(frame);
        self.clip.setFrame(NSRect::new(NSPoint::ZERO, frame.size));
        self.root.setAutoresizingMask(stick_to(corner, flipped));
        let pill = self.controls.view().frame().size;
        self.controls.set_frame(NSRect::new(
            NSPoint::new(((frame.size.width - pill.width) / 2.0).round(), space::SM),
            pill,
        ));
        // The medium shadow of an E2 surface, without its dimming: the player floats, but the
        // page stays as it is.
        cast_shadow(&self.root, Elevation::E2, radius::CARD);
    }

    /// A press, a drag, or a release on the grip, at `in_window`.
    fn pointer(&self, phase: Phase, in_window: NSPoint, time: f64) {
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let Some(host) = (unsafe { self.root.superview() }) else {
            return;
        };
        let at = host.convertPoint_fromView(in_window, None);
        match phase {
            Phase::Press => {
                let base = motion::dragged_offset(&self.root);
                self.drag.set(Some(Drag {
                    start: at,
                    base,
                    offset: base,
                    last: at,
                    time,
                    velocity: NSPoint::ZERO,
                    moved: false,
                }));
                if base != NSPoint::ZERO {
                    motion::drag(&self.root, base);
                }
            }
            Phase::Drag => {
                if let Some(drag) = self.drag.get() {
                    self.drag.set(Some(self.dragged(drag, &host, at, time)));
                }
            }
            Phase::Release => {
                let Some(drag) = self.drag.take() else {
                    return;
                };
                if !drag.moved && drag.base == NSPoint::ZERO {
                    return;
                }
                let velocity = if time - drag.time > STILL {
                    NSPoint::ZERO
                } else {
                    drag.velocity
                };
                let frame = self.root.frame();
                self.drop_at(
                    NSPoint::new(
                        frame.origin.x + drag.offset.x + frame.size.width / 2.0,
                        frame.origin.y + drag.offset.y + frame.size.height / 2.0,
                    ),
                    velocity,
                );
            }
        }
    }

    /// Follows the pointer to `at`: the player moves with it, inside the window.
    fn dragged(&self, mut drag: Drag, host: &NSView, at: NSPoint, time: f64) -> Drag {
        let moved = NSPoint::new(at.x - drag.start.x, at.y - drag.start.y);
        drag.moved |= moved.x.hypot(moved.y) > SLOP;
        if !drag.moved {
            return drag;
        }
        let elapsed = time - drag.time;
        if elapsed > 0.0 {
            let now = NSPoint::new(
                (at.x - drag.last.x) / elapsed,
                (at.y - drag.last.y) / elapsed,
            );
            // Half the last sample, half the ones before: a jittery pointer does not fling.
            drag.velocity = NSPoint::new(
                f64::midpoint(drag.velocity.x, now.x),
                f64::midpoint(drag.velocity.y, now.y),
            );
        }
        drag.offset = snap::within(
            host.bounds().size,
            self.root.frame(),
            NSPoint::new(drag.base.x + moved.x, drag.base.y + moved.y),
        );
        drag.last = at;
        drag.time = time;
        motion::drag(&self.root, drag.offset);
        drag
    }
}

fn fill() -> NSAutoresizingMaskOptions {
    NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable
}

/// Keeps a player in `corner` there as the window resizes: the margins away from it stretch.
fn stick_to(corner: Corner, flipped: bool) -> NSAutoresizingMaskOptions {
    let horizontal = match corner {
        Corner::TopLeft | Corner::BottomLeft => NSAutoresizingMaskOptions::ViewMaxXMargin,
        Corner::TopRight | Corner::BottomRight => NSAutoresizingMaskOptions::ViewMinXMargin,
    };
    let top = matches!(corner, Corner::TopLeft | Corner::TopRight);
    let vertical = if top == flipped {
        NSAutoresizingMaskOptions::ViewMaxYMargin
    } else {
        NSAutoresizingMaskOptions::ViewMinYMargin
    };
    horizontal | vertical
}

/// The symbol `name` at the controls' size, described as `label`.
fn symbol_image(name: &str, label: &str, mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
    symbol(name, label, TextRole::Headline, &color::label(), mtm)?.image()
}

/// A borderless round button showing the symbol `name`, sending `action` to `target`, named
/// `label` for `VoiceOver`.
fn button(
    name: &str,
    label: &str,
    action: Sel,
    target: &PlayerTarget,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    // SAFETY: `target` implements `action`, which takes a sender, and the player keeps the
    // target alive for as long as its buttons.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::new(), Some(target), Some(action), mtm)
    };
    if let Some(image) = symbol_image(name, label, mtm) {
        button.setImage(Some(&image));
    }
    button.setBordered(false);
    button.setContentTintColor(Some(&color::label()));
    button.setRefusesFirstResponder(true);
    button.setAccessibilityLabel(Some(&NSString::from_str(label)));
    button.setFrameSize(NSSize::new(BUTTON, BUTTON));
    button
}

/// Sizes the pill around `buttons`, side by side, and puts them in it.
fn lay_out_controls(controls: &GlassSurface, buttons: &[Retained<NSButton>]) {
    let pad = space::XS;
    let count = CGFloat::from(u32::try_from(buttons.len()).unwrap_or(0));
    let width = pad * 2.0 + BUTTON * count + space::XS * (count - 1.0).max(0.0);
    let height = BUTTON + pad * 2.0;
    controls.set_frame(NSRect::new(NSPoint::ZERO, NSSize::new(width, height)));
    let content = controls.content_view();
    let mut x = pad;
    for button in buttons {
        button.setFrameOrigin(NSPoint::new(x, pad));
        content.addSubview(button);
        x += BUTTON + space::XS;
    }
}

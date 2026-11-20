//! The mini player: the video of a tab the user left, playing or paused, and resting in each
//! corner after a drag. Two bands of colour stand in for the video, which only an engine
//! draws.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::stories::{LIGHT, LIGHT_DARK, Scene, Story, Theme};
use quickie_ui_widgets::tabs::mini_player::snap;
use quickie_ui_widgets::{Corner, MiniPlayer, PlayerRequest};

/// A window small enough that a render of all of it shows which corner the player is in.
const SMALL: (f64, f64) = (720.0, 450.0);

const fn player(
    state: &'static str,
    themes: &'static [Theme],
    window: (f64, f64),
    scene: fn(&mut Scene),
) -> Story {
    Story {
        group: "tabs",
        widget: "mini-player",
        state,
        themes,
        window,
        scene,
    }
}

pub const STORIES: &[Story] = &[
    player("playing", LIGHT_DARK, SMALL, playing),
    player("paused", LIGHT, SMALL, paused),
    player("top-left", LIGHT, SMALL, top_left),
    player("top-right", LIGHT, SMALL, top_right),
    player("bottom-left", LIGHT, SMALL, bottom_left),
    player("bottom-right", LIGHT, SMALL, bottom_right),
];

/// A video frame's stand-in, filling `slot` as it is now: a dusk sky over a lit ground, the
/// same in every theme, as a video is.
fn video(slot: &NSView, mtm: MainThreadMarker) {
    let size = slot.bounds().size;
    let fill = |red, green, blue, height: f64| {
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(size.width, height));
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0);
            layer.setBackgroundColor(Some(&color.CGColor()));
        }
        slot.addSubview(&view);
    };
    fill(0.16, 0.20, 0.36, size.height);
    fill(0.85, 0.55, 0.30, (size.height * 0.35).round());
}

/// The player over the page, showing a video of `title`, the whole window in the render.
fn shown(scene: &mut Scene, playing: bool) -> Rc<MiniPlayer> {
    let player = MiniPlayer::new(scene.mtm());
    player.show(
        scene.host(),
        "Rust in 100 Seconds — YouTube",
        playing,
        Trigger::Keyboard,
    );
    video(player.slot(), scene.mtm());
    let host = scene.host().retain();
    scene.crop_to(&host);
    scene.check(player.is_open(), "the player shows");
    player
}

/// Plays, then each control: play or pause, back to the tab, close.
fn playing(scene: &mut Scene) {
    let player = shown(scene, true);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&asked);
    player.on_request(move |request| heard.borrow_mut().push(request));
    scene.check(
        scene
            .accessibility()
            .contains("Video of Rust in 100 Seconds"),
        "the video is named after its tab",
    );
    for label in ["Pause", "Back to Tab", "Close"] {
        match scene.view_labelled(label) {
            Some(button) => scene.click(&button),
            None => scene.check(false, format!("no {label} button")),
        }
    }
    scene.check_eq(
        asked.borrow().clone(),
        vec![
            PlayerRequest::PlayPause,
            PlayerRequest::BackToTab,
            PlayerRequest::Close,
        ],
        "each control asks the browser",
    );
    scene.check_eq(
        player.corner(),
        Corner::BottomRight,
        "it starts bottom right",
    );
    scene.keep(player);
}

fn paused(scene: &mut Scene) {
    let player = shown(scene, true);
    player.set_playing(false);
    scene.check(!player.playing(), "it says the video is paused");
    scene.check(
        scene.view_labelled("Play").is_some() && scene.view_labelled("Pause").is_none(),
        "the button offers to play",
    );
    scene.keep(player);
}

/// Drags the player and lets go with its center at `x`, `y` of the window (0 to 1 from the
/// bottom left), moving at `velocity`; it must rest in `corner`, a margin from the edges.
fn dropped(scene: &mut Scene, (x, y): (f64, f64), velocity: NSPoint, corner: Corner) {
    let player = shown(scene, true);
    let bounds = scene.host().bounds().size;
    player.drop_at(NSPoint::new(bounds.width * x, bounds.height * y), velocity);
    scene.check_eq(player.corner(), corner, "the corner it rests in");
    let size = snap::size_in(bounds);
    scene.check_eq(
        player.view().frame(),
        snap::frame(bounds, size, corner, scene.host().isFlipped()),
        "where it rests",
    );
    scene.keep(player);
}

/// Let go just past the middle toward the top left.
fn top_left(scene: &mut Scene) {
    dropped(scene, (0.45, 0.55), NSPoint::ZERO, Corner::TopLeft);
}

/// Flung up and right from the bottom left: the fling decides, not where it was let go.
fn top_right(scene: &mut Scene) {
    dropped(
        scene,
        (0.3, 0.3),
        NSPoint::new(2000.0, 1500.0),
        Corner::TopRight,
    );
}

fn bottom_left(scene: &mut Scene) {
    dropped(scene, (0.1, 0.2), NSPoint::ZERO, Corner::BottomLeft);
}

/// Dragged away and brought back home.
fn bottom_right(scene: &mut Scene) {
    dropped(scene, (0.8, 0.1), NSPoint::ZERO, Corner::BottomRight);
}

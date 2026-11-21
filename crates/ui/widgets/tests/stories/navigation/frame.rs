//! The page frame: the page edge to edge, inset as a card (*Inset page*), the load glow along
//! the window's edge and around the card, and full screen dropping the inset. Renders show
//! the faint ring only: the comet turns on the render server, and the halo is a layer shadow,
//! which a render does not draw.

use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSImageScaling, NSImageView, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::motion;
use quickie_ui_shared::stories::{LIGHT, LIGHT_DARK, Scene, Story, page_picture};
use quickie_ui_widgets::PageFrame;

pub const STORIES: &[Story] = &[
    Story {
        group: "page",
        widget: "page-frame",
        state: "edge-to-edge",
        themes: LIGHT,
        window: WINDOW,
        scene: edge_to_edge,
    },
    Story {
        group: "page",
        widget: "page-frame",
        state: "inset",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: inset,
    },
    Story {
        group: "page",
        widget: "page-frame",
        state: "inset-glow",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: inset_loading,
    },
    Story {
        group: "page",
        widget: "page-frame",
        state: "edge-glow",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: edge_loading,
    },
    Story {
        group: "page",
        widget: "page-frame",
        state: "loaded",
        themes: LIGHT,
        window: WINDOW,
        scene: loaded,
    },
    Story {
        group: "page",
        widget: "page-frame",
        state: "inset-full-screen",
        themes: LIGHT,
        window: WINDOW,
        scene: inset_full_screen,
    },
];

/// A small window: the frame is about edges and corners.
const WINDOW: (f64, f64) = (640.0, 400.0);

/// The frame in the scene's window over the window's own background, a page picture in its
/// host as the engine would put a page there.
fn open(scene: &mut Scene) -> PageFrame {
    if let Some(layer) = scene.host().layer() {
        layer.setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
    }
    let frame = PageFrame::new(scene.mtm());
    frame.install(scene.host());
    let page = NSImageView::new(scene.mtm());
    page.setImageScaling(NSImageScaling::ScaleAxesIndependently);
    page.setImage(page_picture(640, 400, 0.58).as_deref());
    page.setFrame(frame.host().bounds());
    page.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    frame.host().addSubview(&page);
    let whole = scene.host().retain();
    scene.crop_to(&whole);
    frame
}

/// Where the card stands in the window.
fn card(frame: &PageFrame) -> NSRect {
    frame.view().frame()
}

fn edge_to_edge(scene: &mut Scene) {
    let frame = open(scene);
    scene.check(!frame.is_inset(), "the page is edge to edge by default");
    scene.check_eq(
        card(&frame),
        scene.host().bounds(),
        "the page fills the window",
    );
    scene.keep(frame);
}

/// An even inset at the sides and the bottom; a borderless window has no titlebar band, so the
/// top is the same.
fn inset(scene: &mut Scene) {
    let frame = open(scene);
    frame.set_inset(true);
    scene.check(frame.is_inset(), "the page is inset");
    scene.check_eq(
        card(&frame),
        NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(624.0, 384.0)),
        "the card keeps the inset",
    );
    let host: Retained<NSView> = frame.host().retain();
    scene.check_eq(
        host.frame().size,
        NSSize::new(624.0, 384.0),
        "the page fills the card",
    );
    // A window passing through a size smaller than the inset, as on the way into full screen.
    let whole = scene.host().bounds().size;
    scene.host().setFrameSize(NSSize::new(4.0, 4.0));
    scene.host().setFrameSize(whole);
    scene.check_eq(
        card(&frame),
        NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(624.0, 384.0)),
        "a resize through a tiny window keeps the inset",
    );
    scene.keep(frame);
}

/// The page loads through `steps`; the glow's speed at each, in turns per second.
fn load(scene: &mut Scene, frame: &PageFrame, steps: &[f64]) -> Vec<f64> {
    let speeds: Vec<f64> = steps
        .iter()
        .map(|&fraction| {
            frame.set_loading(Some(fraction));
            frame.glow_speed()
        })
        .collect();
    scene.check(frame.is_loading(), "a load in progress shows");
    if motion::reduce_motion() {
        scene.check(
            speeds.iter().all(|&speed| speed == 0.0),
            "with Reduce Motion the edge stands still",
        );
    } else {
        scene.check(
            speeds.windows(2).all(|pair| pair[0] < pair[1]),
            format!("the light speeds up as the page loads: {speeds:?}"),
        );
    }
    speeds
}

/// Inset, the glow runs around the card as its box shadow.
fn inset_loading(scene: &mut Scene) {
    let frame = open(scene);
    frame.set_inset(true);
    load(scene, &frame, &[0.1, 0.5, 0.9]);
    scene.check_eq(
        frame.glow_view().frame(),
        card(&frame),
        "the glow runs around the card",
    );
    scene.keep(frame);
}

/// Edge to edge, the glow runs along the window's inner edge, over the page.
fn edge_loading(scene: &mut Scene) {
    let frame = open(scene);
    load(scene, &frame, &[0.3]);
    scene.check_eq(
        frame.glow_view().frame(),
        scene.host().bounds(),
        "the glow runs along the window's edge",
    );
    scene.keep(frame);
}

/// When loading ends the glow fades out and stops; a new load brings it back.
fn loaded(scene: &mut Scene) {
    let frame = open(scene);
    frame.set_inset(true);
    load(scene, &frame, &[0.4]);
    frame.set_loading(None);
    scene.check(!frame.is_loading(), "a finished load stops showing");
    frame.reset_loading();
    scene.check_eq(frame.glow_speed(), 0.0, "a reset glow stands still");
    scene.keep(frame);
}

/// An element full screen drops the inset and gives it back when it ends. (The window's own
/// full screen keeps it; that comes through `AppKit`'s notifications, seen in the app.)
fn inset_full_screen(scene: &mut Scene) {
    let frame = open(scene);
    frame.set_inset(true);
    frame.set_page_full_screen(true);
    frame.set_page_full_screen(false);
    scene.check(
        frame.is_inset(),
        "leaving full screen brings the inset back",
    );
    frame.set_page_full_screen(true);
    scene.check(!frame.is_inset(), "a page full screen drops the inset");
    scene.check_eq(
        card(&frame),
        scene.host().bounds(),
        "the page fills the window",
    );
    scene.keep(frame);
}

//! The page frame: the page edge to edge, inset as a card (*Inset page*), the card loading,
//! and full screen dropping the inset.

use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSImageScaling, NSImageView, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::stories::{LIGHT, LIGHT_DARK, Scene, Story, page_picture};
use quickie_ui_widgets::{LoadProgressLine, PageFrame};

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
        state: "inset-loading",
        themes: LIGHT,
        window: WINDOW,
        scene: inset_loading,
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
    scene.keep(frame);
}

/// The load progress line runs along the card's top edge, inside its corners.
fn inset_loading(scene: &mut Scene) {
    let frame = open(scene);
    frame.set_inset(true);
    let line = LoadProgressLine::new(scene.mtm());
    line.set(frame.host(), Some(0.6));
    scene.check(line.is_loading(), "a load in progress shows");
    scene.keep(line);
    scene.keep(frame);
}

/// An element full screen drops the inset and gives it back when it ends. (The window's own
/// full screen does the same through `AppKit`'s notifications, seen in the app.)
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

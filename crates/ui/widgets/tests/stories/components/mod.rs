//! Stories of the shared controls every overlay is made of, on glass as overlays show them:
//! the keycap in each style (`docs/design/mocks/components/keycap.html`) and the hint strip
//! that closes an E2 overlay.

use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::stories::{LIGHT_DARK, Scene, Story, Theme};
use quickie_ui_shared::tokens::{color, radius, space};
use quickie_ui_shared::{
    Elevation, GlassSurface, HintStrip, KeycapStyle, Plate, keycap, keycap_group_styled,
};

pub const STORIES: &[Story] = &[
    Story {
        group: "components",
        widget: "keycap",
        state: "styles",
        themes: &Theme::ALL,
        window: (640.0, 320.0),
        scene: keycaps,
    },
    Story {
        group: "components",
        widget: "hint-bar",
        state: "strip",
        themes: LIGHT_DARK,
        window: (760.0, 200.0),
        scene: strip,
    },
];

/// A glass surface `size` big, centred in the window, kept alive by the scene.
fn surface(scene: &mut Scene, size: NSSize) -> Retained<NSView> {
    let surface = GlassSurface::new(Elevation::E2, scene.mtm());
    let host = scene.host().bounds().size;
    surface.set_frame(NSRect::new(
        NSPoint::new(
            ((host.width - size.width) / 2.0).round(),
            ((host.height - size.height) / 2.0).round(),
        ),
        size,
    ));
    scene.host().addSubview(surface.view());
    let content: Retained<NSView> = surface.content_view().retain();
    scene.crop_to(surface.view());
    scene.keep(surface);
    content
}

/// The keycap sheet's width.
const WIDTH: f64 = 540.0;

/// Each style on a line of its own, top down: plain caps, the caps of a selected row on its
/// tint, caps in conflict, and a pinned slot's badge.
fn keycaps(scene: &mut Scene) {
    let mtm = scene.mtm();
    let content = surface(scene, NSSize::new(WIDTH, 220.0));
    let lines: [(&[&str], KeycapStyle); 4] = [
        (
            &["⌘", "⇧", "⌥", "⌃", "⏎", "⎋", "⌘K", "⌘K ⌘S"],
            KeycapStyle::Plain,
        ),
        (&["⌘R", "⇧⌘P"], KeycapStyle::Selected),
        (&["⌥⌘I"], KeycapStyle::Conflict),
        (&[], KeycapStyle::Accent),
    ];
    let mut top = 220.0 - space::XL;
    for (chords, style) in lines {
        top -= 20.0;
        let mut x = space::XL;
        if style == KeycapStyle::Selected {
            let tint = Plate::new(
                Some(color::selection_fill),
                Some(color::accent_line),
                radius::ROW,
                mtm,
            );
            tint.setFrame(NSRect::new(
                NSPoint::new(space::LG, top - space::SM),
                NSSize::new(WIDTH - space::LG * 2.0, 20.0 + space::LG),
            ));
            content.addSubview(&tint);
            x += space::SM;
        }
        let caps: Vec<Retained<NSView>> = if style == KeycapStyle::Accent {
            ["⌘3", "⌘4"]
                .into_iter()
                .map(|slot| keycap(slot, style, mtm))
                .collect()
        } else {
            chords
                .iter()
                .map(|chord| keycap_group_styled(chord, style, mtm))
                .collect()
        };
        for group in caps {
            group.setFrameOrigin(NSPoint::new(x, top));
            x += group.frame().size.width + space::LG;
            content.addSubview(&group);
        }
        top -= space::XL + space::SM;
    }
    let accessibility = scene.accessibility();
    scene.check(
        accessibility.contains("label=\"⌘K ⌘S\""),
        "a chord reads as one element",
    );
}

/// The palette's strip: the prefixes with `>` active, `⏎ select` on the right.
fn strip(scene: &mut Scene) {
    let mtm = scene.mtm();
    let width = 720.0;
    let content = surface(scene, NSSize::new(width, HintStrip::HEIGHT + space::XL));
    let strip = HintStrip::new(
        &[
            (">", "commands"),
            ("@", "headings"),
            ("#", "tags"),
            ("?", "agent"),
            ("!", "search"),
        ],
        &[("↑↓", "move"), ("⏎", "select")],
        mtm,
    );
    strip.set_active(Some(0));
    strip.set_frame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, HintStrip::HEIGHT),
    ));
    content.addSubview(strip.view());
    let accessibility = scene.accessibility();
    scene.check(
        accessibility.contains("> commands, @ headings"),
        "the strip reads its hints",
    );
    scene.keep(strip);
}

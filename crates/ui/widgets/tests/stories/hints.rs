//! The hints group: link hints over a made-up page, as the keyboard narrows and follows them.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::Message;
use objc2_app_kit::{NSImageScaling, NSImageView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Scene, Story, Theme, page_picture};
use quickie_ui_widgets::{HintKey, LinkHintsLayer};

const fn hints(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "hints",
        widget: "link-hints-layer",
        state,
        themes,
        window: (720.0, 440.0),
        scene,
    }
}

pub const STORIES: &[Story] = &[
    hints("labels", LIGHT_DARK, labels),
    hints("narrowed", LIGHT, narrowed),
    hints("background", LIGHT, background),
    hints("followed", LIGHT, followed),
];

/// Twelve links down the page's lines of text, more than the home row has letters, so the
/// last ones get two-letter labels.
fn targets() -> Vec<NSRect> {
    (0..12_u32)
        .map(|index| {
            let column = f64::from(index % 2);
            let row = f64::from(index / 2);
            NSRect::new(
                NSPoint::new(60.0 + column * 300.0, 300.0 - row * 44.0),
                NSSize::new(180.0, 18.0),
            )
        })
        .collect()
}

/// The page, the hints over it, and the browser's routing of letters and `⌫` to them; what
/// the last letter did is kept.
fn open(scene: &mut Scene, background: bool) -> (Rc<LinkHintsLayer>, Rc<RefCell<Vec<HintKey>>>) {
    let page = NSImageView::new(scene.mtm());
    page.setImageScaling(NSImageScaling::ScaleAxesIndependently);
    page.setImage(page_picture(720, 440, 0.58).as_deref());
    page.setFrame(scene.host().bounds());
    scene.host().addSubview(&page);
    let layer = Rc::new(LinkHintsLayer::new(scene.mtm()));
    layer.show(scene.host(), &targets(), background);
    let typed = Rc::new(RefCell::new(Vec::new()));
    let (hints, log) = (Rc::clone(&layer), Rc::clone(&typed));
    scene.route(move |stroke| {
        match stroke.key {
            Key::Char(letter) => log.borrow_mut().push(hints.type_letter(letter)),
            Key::Backspace => hints.backspace(),
            _ => return false,
        }
        true
    });
    let whole = scene.host().retain();
    scene.crop_to(&whole);
    scene.keep(Rc::clone(&layer));
    (layer, typed)
}

fn labels(scene: &mut Scene) {
    let (layer, _) = open(scene, false);
    scene.check(layer.is_open(), "the hints show");
}

/// `l` starts the two-letter labels: the others go, and the typed `L` dims.
fn narrowed(scene: &mut Scene) {
    let (_, typed) = open(scene, false);
    scene.type_text("l");
    scene.check_eq(typed.borrow().clone(), vec![HintKey::Typed], "l narrows");
}

fn background(scene: &mut Scene) {
    let (layer, _) = open(scene, true);
    scene.check(layer.background(), "⇧F opens in a background tab");
}

/// `q` starts nothing; `l`, `⌫`, `l`, `a` follows the first two-letter label, and the hints go.
fn followed(scene: &mut Scene) {
    let (layer, typed) = open(scene, false);
    scene.type_text("q");
    scene.type_text("l");
    scene.press(Key::Backspace);
    scene.type_text("la");
    let followed = typed.borrow().last().copied();
    scene.check_eq(
        typed.borrow()[..3].to_vec(),
        vec![HintKey::Ignored, HintKey::Typed, HintKey::Typed],
        "q is ignored, l narrows, and narrows again after ⌫",
    );
    scene.check(
        matches!(followed, Some(HintKey::Follow(index)) if index >= 8),
        format!("la follows a two-letter target: {followed:?}"),
    );
    scene.check(!layer.is_open(), "following hides the hints");
}

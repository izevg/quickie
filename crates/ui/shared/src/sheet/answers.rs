//! A sheet's answers as buttons (`docs/design/mocks/system/permission/`): the plain answers in
//! pairs of equal width, then each filled answer, the default or the primary one, across the
//! whole sheet. Every button names its keys on itself, so the sheet needs no hint bar.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColor, NSEventModifierFlags, NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{Emphasis, SheetAnswer};
use crate::controls::keycap::caps;
use crate::controls::{KeycapStyle, keycap_group_styled, symbol};
use crate::form::{Paint, Plate};
use crate::tokens::{self, TextRole, color, radius, space};

/// Height of an answer button.
pub const BUTTON_HEIGHT: CGFloat = 36.0;

/// The answers laid out in rows, by index: the plain ones first, two to a row, then each
/// filled one alone. The indexes stay the answers' own, so a click answers what was asked.
#[must_use]
pub fn rows(emphases: &[Emphasis]) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let plain = emphases
        .iter()
        .enumerate()
        .filter(|(_, emphasis)| **emphasis == Emphasis::Plain)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    rows.extend(plain.chunks(2).map(<[usize]>::to_vec));
    rows.extend(
        emphases
            .iter()
            .enumerate()
            .filter(|(_, emphasis)| **emphasis != Emphasis::Plain)
            .map(|(index, _)| vec![index]),
    );
    rows
}

/// What a button is painted with.
struct Look {
    fill: Paint,
    text: Paint,
    filled: bool,
}

fn look(emphasis: Emphasis) -> Look {
    match emphasis {
        Emphasis::Plain => Look {
            fill: color::group_fill,
            text: color::label,
            filled: false,
        },
        Emphasis::Primary => Look {
            fill: color::accent,
            text: NSColor::whiteColor,
            filled: true,
        },
        Emphasis::Default => Look {
            fill: color::destructive,
            text: NSColor::whiteColor,
            filled: true,
        },
        Emphasis::Agent => Look {
            fill: color::agent,
            text: NSColor::whiteColor,
            filled: true,
        },
    }
}

/// A button that names its keys: `answer`'s label, symbol, and keys painted by its emphasis,
/// `width` wide or as wide as it needs. A click sends `action` to `target` with `tag` as the
/// sender's tag; `VoiceOver` reads the label, and the keys as its help.
///
/// # Safety
///
/// `target` must implement `action`, taking the sender, and outlive the button.
pub unsafe fn keyed_button(
    answer: &SheetAnswer,
    width: Option<CGFloat>,
    target: &AnyObject,
    action: Sel,
    tag: isize,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    // SAFETY: this function has the same target and action requirements.
    unsafe { keyed_button_with_control(answer, width, target, action, tag, mtm) }.0
}

unsafe fn keyed_button_with_control(
    answer: &SheetAnswer,
    width: Option<CGFloat>,
    target: &AnyObject,
    action: Sel,
    tag: isize,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Retained<NSButton>) {
    let look = look(answer.emphasis);
    let text = (look.text)();
    let title = NSTextField::labelWithString(&NSString::from_str(&answer.label), mtm);
    let font = if look.filled {
        semibold(TextRole::RowTitle)
    } else {
        TextRole::RowTitle.font()
    };
    title.setFont(Some(&font));
    title.setTextColor(Some(&text));
    title.sizeToFit();
    let keys = if look.filled {
        keys_on_fill(&answer.keys, mtm)
    } else {
        keycap_group_styled(&answer.keys, KeycapStyle::Plain, mtm)
    };
    let glyph = answer
        .symbol
        .filter(|_| look.filled)
        .and_then(|name| symbol(name, "", TextRole::RowTitle, &text, mtm));
    let note = (answer.emphasis == Emphasis::Default).then(|| {
        let note = NSTextField::labelWithString(&NSString::from_str("Default"), mtm);
        note.setFont(Some(&TextRole::Caption.mono_font()));
        note.setTextColor(Some(&text.colorWithAlphaComponent(0.8)));
        note.sizeToFit();
        note
    });
    let title_size = title.frame().size;
    let keys_size = keys.frame().size;
    let keys_room = if keys_size.width > 0.0 {
        keys_size.width + space::SM
    } else {
        0.0
    };
    let width = width.unwrap_or_else(|| {
        let glyph = glyph
            .as_ref()
            .map_or(0.0, |glyph| glyph.frame().size.width + space::SM);
        let note = note
            .as_ref()
            .map_or(0.0, |note| note.frame().size.width + space::SM);
        (space::MD * 2.0 + glyph + title_size.width + note + keys_room).ceil()
    });
    let plate = Plate::new(Some(look.fill), None, radius::KEYCAP + 2.0, mtm);
    plate.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, BUTTON_HEIGHT),
    ));
    // The plate is flipped: y runs down from its top edge.
    let middle = |height: CGFloat| ((BUTTON_HEIGHT - height) / 2.0).round();
    if look.filled {
        let mut left = space::MD;
        if let Some(glyph) = glyph {
            let size = glyph.frame().size;
            glyph.setFrameOrigin(NSPoint::new(left, middle(size.height)));
            plate.addSubview(&glyph);
            left += size.width + space::SM;
        }
        let mut right = width - space::MD - keys_size.width;
        keys.setFrameOrigin(NSPoint::new(right, middle(keys_size.height)));
        if let Some(note) = note {
            let size = note.frame().size;
            right -= size.width + space::SM;
            note.setFrameOrigin(NSPoint::new(right, middle(size.height)));
            plate.addSubview(&note);
        }
        title.setFrame(NSRect::new(
            NSPoint::new(left, middle(title_size.height)),
            NSSize::new((right - left - space::SM).max(0.0), title_size.height),
        ));
    } else {
        // Label and keys centered together, the label cut when the button is narrow.
        let room = (width - space::MD * 2.0 - keys_room).max(0.0);
        let title_width = title_size.width.min(room);
        let left = ((width - title_width - keys_room) / 2.0).round();
        title.setFrame(NSRect::new(
            NSPoint::new(left, middle(title_size.height)),
            NSSize::new(title_width, title_size.height),
        ));
        keys.setFrameOrigin(NSPoint::new(
            left + title_width + space::SM,
            middle(keys_size.height),
        ));
    }
    plate.addSubview(&title);
    plate.addSubview(&keys);
    // SAFETY: the caller promises `target` implements `action` and outlives the button.
    let click = unsafe { click_target(answer, plate.bounds(), target, action, mtm) };
    click.setTag(tag);
    plate.addSubview(&click);
    (plate.into_super(), click)
}

/// One answer of a sheet as a button `width` wide; a click sends `answer:` to `target` with
/// the answer's index as the sender's tag.
pub fn button(
    answer: &SheetAnswer,
    index: usize,
    width: CGFloat,
    target: &AnyObject,
    mtm: MainThreadMarker,
    native_keys: bool,
) -> Retained<NSView> {
    let tag = isize::try_from(index).unwrap_or(isize::MAX);
    // SAFETY: the sheet's target implements `answer:`, which takes a sender, and the sheet
    // keeps it alive for as long as its buttons.
    let (view, control) =
        unsafe { keyed_button_with_control(answer, Some(width), target, sel!(answer:), tag, mtm) };
    if native_keys {
        let key = match answer.keys.as_str() {
            "⏎" => "\r",
            "⎋" => "\u{1b}",
            _ => "",
        };
        control.setKeyEquivalent(&NSString::from_str(key));
        control.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
    }
    view
}

/// A transparent button over the whole answer: the click, and what `VoiceOver` reads.
///
/// # Safety
///
/// `target` must implement `action`, taking the sender, and outlive the button.
unsafe fn click_target(
    answer: &SheetAnswer,
    frame: NSRect,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let label = NSString::from_str(&answer.label);
    // SAFETY: the caller promises `target` implements `action` and outlives the button.
    let button =
        unsafe { NSButton::buttonWithTitle_target_action(&label, Some(target), Some(action), mtm) };
    button.setBordered(false);
    button.setTransparent(true);
    button.setRefusesFirstResponder(true);
    button.setAccessibilityLabel(Some(&label));
    if !answer.keys.is_empty() {
        button.setAccessibilityHelp(Some(&NSString::from_str(&format!("Keys: {}", answer.keys))));
    }
    button.setFrame(frame);
    button
}

/// The keys of a filled button: caps of white glyphs on a darker fill, so they read on red
/// and on the accent alike.
fn keys_on_fill(chord: &str, mtm: MainThreadMarker) -> Retained<NSView> {
    let group = NSView::new(mtm);
    let mut x: CGFloat = 0.0;
    let mut height: CGFloat = 0.0;
    for (index, stroke) in caps(chord).iter().enumerate() {
        if index > 0 {
            x += space::SM - space::XS;
        }
        for glyph in stroke {
            let label = NSTextField::labelWithString(&NSString::from_str(glyph), mtm);
            label.setFont(Some(&TextRole::Caption.mono_font()));
            label.setAlignment(NSTextAlignment::Center);
            label.setTextColor(Some(&NSColor::whiteColor()));
            label.sizeToFit();
            let text = label.frame().size;
            let cap_height = (text.height + space::XS).ceil();
            let cap_width = (text.width + space::SM).max(cap_height).ceil();
            let cap = Plate::new(
                Some(|| tokens::black(0.2)),
                Some(|| tokens::white(0.25)),
                radius::KEYCAP,
                mtm,
            );
            cap.setFrame(NSRect::new(
                NSPoint::new(x, 0.0),
                NSSize::new(cap_width, cap_height),
            ));
            label.setFrame(NSRect::new(
                NSPoint::new(0.0, ((cap_height - text.height) / 2.0).ceil()),
                NSSize::new(cap_width, text.height),
            ));
            cap.addSubview(&label);
            group.addSubview(&cap);
            x += cap_width + space::XS;
            height = height.max(cap_height);
        }
    }
    group.setFrameSize(NSSize::new((x - space::XS).max(0.0), height));
    group
}

fn semibold(role: TextRole) -> Retained<objc2_app_kit::NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let weight = unsafe { objc2_app_kit::NSFontWeightSemibold };
    objc2_app_kit::NSFont::systemFontOfSize_weight(role.point_size(), weight)
}

#[cfg(test)]
mod tests {
    use super::{Emphasis, rows};

    #[test]
    fn plain_answers_pair_up_and_filled_ones_come_last_alone() {
        let permission = [Emphasis::Plain, Emphasis::Plain, Emphasis::Default];
        assert_eq!(rows(&permission), [vec![0, 1], vec![2]]);

        let dialog = [Emphasis::Primary, Emphasis::Plain, Emphasis::Plain];
        assert_eq!(rows(&dialog), [vec![1, 2], vec![0]]);
    }

    #[test]
    fn an_odd_plain_answer_takes_a_row_of_its_own() {
        assert_eq!(
            rows(&[Emphasis::Plain, Emphasis::Default]),
            [vec![0], vec![1]]
        );
        assert_eq!(
            rows(&[Emphasis::Plain, Emphasis::Plain, Emphasis::Plain]),
            [vec![0, 1], vec![2]]
        );
        assert_eq!(rows(&[Emphasis::Primary]), [vec![0]]);
        assert_eq!(rows(&[]), Vec::<Vec<usize>>::new());
    }
}

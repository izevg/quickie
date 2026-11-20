//! The tinted icon tile that says what a sheet is about (a camera, an extension), and the card
//! under its text that says what the system does about it.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::controls::symbol;
use crate::form::{Paint, Plate};
use crate::tokens::{TextRole, color, radius, space};

/// The icon tile's side.
pub const TILE: CGFloat = 40.0;

/// What heads a sheet beside its text: an SF Symbol on a tile of its tint.
#[derive(Clone, Copy, Debug)]
pub struct SheetIcon {
    /// The SF Symbol: `video.fill`, `puzzlepiece.extension.fill`.
    pub symbol: &'static str,
    /// One of [`crate::tokens::color`]: the destructive red for a device, the accent for an
    /// extension.
    pub tint: Paint,
    /// The tint at [`color::TINT_ALPHA`], for the tile's fill.
    pub fill: Paint,
}

/// A card under the sheet's text: a symbol and a line, and a detail in SF Mono.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetCard {
    pub symbol: &'static str,
    pub title: String,
    pub detail: String,
}

/// The icon as a [`TILE`]-sized tile.
pub fn tile(icon: &SheetIcon, mtm: MainThreadMarker) -> Retained<NSView> {
    let plate = Plate::new(Some(icon.fill), None, radius::ROW, mtm);
    plate.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(TILE, TILE)));
    if let Some(glyph) = symbol(icon.symbol, "", TextRole::Title, &(icon.tint)(), mtm) {
        let size = glyph.frame().size;
        glyph.setFrameOrigin(NSPoint::new(
            ((TILE - size.width) / 2.0).round(),
            ((TILE - size.height) / 2.0).round(),
        ));
        plate.addSubview(&glyph);
    }
    plate.into_super()
}

/// The card `width` wide, sized to its text.
pub fn card(card: &SheetCard, width: CGFloat, mtm: MainThreadMarker) -> Retained<NSView> {
    let plate = Plate::new(
        Some(color::group_fill),
        Some(color::separator),
        radius::ROW,
        mtm,
    );
    let inner = width - space::MD * 2.0;
    let mut y = space::MD;
    let mut left = space::MD;
    let title = NSTextField::wrappingLabelWithString(&NSString::from_str(&card.title), mtm);
    title.setFont(Some(&TextRole::RowDetail.font()));
    title.setTextColor(Some(&color::label()));
    title.setSelectable(false);
    if let Some(glyph) = symbol(card.symbol, "", TextRole::RowDetail, &color::success(), mtm) {
        let size = glyph.frame().size;
        let line = title.fittingSize().height;
        glyph.setFrameOrigin(NSPoint::new(left, y + ((line - size.height) / 2.0).round()));
        plate.addSubview(&glyph);
        left += size.width + space::SM;
    }
    let title_width = width - left - space::MD;
    title.setPreferredMaxLayoutWidth(title_width);
    let title_height = title.fittingSize().height.ceil();
    title.setFrame(NSRect::new(
        NSPoint::new(left, y),
        NSSize::new(title_width, title_height),
    ));
    plate.addSubview(&title);
    y += title_height + space::XS;
    let detail = NSTextField::wrappingLabelWithString(&NSString::from_str(&card.detail), mtm);
    detail.setFont(Some(&TextRole::Caption.mono_font()));
    detail.setTextColor(Some(&color::secondary_label()));
    detail.setSelectable(false);
    let detail_width = inner - (left - space::MD);
    detail.setPreferredMaxLayoutWidth(detail_width);
    let detail_height = detail.fittingSize().height.ceil();
    detail.setFrame(NSRect::new(
        NSPoint::new(left, y),
        NSSize::new(detail_width, detail_height),
    ));
    plate.addSubview(&detail);
    y += detail_height + space::MD;
    plate.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, y.ceil()),
    ));
    plate.into_super()
}

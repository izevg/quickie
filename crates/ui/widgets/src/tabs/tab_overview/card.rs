//! One tab in the overview: its title and pinned slot above, the page's last picture in
//! 16:10, its host below. A tab whose page is not loaded has no picture: its host stands in
//! the picture's place. The selected card wears the accent ring.
//!
//! The picture is an `NSImageView`, `AppKit`'s own way to draw an image, not contents set on
//! the layer of a layer-backed view, which `AppKit` owns and manages.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::kCACornerCurveContinuous;
use quickie_ui_shared::keycap_group;
use quickie_ui_shared::tokens::{TextRole, color, radius, space};

use super::OverviewTab;

/// The ring of the selected card, and the hairline of the others.
const RING: CGFloat = 3.0;
const HAIRLINE: CGFloat = 1.0;

pub struct Card {
    root: Retained<NSView>,
    title: Retained<NSTextField>,
    host: Retained<NSTextField>,
    picture: Retained<NSView>,
    image: Retained<NSImageView>,
    stand_in: Retained<NSTextField>,
    slot: Option<Retained<NSView>>,
}

/// A card's height besides its picture: the title row and the host row.
#[must_use]
pub fn chrome() -> CGFloat {
    title_row() + host_row()
}

fn title_row() -> CGFloat {
    (line(TextRole::RowTitle) + space::SM * 2.0).ceil()
}

fn host_row() -> CGFloat {
    (line(TextRole::RowDetail) + space::SM * 2.0).ceil()
}

fn line(role: TextRole) -> CGFloat {
    role.font().boundingRectForFont().size.height.ceil()
}

impl Card {
    #[must_use]
    pub fn new(tab: &OverviewTab, mtm: MainThreadMarker) -> Self {
        let root = NSView::new(mtm);
        root.setWantsLayer(true);
        if let Some(layer) = root.layer() {
            layer.setCornerRadius(radius::CARD);
            // SAFETY: an immutable string exported by Core Animation.
            layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
            layer.setMasksToBounds(true);
        }
        let title = label(&tab.title, TextRole::RowTitle, mtm);
        let host = label(&tab.host, TextRole::RowDetail, mtm);
        host.setFont(Some(&TextRole::RowDetail.mono_font()));
        host.setTextColor(Some(&color::secondary_label()));
        let picture = NSView::new(mtm);
        picture.setWantsLayer(true);
        if let Some(layer) = picture.layer() {
            layer.setBackgroundColor(Some(&color::separator().CGColor()));
            layer.setMasksToBounds(true);
        }
        // The engine draws pictures in the card's shape, so filling the frame keeps the page's
        // proportions.
        let image = NSImageView::new(mtm);
        image.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        image.setEditable(false);
        let stand_in = label(&tab.host, TextRole::RowDetail, mtm);
        stand_in.setAlignment(NSTextAlignment::Center);
        stand_in.setTextColor(Some(&color::secondary_label()));
        picture.addSubview(&image);
        picture.addSubview(&stand_in);
        let slot = tab
            .slot
            .map(|slot| keycap_group(&format!("⌘{slot}"), false, mtm));
        for view in [&*title, &*picture, &*host] {
            root.addSubview(view);
        }
        if let Some(slot) = &slot {
            root.addSubview(slot);
        }
        let card = Self {
            root,
            title,
            host,
            picture,
            image,
            stand_in,
            slot,
        };
        card.set_picture(tab.thumbnail.as_deref());
        card.set_selected(false);
        card
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.root
    }

    /// Shows `image`, or the host in its place.
    pub fn set_picture(&self, image: Option<&NSImage>) {
        self.image.setImage(image);
        self.image.setHidden(image.is_none());
        self.stand_in.setHidden(image.is_some());
    }

    pub fn set_selected(&self, selected: bool) {
        if let Some(layer) = self.root.layer() {
            let (ring, width) = if selected {
                (color::accent(), RING)
            } else {
                (color::separator(), HAIRLINE)
            };
            layer.setBorderColor(Some(&ring.CGColor()));
            layer.setBorderWidth(width);
        }
    }

    /// Places the card at `frame` and lays out its rows.
    pub fn set_frame(&self, frame: NSRect) {
        self.root.setFrame(frame);
        let NSSize { width, height } = frame.size;
        let (top, bottom) = (title_row(), host_row());
        let inset = space::MD;
        let slot_width = self.slot.as_ref().map_or(0.0, |slot| {
            let size = slot.frame().size;
            slot.setFrameOrigin(NSPoint::new(
                width - inset - size.width,
                height - top + ((top - size.height) / 2.0).floor(),
            ));
            size.width + space::SM
        });
        let title_line = line(TextRole::RowTitle);
        self.title.setFrame(NSRect::new(
            NSPoint::new(inset, height - top + ((top - title_line) / 2.0).floor()),
            NSSize::new((width - inset * 2.0 - slot_width).max(0.0), title_line),
        ));
        let host_line = line(TextRole::RowDetail);
        self.host.setFrame(NSRect::new(
            NSPoint::new(inset, ((bottom - host_line) / 2.0).floor()),
            NSSize::new((width - inset * 2.0).max(0.0), host_line),
        ));
        let picture = NSRect::new(
            NSPoint::new(0.0, bottom),
            NSSize::new(width, (height - top - bottom).max(0.0)),
        );
        self.picture.setFrame(picture);
        self.image
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), picture.size));
        self.stand_in.setFrame(NSRect::new(
            NSPoint::new(inset, ((picture.size.height - host_line) / 2.0).floor()),
            NSSize::new((width - inset * 2.0).max(0.0), host_line),
        ));
    }
}

fn label(text: &str, role: TextRole, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&role.font()));
    label.setTextColor(Some(&color::label()));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

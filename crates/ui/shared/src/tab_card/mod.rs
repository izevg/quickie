//! One tab as a card with the page's last picture in 16:10, in two looks:
//!
//! - [`Look::Grid`], the overview's: a card with corners of `radius-lg`, its favicon, title,
//!   close button and pinned slot above the picture, its host below. The selected card wears
//!   the accent ring and shows the close button.
//! - [`Look::Strip`], the switcher's: the picture alone with corners of `radius-md`, title and
//!   host under it. The selected card wears an accent ring and halo, the `ACTIVE` badge, and
//!   its title in the accent.
//!
//! A tab whose page is not loaded has no picture: its host stands in the picture's place. Cards
//! are pooled: a widget makes as many as it shows and refills them with [`Card::show`].
//!
//! The picture is an `NSImageView`, `AppKit`'s own way to draw an image, not contents set on
//! the layer of a layer-backed view, which `AppKit` owns and manages.
//!
//! The switcher and the overview both show tabs this way, so the card lives here, below both:
//!
//! - [`badge`]: the chips of the cards, which also count tabs in the widgets' headers.
//! - [`filter`]: which cards typed words keep, and how a header counts them.

pub mod badge;
pub mod filter;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAccessibility, NSFont, NSImage, NSImageScaling, NSImageView, NSLineBreakMode,
    NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::kCACornerCurveContinuous;

use crate::controls::symbol;
use crate::form::Plate;
use crate::tokens::{TextRole, color, overlay, radius, space};
pub use badge::Badge;

/// One tab as a card shows it.
#[derive(Clone, Debug, Default)]
pub struct TabCard {
    pub title: String,
    pub host: String,
    /// The pinned slot, `1` … `9`.
    pub slot: Option<u8>,
    /// The page's last picture; `None` for a tab not loaded or not drawn yet.
    pub thumbnail: Option<Retained<NSImage>>,
    /// The site's icon, when the engine has one.
    pub favicon: Option<Retained<NSImage>>,
}

/// Which widget the card is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// The overview's grid.
    Grid,
    /// The switcher's strip.
    Strip,
}

/// The ring of the selected grid card, and the hairline of the others.
const RING: CGFloat = 3.0;
const HAIRLINE: CGFloat = 1.0;
/// The halo around the selected strip picture: a strip card is this much wider than its
/// picture on each side.
pub const HALO: CGFloat = 4.0;
/// A favicon's side.
const FAVICON: CGFloat = 16.0;

pub struct Card {
    look: Look,
    root: Retained<NSView>,
    title: Retained<NSTextField>,
    host: Retained<NSTextField>,
    favicon: Retained<NSImageView>,
    /// The strip's halo; never shown in the grid.
    halo: Retained<Plate>,
    picture: Retained<Plate>,
    image: Retained<NSImageView>,
    stand_in: Retained<NSTextField>,
    /// The pinned slot in the grid, `ACTIVE` in the strip.
    badge: Badge,
    close: Option<Retained<NSImageView>>,
    has_favicon: std::cell::Cell<bool>,
    selected: std::cell::Cell<bool>,
}

/// A grid card's height besides its picture: the title row and the host row.
#[must_use]
pub fn chrome() -> CGFloat {
    title_row() + host_row()
}

/// A strip card's size for a picture `width` wide: the halo's room around the picture, and
/// the title and host under it.
#[must_use]
pub fn strip_size(width: CGFloat) -> NSSize {
    NSSize::new(
        width + HALO * 2.0,
        HALO + picture_height(width) + space::SM + strip_labels(),
    )
}

fn picture_height(width: CGFloat) -> CGFloat {
    (width * overlay::THUMBNAIL_SHAPE).round()
}

fn strip_labels() -> CGFloat {
    line(TextRole::RowDetail) + line(TextRole::Caption) + HALO
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
    pub fn new(look: Look, mtm: MainThreadMarker) -> Self {
        let root = NSView::new(mtm);
        root.setWantsLayer(true);
        if look == Look::Grid
            && let Some(layer) = root.layer()
        {
            layer.setCornerRadius(radius::CARD);
            // SAFETY: an immutable string exported by Core Animation.
            layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
            layer.setMasksToBounds(true);
        }
        let (title_role, host_role) = match look {
            Look::Grid => (TextRole::RowTitle, TextRole::RowDetail),
            Look::Strip => (TextRole::RowDetail, TextRole::Caption),
        };
        let title = label(&title_role.font(), mtm);
        let host = label(&host_role.mono_font(), mtm);
        host.setTextColor(Some(&color::secondary_label()));
        let favicon = NSImageView::new(mtm);
        favicon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        favicon.setHidden(true);
        let halo = Plate::new(Some(color::selection_fill), None, radius::ROW + HALO, mtm);
        halo.setHidden(true);
        let corner = match look {
            Look::Grid => 0.0,
            Look::Strip => radius::ROW,
        };
        let border = (look == Look::Strip).then_some(color::separator as fn() -> _);
        let picture = Plate::new(Some(color::separator), border, corner, mtm);
        if let Some(layer) = picture.layer() {
            layer.setMasksToBounds(true);
        }
        // The engine draws pictures in the card's shape, so filling the frame keeps the page's
        // proportions.
        let image = NSImageView::new(mtm);
        image.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        image.setEditable(false);
        let stand_in = label(&TextRole::RowDetail.font(), mtm);
        stand_in.setAlignment(NSTextAlignment::Center);
        stand_in.setTextColor(Some(&color::secondary_label()));
        picture.addSubview(&image);
        picture.addSubview(&stand_in);
        let badge = Badge::filled(mtm);
        let close = (look == Look::Grid)
            .then(|| {
                symbol(
                    "xmark.circle.fill",
                    "Close tab",
                    TextRole::RowTitle,
                    &color::destructive(),
                    mtm,
                )
            })
            .flatten();
        let parts: [&NSView; 5] = [&halo, &picture, &title, &host, &favicon];
        for view in parts {
            root.addSubview(view);
        }
        match look {
            Look::Grid => root.addSubview(badge.view()),
            Look::Strip => picture.addSubview(badge.view()),
        }
        if let Some(close) = &close {
            close.setAccessibilityLabel(Some(&NSString::from_str("Close tab")));
            close.setHidden(true);
            root.addSubview(close);
        }
        Self {
            look,
            root,
            title,
            host,
            favicon,
            halo,
            picture,
            image,
            stand_in,
            badge,
            close,
            has_favicon: std::cell::Cell::new(false),
            selected: std::cell::Cell::new(false),
        }
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.root
    }

    /// Fills the card with `tab`, unselected.
    pub fn show(&self, tab: &TabCard) {
        self.title.setStringValue(&NSString::from_str(&tab.title));
        self.host.setStringValue(&NSString::from_str(&tab.host));
        self.stand_in.setStringValue(&NSString::from_str(&tab.host));
        self.favicon.setImage(tab.favicon.as_deref());
        self.favicon.setHidden(tab.favicon.is_none());
        self.has_favicon.set(tab.favicon.is_some());
        if self.look == Look::Grid {
            match tab.slot {
                Some(slot) => self.badge.set_text(&format!("⌘{slot}")),
                None => self.badge.set_text(""),
            }
        }
        self.root
            .setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{}, {}",
                tab.title, tab.host
            ))));
        self.set_picture(tab.thumbnail.as_deref());
        self.set_selected(false);
    }

    /// Shows `image`, or the host in its place.
    pub fn set_picture(&self, image: Option<&NSImage>) {
        self.image.setImage(image);
        self.image.setHidden(image.is_none());
        self.stand_in.setHidden(image.is_some());
    }

    pub fn set_selected(&self, selected: bool) {
        self.selected.set(selected);
        let title = if selected {
            color::accent()
        } else {
            color::label()
        };
        match self.look {
            Look::Grid => {
                if let Some(layer) = self.root.layer() {
                    let (ring, width) = if selected {
                        (color::accent(), RING)
                    } else {
                        (color::separator(), HAIRLINE)
                    };
                    layer.setBorderColor(Some(&ring.CGColor()));
                    layer.setBorderWidth(width);
                }
                if let Some(close) = &self.close {
                    close.setHidden(!selected);
                }
                self.title.setTextColor(Some(&title));
            }
            Look::Strip => {
                self.halo.setHidden(!selected);
                let border = if selected {
                    color::accent
                } else {
                    color::separator
                };
                self.picture.set_paint(Some(color::separator), Some(border));
                self.badge.set_text(if selected { "ACTIVE" } else { "" });
                self.title.setTextColor(Some(&title));
                self.title
                    .setFont(Some(&weighted(TextRole::RowDetail, selected)));
            }
        }
    }

    /// True when `point`, in the card's superview, is on the selected card's close button.
    #[must_use]
    pub fn closes_at(&self, point: NSPoint) -> bool {
        let Some(close) = self.close.as_ref().filter(|_| self.selected.get()) else {
            return false;
        };
        let card = self.root.frame().origin;
        let frame = close.frame();
        let (x, y) = (point.x - card.x, point.y - card.y);
        x >= frame.origin.x
            && x <= frame.origin.x + frame.size.width
            && y >= frame.origin.y
            && y <= frame.origin.y + frame.size.height
    }

    /// Places the card at `frame` and lays out its parts.
    pub fn set_frame(&self, frame: NSRect) {
        self.root.setFrame(frame);
        match self.look {
            Look::Grid => self.lay_out_grid(frame.size),
            Look::Strip => self.lay_out_strip(frame.size),
        }
    }

    fn lay_out_grid(&self, size: NSSize) {
        let NSSize { width, height } = size;
        let (top, bottom) = (title_row(), host_row());
        let inset = space::MD;
        let middle = |item: CGFloat| height - top + ((top - item) / 2.0).floor();
        let mut right = width - inset;
        let close = self.close.as_deref().map(|close| -> &NSView { close });
        for view in [Some(self.badge.view()), close].into_iter().flatten() {
            if view.isHidden() {
                continue;
            }
            let item = view.frame().size;
            right -= item.width;
            view.setFrameOrigin(NSPoint::new(right, middle(item.height)));
            right -= space::SM;
        }
        let mut left = inset;
        if self.has_favicon.get() {
            self.favicon.setFrame(NSRect::new(
                NSPoint::new(left, middle(FAVICON)),
                NSSize::new(FAVICON, FAVICON),
            ));
            left += FAVICON + space::SM;
        }
        let title_line = line(TextRole::RowTitle);
        self.title.setFrame(NSRect::new(
            NSPoint::new(left, middle(title_line)),
            NSSize::new((right - left).max(0.0), title_line),
        ));
        let host_line = line(TextRole::RowDetail);
        self.host.setFrame(NSRect::new(
            NSPoint::new(inset, ((bottom - host_line) / 2.0).floor()),
            NSSize::new((width - inset * 2.0).max(0.0), host_line),
        ));
        self.place_picture(NSRect::new(
            NSPoint::new(0.0, bottom),
            NSSize::new(width, (height - top - bottom).max(0.0)),
        ));
    }

    fn lay_out_strip(&self, size: NSSize) {
        let width = size.width - HALO * 2.0;
        let picture = picture_height(width);
        let top = size.height - HALO - picture;
        self.halo.setFrame(NSRect::new(
            NSPoint::new(0.0, top - HALO),
            NSSize::new(size.width, picture + HALO * 2.0),
        ));
        self.place_picture(NSRect::new(
            NSPoint::new(HALO, top),
            NSSize::new(width, picture),
        ));
        let badge = self.badge.view().frame().size;
        // The picture is a flipped plate: the badge's origin is its top left.
        self.badge.view().setFrameOrigin(NSPoint::new(
            width - space::SM + space::XS - badge.width,
            space::SM - space::XS,
        ));
        let title_line = line(TextRole::RowDetail);
        let host_line = line(TextRole::Caption);
        let title_top = top - space::SM - title_line;
        let mut left = HALO;
        if self.has_favicon.get() {
            let icon = title_line.min(FAVICON);
            self.favicon.setFrame(NSRect::new(
                NSPoint::new(left, title_top + ((title_line - icon) / 2.0).floor()),
                NSSize::new(icon, icon),
            ));
            left += icon + space::XS;
        }
        self.title.setFrame(NSRect::new(
            NSPoint::new(left, title_top),
            NSSize::new((HALO + width - left).max(0.0), title_line),
        ));
        self.host.setFrame(NSRect::new(
            NSPoint::new(HALO, title_top - host_line),
            NSSize::new(width, host_line),
        ));
    }

    fn place_picture(&self, frame: NSRect) {
        self.picture.setFrame(frame);
        let size = frame.size;
        self.image
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        let host_line = line(TextRole::RowDetail);
        let inset = space::MD;
        self.stand_in.setFrame(NSRect::new(
            NSPoint::new(inset, ((size.height - host_line) / 2.0).floor()),
            NSSize::new((size.width - inset * 2.0).max(0.0), host_line),
        ));
    }
}

/// `role`'s font, semibold when `strong`.
#[must_use]
pub fn weighted(role: TextRole, strong: bool) -> Retained<NSFont> {
    if !strong {
        return role.font();
    }
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::systemFontOfSize_weight(role.point_size(), semibold)
}

fn label(font: &NSFont, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::new(), mtm);
    label.setFont(Some(font));
    label.setTextColor(Some(&color::label()));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

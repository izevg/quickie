//! A stat tile (`docs/design/mocks/system/shields/`): a measured number, large, over what it
//! counts. A number the engine did not measure shows as `—`, never as a zero.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAccessibility, NSColor, NSTextAlignment, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::Plate;
use quickie_ui_shared::tokens::{TextRole, color, radius, space};

/// A tile's height.
pub(crate) const TILE_HEIGHT: CGFloat = 60.0;

pub(crate) struct StatTile {
    plate: Retained<Plate>,
    number: Retained<NSTextField>,
    caption: Retained<NSTextField>,
}

impl StatTile {
    pub(crate) fn new(mtm: MainThreadMarker) -> Self {
        let plate = Plate::new(
            Some(color::group_fill),
            Some(color::separator),
            radius::ROW,
            mtm,
        );
        let number = NSTextField::labelWithString(&NSString::from_str("—"), mtm);
        number.setFont(Some(&semibold(TextRole::Title)));
        number.setAlignment(NSTextAlignment::Center);
        number.sizeToFit();
        let caption = NSTextField::labelWithString(&NSString::from_str(" "), mtm);
        caption.setFont(Some(&TextRole::Caption.font()));
        caption.setTextColor(Some(&color::secondary_label()));
        caption.setAlignment(NSTextAlignment::Center);
        caption.sizeToFit();
        plate.addSubview(&number);
        plate.addSubview(&caption);
        Self {
            plate,
            number,
            caption,
        }
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.plate
    }

    /// Shows `count` (`None`: not measured) over `caption`, the number in `tint`.
    pub(crate) fn set(&self, count: Option<u32>, caption: &str, tint: &NSColor) {
        let shown = shown(count);
        self.number.setStringValue(&NSString::from_str(&shown));
        self.number.setTextColor(Some(tint));
        self.caption.setStringValue(&NSString::from_str(caption));
        self.plate.setAccessibilityElement(true);
        self.plate
            .setAccessibilityLabel(Some(&NSString::from_str(&format!("{caption}: {shown}"))));
    }

    /// Places the tile at `origin`, `width` wide.
    pub(crate) fn place(&self, origin: NSPoint, width: CGFloat) {
        self.plate
            .setFrame(NSRect::new(origin, NSSize::new(width, TILE_HEIGHT)));
        let number = self.number.frame().size.height;
        let caption = self.caption.frame().size.height;
        // The plate is flipped: y runs down from its top edge.
        let top = ((TILE_HEIGHT - number - caption) / 2.0).round();
        self.number.setFrame(NSRect::new(
            NSPoint::new(space::XS, top),
            NSSize::new(width - space::XS * 2.0, number),
        ));
        self.caption.setFrame(NSRect::new(
            NSPoint::new(space::XS, top + number),
            NSSize::new(width - space::XS * 2.0, caption),
        ));
    }
}

/// The number as the tile shows it: measured, or `—` when it was not.
fn shown(count: Option<u32>) -> String {
    count.map_or_else(|| "—".to_owned(), |count| count.to_string())
}

fn semibold(role: TextRole) -> Retained<objc2_app_kit::NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let weight = unsafe { objc2_app_kit::NSFontWeightSemibold };
    objc2_app_kit::NSFont::systemFontOfSize_weight(role.point_size(), weight)
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn an_unmeasured_count_shows_a_dash_not_a_zero() {
        assert_eq!(shown(None), "—");
        assert_eq!(shown(Some(0)), "0");
        assert_eq!(shown(Some(12)), "12");
    }
}

//! Design tokens from `docs/design/system.md`: spacing, shape, semantic colors, typography.
//! Nothing outside this module hard-codes one of these values.

use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAccessibilityHighContrastAqua,
    NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua, NSColor, NSFont, NSFontTextStyle, NSFontTextStyleBody,
    NSFontTextStyleCaption1, NSFontTextStyleHeadline, NSFontTextStyleSubheadline,
    NSFontTextStyleTitle2, NSFontTextStyleTitle3, NSFontWeightRegular, NSView, NSWorkspace,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSArray, NSDictionary};

/// Spacing scale; no other values are used for gaps and insets.
pub mod space {
    use objc2_core_foundation::CGFloat;

    pub const XS: CGFloat = 4.0;
    pub const SM: CGFloat = 8.0;
    pub const MD: CGFloat = 12.0;
    pub const LG: CGFloat = 16.0;
    pub const XL: CGFloat = 24.0;
    /// Between groups in settings and sheets.
    pub const SECTION: CGFloat = 32.0;
    /// Inner padding of E2 overlays.
    pub const PALETTE_PADDING: CGFloat = 20.0;
}

/// Corner radii, drawn with continuous corners.
pub mod radius {
    use objc2_core_foundation::CGFloat;

    pub const KEYCAP: CGFloat = 6.0;
    pub const ROW: CGFloat = 10.0;
    pub const CARD: CGFloat = 16.0;
    /// E2 and E3 surfaces.
    pub const OVERLAY: CGFloat = 22.0;
}

/// Sizes of the small controls: keycaps, row icons, list rows.
pub mod control {
    use objc2_core_foundation::CGFloat;

    /// A keycap's height; a cap is at least as wide.
    pub const KEYCAP_HEIGHT: CGFloat = 20.0;
    /// A symbol before a one-line row's title.
    pub const ICON: CGFloat = 16.0;
    /// The tile a page row's favicon or glyph sits on.
    pub const TILE: CGFloat = 24.0;
    /// A one-line row: a command, a tag, a skill.
    pub const ROW_HEIGHT: CGFloat = 40.0;
}

/// Overlay widths and placement from the mockups.
pub mod overlay {
    use objc2_core_foundation::CGFloat;

    pub const ADDRESS_WIDTH: CGFloat = 640.0;
    pub const PALETTE_WIDTH: CGFloat = 720.0;
    /// Popovers: shields, downloads.
    pub const POPOVER_WIDTH: CGFloat = 360.0;
    /// E3 sheets: permission requests and page dialogs.
    pub const SHEET_WIDTH: CGFloat = 440.0;
    /// The agent's answer panel at its widest. It docks at the right and takes at most
    /// [`ANSWER_PANEL_SHARE`] of the window, so a cited passage stays in view beside it.
    pub const ANSWER_PANEL_WIDTH: CGFloat = 760.0;
    pub const ANSWER_PANEL_SHARE: CGFloat = 0.4;
    /// The answer panel at its tallest.
    pub const ANSWER_PANEL_MAX_HEIGHT: CGFloat = 640.0;
    /// Palettes sit this fraction of the window height below its top edge.
    pub const PALETTE_TOP: CGFloat = 0.15;
    /// The address overlay sits a little lower than the palettes.
    pub const ADDRESS_TOP: CGFloat = 0.20;
    /// The tab overview at its widest, and the narrowest it gets for its header and hints.
    pub const OVERVIEW_MAX_WIDTH: CGFloat = 1360.0;
    pub const OVERVIEW_MIN_WIDTH: CGFloat = 480.0;
    /// The share of the window's height the overview may take.
    pub const OVERVIEW_MAX_HEIGHT: CGFloat = 0.85;
    /// The narrowest and widest a tab thumbnail is drawn in the overview.
    pub const THUMBNAIL_MIN_WIDTH: CGFloat = 200.0;
    pub const THUMBNAIL_MAX_WIDTH: CGFloat = 320.0;
    /// A tab thumbnail's height for each point of its width: 16:10.
    pub const THUMBNAIL_SHAPE: CGFloat = 10.0 / 16.0;
}

/// Semantic colors. They follow the accent color, appearance, and contrast settings by themselves.
pub mod color {
    use objc2::rc::Retained;
    use objc2_app_kit::NSColor;

    /// Selection fill, focus ring, primary action.
    #[must_use]
    pub fn accent() -> Retained<NSColor> {
        NSColor::controlAccentColor()
    }

    /// Block, deny, revoke, conflicts, `PROD`.
    #[must_use]
    pub fn destructive() -> Retained<NSColor> {
        NSColor::systemRedColor()
    }

    /// Everything an agent does.
    #[must_use]
    pub fn agent() -> Retained<NSColor> {
        NSColor::systemOrangeColor()
    }

    /// Protection on, download done, `LOCAL`.
    #[must_use]
    pub fn success() -> Retained<NSColor> {
        NSColor::systemGreenColor()
    }

    /// Text on glass.
    #[must_use]
    pub fn label() -> Retained<NSColor> {
        NSColor::labelColor()
    }

    /// Secondary text on glass.
    #[must_use]
    pub fn secondary_label() -> Retained<NSColor> {
        NSColor::secondaryLabelColor()
    }

    /// Hairlines inside overlays.
    #[must_use]
    pub fn separator() -> Retained<NSColor> {
        NSColor::separatorColor()
    }

    /// How much accent a selected row's fill has over the glass.
    pub const SELECTION_FILL_ALPHA: objc2_core_foundation::CGFloat = 0.18;

    /// Fill behind a selected row: a subtle accent tint, never glass.
    #[must_use]
    pub fn selection_fill() -> Retained<NSColor> {
        NSColor::controlAccentColor().colorWithAlphaComponent(SELECTION_FILL_ALPHA)
    }

    /// Text on [`selection_fill`]: the primary label color, so the tint stays quiet and the
    /// accent match highlights stay visible.
    #[must_use]
    pub fn selection_text() -> Retained<NSColor> {
        NSColor::labelColor()
    }

    /// Placeholders, and what is there but does not matter now.
    #[must_use]
    pub fn tertiary_label() -> Retained<NSColor> {
        NSColor::tertiaryLabelColor()
    }

    /// Behind a group of rows in a standard window, where there is no glass to sit on.
    #[must_use]
    pub fn group_fill() -> Retained<NSColor> {
        NSColor::quaternarySystemFillColor()
    }

    /// How much color a tinted row or card has.
    pub const TINT_ALPHA: objc2_core_foundation::CGFloat = 0.12;

    /// How much color a tinted border has.
    pub const LINE_ALPHA: objc2_core_foundation::CGFloat = 0.5;

    /// Behind a row that is in conflict.
    #[must_use]
    pub fn destructive_fill() -> Retained<NSColor> {
        NSColor::systemRedColor().colorWithAlphaComponent(TINT_ALPHA)
    }

    /// The border of something in conflict: a keycap, a card.
    #[must_use]
    pub fn destructive_line() -> Retained<NSColor> {
        NSColor::systemRedColor().colorWithAlphaComponent(LINE_ALPHA)
    }

    /// Behind a card that says protection is on.
    #[must_use]
    pub fn success_fill() -> Retained<NSColor> {
        NSColor::systemGreenColor().colorWithAlphaComponent(TINT_ALPHA)
    }

    /// The border of a card that says protection is on.
    #[must_use]
    pub fn success_line() -> Retained<NSColor> {
        NSColor::systemGreenColor().colorWithAlphaComponent(LINE_ALPHA)
    }

    /// The border of something selected or waiting for keys: a keycap on a selected row, the
    /// box that records a shortcut.
    #[must_use]
    pub fn accent_line() -> Retained<NSColor> {
        NSColor::controlAccentColor().colorWithAlphaComponent(LINE_ALPHA)
    }

    /// Behind a keycap: `#E8E8ED` on light, `#3A3A3C` on dark.
    #[must_use]
    pub fn keycap_fill() -> Retained<NSColor> {
        NSColor::secondarySystemFillColor()
    }

    /// Behind a row's tile, a category tag, a footer strip: quieter than a keycap.
    #[must_use]
    pub fn subtle_fill() -> Retained<NSColor> {
        NSColor::quaternarySystemFillColor()
    }

    /// Behind a filled accent badge: the slot a tab is pinned to.
    #[must_use]
    pub fn accent_fill() -> Retained<NSColor> {
        NSColor::controlAccentColor()
    }

    /// Text on [`accent_fill`].
    #[must_use]
    pub fn on_accent() -> Retained<NSColor> {
        NSColor::alternateSelectedControlTextColor()
    }
}

/// Whether `view` is drawn with Increase Contrast on: borders that carry meaning get heavier.
/// The system setting, or a high-contrast appearance on the view.
#[must_use]
pub fn increased_contrast(view: &NSView) -> bool {
    if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast() {
        return true;
    }
    // SAFETY: the appearance names are immutable `NSString`s exported by AppKit.
    let (aqua, dark, high, high_dark) = unsafe {
        (
            NSAppearanceNameAqua,
            NSAppearanceNameDarkAqua,
            NSAppearanceNameAccessibilityHighContrastAqua,
            NSAppearanceNameAccessibilityHighContrastDarkAqua,
        )
    };
    let names = NSArray::from_slice(&[aqua, dark, high, high_dark]);
    view.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&names)
        .is_some_and(|name| *name == *high || *name == *high_dark)
}

/// Typographic roles: SF Pro through text styles, SF Mono at the matching sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    /// Palette and address input: `.title3` at 1.2×.
    PaletteInput,
    /// Sheet titles, empty states.
    Title,
    /// The title of a Settings pane: `.headline`, bold.
    Headline,
    /// Rows in palette, switcher, popovers.
    RowTitle,
    /// Hosts, ages, counters.
    RowDetail,
    /// Hint bars, footers, section headers.
    Caption,
}

impl TextRole {
    /// Scale applied on top of the text style's size.
    #[must_use]
    pub fn scale(self) -> CGFloat {
        match self {
            Self::PaletteInput => 1.2,
            Self::Title | Self::Headline | Self::RowTitle | Self::RowDetail | Self::Caption => 1.0,
        }
    }

    fn style(self) -> &'static NSFontTextStyle {
        // SAFETY: the text style constants are immutable `NSString`s exported by AppKit.
        unsafe {
            match self {
                Self::PaletteInput => NSFontTextStyleTitle3,
                Self::Title => NSFontTextStyleTitle2,
                Self::Headline => NSFontTextStyleHeadline,
                Self::RowTitle => NSFontTextStyleBody,
                Self::RowDetail => NSFontTextStyleSubheadline,
                Self::Caption => NSFontTextStyleCaption1,
            }
        }
    }

    /// Point size of the role at the user's current text size.
    #[must_use]
    pub fn point_size(self) -> CGFloat {
        preferred(self.style()).pointSize() * self.scale()
    }

    /// SF Pro in this role.
    #[must_use]
    pub fn font(self) -> Retained<NSFont> {
        let base = preferred(self.style());
        if (self.scale() - 1.0).abs() < CGFloat::EPSILON {
            base
        } else {
            NSFont::systemFontOfSize(base.pointSize() * self.scale())
        }
    }

    /// SF Mono at this role's size: URLs, command ids, payloads.
    #[must_use]
    pub fn mono_font(self) -> Retained<NSFont> {
        // SAFETY: `NSFontWeightRegular` is an immutable `CGFloat` exported by AppKit.
        let regular = unsafe { NSFontWeightRegular };
        NSFont::monospacedSystemFontOfSize_weight(self.point_size(), regular)
    }

    /// SF Mono at this role's size, semibold: the host of a URL, set apart from the rest.
    #[must_use]
    pub fn strong_mono_font(self) -> Retained<NSFont> {
        // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
        let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
        NSFont::monospacedSystemFontOfSize_weight(self.point_size(), semibold)
    }
}

fn preferred(style: &NSFontTextStyle) -> Retained<NSFont> {
    let options = NSDictionary::new();
    // SAFETY: an empty options dictionary is valid for any key and value type.
    unsafe { NSFont::preferredFontForTextStyle_options(style, &options) }
}

/// Black at `alpha`, for scrims and shadows.
#[must_use]
pub fn black(alpha: CGFloat) -> Retained<NSColor> {
    NSColor::blackColor().colorWithAlphaComponent(alpha)
}

/// White at `alpha`, for the hairline ring of floating surfaces.
#[must_use]
pub fn white(alpha: CGFloat) -> Retained<NSColor> {
    NSColor::whiteColor().colorWithAlphaComponent(alpha)
}

#[cfg(test)]
mod tests {
    use super::{TextRole, radius, space};

    #[test]
    fn spacing_follows_the_scale() {
        assert_eq!(
            [space::XS, space::SM, space::MD, space::LG, space::XL],
            [4.0, 8.0, 12.0, 16.0, 24.0]
        );
        assert!((radius::OVERLAY - 22.0).abs() < f64::EPSILON);
    }

    #[test]
    fn only_the_palette_input_is_scaled() {
        assert!((TextRole::PaletteInput.scale() - 1.2).abs() < f64::EPSILON);
        assert!((TextRole::RowTitle.scale() - 1.0).abs() < f64::EPSILON);
    }
}

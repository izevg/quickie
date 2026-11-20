//! One download as a row (`docs/design/inventory.md`, entities `download_row`;
//! `docs/design/mocks/system/downloads/`): its state's glyph on a tinted circle, the file name,
//! the detail line, a 4 pt accent bar under them while the size is known, and its buttons. It
//! lives in the popover's slice until an `entities` crate exists.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSColor, NSImage, NSImageSymbolConfiguration, NSImageView,
    NSLineBreakMode, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::Plate;
use quickie_ui_shared::form::Paint;
use quickie_ui_shared::tokens::{TextRole, color, radius, space};

use super::line::{Action, Line, Phase};

/// A row's height.
pub(crate) const ROW_HEIGHT: CGFloat = 56.0;
/// A round button around a symbol.
const BUTTON: CGFloat = 24.0;
/// The tinted circle behind the state glyph.
const CIRCLE: CGFloat = 32.0;
/// The progress bar's thickness.
const BAR: CGFloat = 4.0;

pub(crate) struct DownloadRow {
    view: Retained<NSView>,
    circle: Retained<Plate>,
    glyph: Retained<NSImageView>,
    name: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    track: Retained<Plate>,
    fill: Retained<Plate>,
    fraction: std::cell::Cell<Option<f64>>,
    buttons: [Retained<NSButton>; 2],
}

impl DownloadRow {
    /// A row whose buttons send `primary:` and `secondary:` to `target`, tagged with `index`.
    pub(crate) fn new(index: usize, target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let view = NSView::new(mtm);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(radius::ROW);
        }
        let circle = Plate::new(Some(color::group_fill), None, CIRCLE / 2.0, mtm);
        let glyph = NSImageView::new(mtm);
        circle.addSubview(&glyph);
        let name = label(TextRole::RowTitle, &color::label(), mtm);
        name.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        let detail = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        detail.setFont(Some(&TextRole::Caption.mono_font()));
        detail.sizeToFit();
        let track = Plate::new(Some(color::group_fill), None, BAR / 2.0, mtm);
        let fill = Plate::new(Some(color::accent), None, BAR / 2.0, mtm);
        track.addSubview(&fill);
        let tag = isize::try_from(index).unwrap_or(isize::MAX);
        let buttons =
            [sel!(primary:), sel!(secondary:)].map(|action| button(action, tag, target, mtm));
        for part in [&*circle as &NSView, &name, &detail, &track] {
            view.addSubview(part);
        }
        for button in &buttons {
            view.addSubview(button);
        }
        Self {
            view,
            circle,
            glyph,
            name,
            detail,
            track,
            fill,
            fraction: std::cell::Cell::new(None),
            buttons,
        }
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }

    /// Clicks button `slot` when it shows, as the pointer does.
    pub(crate) fn press(&self, slot: usize) {
        if let Some(button) = self.buttons.get(slot).filter(|button| !button.isHidden()) {
            // SAFETY: `performClick:` takes any sender, including none, and sends the button's
            // action to its target, which the popover keeps alive.
            unsafe { button.performClick(None) };
        }
    }

    /// Shows `line`, `width` wide; a selected row gets the selection fill.
    pub(crate) fn show(&self, line: &Line, selected: bool, width: CGFloat) {
        let fill = selected.then(color::selection_fill);
        if let Some(layer) = self.view.layer() {
            layer.setBackgroundColor(fill.map(|fill| fill.CGColor()).as_deref());
        }
        let (glyph, tint, circle): (&str, Paint, Paint) = match line.phase {
            Phase::Running => ("arrow.down", color::accent, accent_fill),
            Phase::Done => ("doc.fill", color::success, color::success_fill),
            Phase::Failed => (
                "exclamationmark.triangle.fill",
                color::destructive,
                color::destructive_fill,
            ),
            Phase::Cancelled => ("xmark", color::secondary_label, color::group_fill),
        };
        self.circle.set_paint(Some(circle), None);
        self.glyph
            .setImage(image(glyph, line.phase.label(), TextRole::RowTitle).as_deref());
        self.glyph.setContentTintColor(Some(&tint()));
        self.name.setStringValue(&NSString::from_str(&line.name));
        self.detail
            .setStringValue(&NSString::from_str(&line.detail));
        self.fraction.set(line.fraction);
        self.track.setHidden(line.fraction.is_none());
        let actions = line.phase.actions();
        for (slot, button) in self.buttons.iter().enumerate() {
            match actions.get(slot) {
                Some(&action) => {
                    let (name, description) = action.symbol();
                    button.setImage(image(name, description, TextRole::RowDetail).as_deref());
                    button.setToolTip(Some(&NSString::from_str(description)));
                    button.setAccessibilityLabel(Some(&NSString::from_str(description)));
                    button.setHidden(false);
                }
                None => button.setHidden(true),
            }
        }
        self.lay_out(width, actions.len());
    }

    fn lay_out(&self, width: CGFloat, buttons: usize) {
        self.view.setFrameSize(NSSize::new(width, ROW_HEIGHT));
        let inset = space::SM;
        let middle = |height: CGFloat| ((ROW_HEIGHT - height) / 2.0).round();
        self.circle.setFrame(NSRect::new(
            NSPoint::new(inset, middle(CIRCLE)),
            NSSize::new(CIRCLE, CIRCLE),
        ));
        self.glyph.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(CIRCLE, CIRCLE),
        ));
        let mut right = width - inset;
        for button in self.buttons.iter().take(buttons).rev() {
            right -= BUTTON;
            button.setFrame(NSRect::new(
                NSPoint::new(right, middle(BUTTON)),
                NSSize::new(BUTTON, BUTTON),
            ));
            right -= space::XS;
        }
        let left = inset + CIRCLE + space::MD;
        let text_width = (right - space::SM - left).max(0.0);
        let name_height = self.name.frame().size.height;
        let detail_height = self.detail.frame().size.height;
        let bar = if self.track.isHidden() {
            0.0
        } else {
            BAR + space::XS
        };
        let stack = name_height + 2.0 + detail_height + bar;
        // Bottom up: the bar, the detail, the name.
        let mut y = middle(stack);
        if let Some(fraction) = self.fraction.get().filter(|_| bar > 0.0) {
            self.track.setFrame(NSRect::new(
                NSPoint::new(left, y),
                NSSize::new(text_width, BAR),
            ));
            self.fill.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new((text_width * fraction).round().max(BAR), BAR),
            ));
            y += bar;
        }
        self.detail.setFrame(NSRect::new(
            NSPoint::new(left, y),
            NSSize::new(text_width, detail_height),
        ));
        y += detail_height + 2.0;
        self.name.setFrame(NSRect::new(
            NSPoint::new(left, y),
            NSSize::new(text_width, name_height),
        ));
    }
}

/// Behind a running download's glyph: the accent at the tint alpha.
fn accent_fill() -> Retained<NSColor> {
    color::accent().colorWithAlphaComponent(color::TINT_ALPHA)
}

impl Phase {
    /// What `VoiceOver` calls the state glyph.
    fn label(self) -> &'static str {
        match self {
            Self::Running => "Downloading",
            Self::Done => "Downloaded",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl Action {
    /// The button's symbol and what it does.
    pub(crate) fn symbol(self) -> (&'static str, &'static str) {
        match self {
            Self::Cancel => ("xmark.circle.fill", "Cancel"),
            Self::Reveal => ("magnifyingglass.circle.fill", "Show in Finder"),
            Self::Open => ("arrow.up.right.circle.fill", "Open"),
            Self::Retry => ("arrow.clockwise.circle.fill", "Retry"),
        }
    }
}

fn label(role: TextRole, tint: &NSColor, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(" "), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

/// The SF Symbol `name` at the size of `role`.
fn image(name: &str, description: &str, role: TextRole) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(description)),
    )?;
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(role.point_size(), 0.0);
    image.imageWithSymbolConfiguration(&configuration)
}

/// A borderless round button that sends `action` to `target`, tagged with its row.
fn button(
    action: Sel,
    tag: isize,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setBordered(false);
    button.setTitle(&NSString::new());
    button.setContentTintColor(Some(&color::secondary_label()));
    button.setTag(tag);
    // SAFETY: the target is weak; the popover keeps it alive for as long as its rows.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.setRefusesFirstResponder(true);
    button
}

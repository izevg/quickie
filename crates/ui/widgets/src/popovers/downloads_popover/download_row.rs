//! One download as a row (`docs/design/inventory.md`, entities `download_row`): a glyph for its
//! state, the file name, a thin bar while the size is known, the detail line, and its buttons.
//! It lives in the popover's slice until an `entities` crate exists.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSButton, NSControlSize, NSImage, NSImageSymbolConfiguration, NSImageView, NSLineBreakMode,
    NSProgressIndicator, NSProgressIndicatorStyle, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, radius, space};

use super::line::{Action, Line, Phase};

/// A row's height.
pub(crate) const ROW_HEIGHT: CGFloat = 52.0;
/// A round button around a symbol.
const BUTTON: CGFloat = 24.0;
/// The state glyph's box.
const GLYPH: CGFloat = 28.0;

pub(crate) struct DownloadRow {
    view: Retained<NSView>,
    glyph: Retained<NSImageView>,
    name: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    bar: Retained<NSProgressIndicator>,
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
        let glyph = NSImageView::new(mtm);
        let name = label(TextRole::RowTitle, &color::label(), mtm);
        name.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        let detail = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        let bar = NSProgressIndicator::new(mtm);
        bar.setStyle(NSProgressIndicatorStyle::Bar);
        bar.setControlSize(NSControlSize::Small);
        bar.setIndeterminate(false);
        bar.setMinValue(0.0);
        bar.setMaxValue(1.0);
        let tag = isize::try_from(index).unwrap_or(isize::MAX);
        let buttons =
            [sel!(primary:), sel!(secondary:)].map(|action| button(action, tag, target, mtm));
        for part in [&*glyph as &NSView, &name, &detail, &bar] {
            view.addSubview(part);
        }
        for button in &buttons {
            view.addSubview(button);
        }
        Self {
            view,
            glyph,
            name,
            detail,
            bar,
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
        let (glyph, tint) = match line.phase {
            Phase::Running => ("arrow.down.circle", color::accent()),
            Phase::Done => ("doc", color::success()),
            Phase::Failed => ("exclamationmark.triangle", color::destructive()),
            Phase::Cancelled => ("xmark.circle", color::secondary_label()),
        };
        self.glyph
            .setImage(image(glyph, line.phase.label(), TextRole::Title).as_deref());
        self.glyph.setContentTintColor(Some(&tint));
        self.name.setStringValue(&NSString::from_str(&line.name));
        self.detail
            .setStringValue(&NSString::from_str(&line.detail));
        if let Some(fraction) = line.fraction {
            self.bar.setDoubleValue(fraction);
        }
        self.bar.setHidden(line.fraction.is_none());
        let actions = line.phase.actions();
        for (slot, button) in self.buttons.iter().enumerate() {
            match actions.get(slot) {
                Some(&action) => {
                    let (name, description) = action.symbol();
                    button.setImage(image(name, description, TextRole::RowDetail).as_deref());
                    button.setToolTip(Some(&NSString::from_str(description)));
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
        self.glyph.setFrame(NSRect::new(
            NSPoint::new(inset, middle(GLYPH)),
            NSSize::new(GLYPH, GLYPH),
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
        let left = inset + GLYPH + space::SM;
        let text_width = (right - space::XS - left).max(0.0);
        let name_height = self.name.frame().size.height;
        let detail_height = self.detail.frame().size.height;
        let bar_height = if self.bar.isHidden() { 0.0 } else { 4.0 };
        let gap = if self.bar.isHidden() { 2.0 } else { 3.0 };
        let stack = name_height + detail_height + bar_height + gap * 2.0;
        let mut y = middle(stack);
        self.detail.setFrame(NSRect::new(
            NSPoint::new(left, y),
            NSSize::new(text_width, detail_height),
        ));
        y += detail_height + gap;
        if bar_height > 0.0 {
            self.bar.setFrame(NSRect::new(
                NSPoint::new(left, y),
                NSSize::new(text_width, bar_height),
            ));
            y += bar_height + gap;
        }
        self.name.setFrame(NSRect::new(
            NSPoint::new(left, y),
            NSSize::new(text_width, name_height),
        ));
    }
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

fn label(
    role: TextRole,
    tint: &objc2_app_kit::NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
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

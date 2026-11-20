//! What frames an answer (`docs/design/mocks/agent/ask/`, `ask-empty/`): the model chip in the
//! header, the empty state without an endpoint, and the footer with what to do next.

mod empty;
mod footer;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSColor, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::Plate;
use quickie_ui_shared::tokens::{TextRole, color, space};

pub use empty::Empty;
pub use footer::{Footer, FooterKind};

/// The chip's status dot.
const DOT: CGFloat = 6.0;

/// The model chip: `● qwen3-8b · local` in SF Mono on a capsule tinted with the agent color;
/// an empty view for no model.
pub fn model_chip(model: &str, mtm: MainThreadMarker) -> Retained<NSView> {
    if model.is_empty() {
        return NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    }
    let label = NSTextField::labelWithString(&NSString::from_str(model), mtm);
    label.setFont(Some(&TextRole::Caption.mono_font()));
    label.setTextColor(Some(&color::label()));
    label.sizeToFit();
    let text = label.frame().size;
    let height = (text.height + space::XS * 2.0).ceil();
    let width = (space::SM + DOT + space::XS + 2.0 + text.width + space::SM).ceil();
    let capsule = Plate::new(Some(agent_fill), Some(agent_line), height / 2.0, mtm);
    // The plate is flipped: y runs down from its top edge.
    capsule.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));
    let dot = Plate::new(Some(color::agent), None, DOT / 2.0, mtm);
    dot.setFrame(NSRect::new(
        NSPoint::new(space::SM, ((height - DOT) / 2.0).round()),
        NSSize::new(DOT, DOT),
    ));
    capsule.addSubview(&dot);
    label.setFrame(NSRect::new(
        NSPoint::new(
            space::SM + DOT + space::XS + 2.0,
            ((height - text.height) / 2.0).round(),
        ),
        text,
    ));
    capsule.addSubview(&label);
    capsule.into_super()
}

/// The agent color at the tint alpha, behind chips and the empty state's tile.
pub fn agent_fill() -> Retained<NSColor> {
    color::agent().colorWithAlphaComponent(color::TINT_ALPHA)
}

/// The agent color at the line alpha, around them.
pub fn agent_line() -> Retained<NSColor> {
    color::agent().colorWithAlphaComponent(color::LINE_ALPHA)
}

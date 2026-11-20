//! A pill: a capsule of E1 glass holding one short line. Toasts, the full-screen notice, and
//! status chips are pills (`docs/design/mocks/components/toast-pill.html`). The line is a row
//! of parts, left to right: a status symbol, the text, a count, a detail in SF Mono, a note
//! after a hairline, keycaps. A success pill sits on a green tint with a green border.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSFont, NSLineBreakMode, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{StyledText, keycap_group, symbol};
use crate::form::{Paint, Plate};
use crate::glass::{Elevation, GlassSurface};
use crate::tokens::{TextRole, color, space};

/// How wide a pill may grow before its text is truncated.
const MAX_WIDTH: CGFloat = 520.0;
/// A status dot's diameter.
const DOT: CGFloat = 6.0;

/// One part of a pill's line.
pub enum PillPart {
    /// A status dot in `tint`: green for a secure page, red for one that is not.
    Dot(Paint),
    /// An SF Symbol at the text's size, in `tint`.
    Symbol { name: &'static str, tint: Paint },
    /// A small SF Symbol at the caption size, in `tint`, read as `description`: a lock.
    Glyph {
        name: &'static str,
        description: &'static str,
        tint: Paint,
    },
    /// The text, in the label color: what happened.
    Text(String),
    /// Styled text, for a host with its domain in bold.
    Styled(StyledText),
    /// A detail in SF Mono, secondary: `to clipboard`, a file name.
    Detail(String),
    /// A count in SF Mono, in `tint` on a capsule of `fill`: `12 requests`.
    Count {
        text: String,
        tint: Paint,
        fill: Paint,
    },
    /// A note after a hairline, in `tint`: `Protection is on`.
    Note { text: String, tint: Paint },
    /// Keycaps of a chord: `⇧⌘T`.
    Keys(String),
}

/// How a pill is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillTone {
    /// Glass alone.
    Plain,
    /// A green tint and border: protection did its job, a download finished.
    Success,
}

/// A pill with its line, sized to fit. Place it with [`Pill::view`] and [`Pill::size`].
pub struct Pill {
    surface: GlassSurface,
    size: NSSize,
}

impl Pill {
    /// A neutral pill reading `text`.
    #[must_use]
    pub fn new(text: &str, mtm: MainThreadMarker) -> Self {
        Self::with_parts(&[PillPart::Text(text.to_owned())], PillTone::Plain, mtm)
    }

    /// A pill of `parts` in `tone`.
    #[must_use]
    pub fn with_parts(parts: &[PillPart], tone: PillTone, mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E1, mtm);
        let views: Vec<(Retained<NSView>, bool)> =
            parts.iter().map(|part| view(part, mtm)).collect();
        let height = (views
            .iter()
            .map(|(view, _)| view.frame().size.height)
            .fold(0.0, CGFloat::max)
            + space::SM * 2.0)
            .ceil();
        let leading = if matches!(
            parts.first(),
            Some(PillPart::Symbol { .. } | PillPart::Dot(_))
        ) {
            space::MD
        } else {
            space::LG
        };
        let natural: CGFloat = views
            .iter()
            .map(|(view, _)| view.frame().size.width)
            .sum::<CGFloat>()
            + space::SM * gaps(views.len());
        let width = (leading + natural + space::LG).min(MAX_WIDTH).ceil();
        // What is left for the stretchable parts (the text) once the others have their width.
        let fixed: CGFloat = views
            .iter()
            .filter(|(_, stretches)| !stretches)
            .map(|(view, _)| view.frame().size.width)
            .sum();
        let stretchable =
            (width - leading - space::LG - space::SM * gaps(views.len()) - fixed).max(0.0);
        let wanted: CGFloat = views
            .iter()
            .filter(|(_, stretches)| *stretches)
            .map(|(view, _)| view.frame().size.width)
            .sum();
        let squeeze = if wanted > 0.0 {
            (stretchable / wanted).min(1.0)
        } else {
            1.0
        };
        let content = surface.content_view();
        if tone == PillTone::Success {
            let tint = Plate::new(
                Some(color::success_fill),
                Some(color::success_line),
                height / 2.0,
                mtm,
            );
            tint.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width, height),
            ));
            content.addSubview(&tint);
        }
        let mut x = leading;
        for (view, stretches) in views {
            let mut size = view.frame().size;
            if stretches {
                size.width = (size.width * squeeze).floor();
            }
            view.setFrame(NSRect::new(
                NSPoint::new(x, ((height - size.height) / 2.0).round()),
                size,
            ));
            content.addSubview(&view);
            x += size.width + space::SM;
        }
        let size = NSSize::new(width, height);
        surface.set_frame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        Self { surface, size }
    }

    /// The view to add above the page.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// Its size, to lay it out.
    #[must_use]
    pub fn size(&self) -> NSSize {
        self.size
    }

    /// Moves it to `origin` in its superview.
    pub fn place(&self, origin: NSPoint) {
        self.surface.set_frame(NSRect::new(origin, self.size));
    }
}

fn gaps(count: usize) -> CGFloat {
    CGFloat::from(u32::try_from(count.saturating_sub(1)).unwrap_or(0))
}

/// A part as a view sized to fit, and whether it gives way when the pill is too narrow.
fn view(part: &PillPart, mtm: MainThreadMarker) -> (Retained<NSView>, bool) {
    match part {
        PillPart::Dot(tint) => {
            let dot = Plate::new(Some(*tint), None, DOT / 2.0, mtm);
            dot.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(DOT, DOT)));
            (dot.into_super(), false)
        }
        PillPart::Glyph {
            name,
            description,
            tint,
        } => {
            let view = symbol(name, description, TextRole::Caption, &tint(), mtm)
                .map_or_else(|| NSView::new(mtm), |view| view.into_super().into_super());
            (view, false)
        }
        PillPart::Symbol { name, tint } => {
            let view = symbol(name, "", TextRole::RowTitle, &tint(), mtm)
                .map_or_else(|| NSView::new(mtm), |view| view.into_super().into_super());
            (view, false)
        }
        PillPart::Text(text) => (
            label(text, &TextRole::RowTitle.font(), color::label, mtm),
            true,
        ),
        PillPart::Styled(styled) => {
            let field = NSTextField::labelWithAttributedString(styled.string(), mtm);
            field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
            field.sizeToFit();
            (field.into_super().into_super(), true)
        }
        PillPart::Detail(text) => (
            label(
                text,
                &TextRole::Caption.mono_font(),
                color::secondary_label,
                mtm,
            ),
            true,
        ),
        PillPart::Count { text, tint, fill } => (count(text, *tint, *fill, mtm), false),
        PillPart::Note { text, tint } => (note(text, *tint, mtm), false),
        PillPart::Keys(chord) => (keycap_group(chord, false, mtm), false),
    }
}

fn label(text: &str, font: &NSFont, tint: Paint, mtm: MainThreadMarker) -> Retained<NSView> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(font));
    field.setTextColor(Some(&tint()));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field.into_super().into_super()
}

/// A capsule of `fill` with the text in `tint`, SF Mono.
fn count(text: &str, tint: Paint, fill: Paint, mtm: MainThreadMarker) -> Retained<NSView> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&TextRole::Caption.mono_font()));
    field.setTextColor(Some(&tint()));
    field.sizeToFit();
    let text_size = field.frame().size;
    let size = NSSize::new(
        (text_size.width + space::SM * 2.0).ceil(),
        (text_size.height + space::XS).ceil(),
    );
    let capsule = Plate::new(Some(fill), None, size.height / 2.0, mtm);
    capsule.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
    field.setFrame(NSRect::new(
        NSPoint::new(space::SM, ((size.height - text_size.height) / 2.0).ceil()),
        text_size,
    ));
    capsule.addSubview(&field);
    capsule.into_super()
}

/// A hairline, then `text` in `tint`, in the caption style.
fn note(text: &str, tint: Paint, mtm: MainThreadMarker) -> Retained<NSView> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&TextRole::RowDetail.font()));
    field.setTextColor(Some(&tint()));
    field.sizeToFit();
    let text_size = field.frame().size;
    let size = NSSize::new(
        (1.0 + space::SM + text_size.width).ceil(),
        text_size.height.ceil(),
    );
    let holder = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), size),
    );
    let line = Plate::new(Some(color::separator), None, 0.0, mtm);
    line.setFrame(NSRect::new(
        NSPoint::new(0.0, 2.0),
        NSSize::new(1.0, (size.height - 4.0).max(0.0)),
    ));
    holder.addSubview(&line);
    field.setFrameOrigin(NSPoint::new(1.0 + space::SM, 0.0));
    holder.addSubview(&field);
    holder
}

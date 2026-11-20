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
        let natural: Vec<(CGFloat, bool)> = views
            .iter()
            .map(|(view, stretches)| (view.frame().size.width, *stretches))
            .collect();
        let (width, widths) = layout(&natural, leading);
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
        for ((view, _), part_width) in views.into_iter().zip(widths) {
            let size = NSSize::new(part_width, view.frame().size.height);
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

/// The pill's width and each part's, from the parts' natural widths and whether each gives
/// way, after `leading` room. A part keeps its natural width while the pill is under
/// [`MAX_WIDTH`]: `AppKit` sizes a label to the backing pixel, half a point on a Retina
/// display, and a label even that much narrower than its text loses its last glyph (#66).
/// Past the limit, the parts that give way share what is left, in whole points.
fn layout(parts: &[(CGFloat, bool)], leading: CGFloat) -> (CGFloat, Vec<CGFloat>) {
    let between = space::SM * gaps(parts.len());
    let natural: CGFloat = parts.iter().map(|(width, _)| width).sum::<CGFloat>() + between;
    let width = (leading + natural + space::LG).min(MAX_WIDTH).ceil();
    let sum = |stretches: bool| -> CGFloat {
        parts
            .iter()
            .filter(|(_, part)| *part == stretches)
            .map(|(width, _)| width)
            .sum()
    };
    // What is left for the parts that give way (the text) once the others have their width.
    let room = (width - leading - space::LG - between - sum(false)).max(0.0);
    let wanted = sum(true);
    let widths = parts
        .iter()
        .map(|&(natural, stretches)| {
            if stretches && wanted > room {
                (natural * room / wanted).floor()
            } else {
                natural
            }
        })
        .collect();
    (width, widths)
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
            // An attributed label breaks as its string's paragraph style says, which wraps by
            // default: a host squeezed by a point would lose its end without an ellipsis.
            styled.set_line_break(NSLineBreakMode::ByTruncatingMiddle);
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

#[cfg(test)]
mod tests {
    use super::{MAX_WIDTH, layout};
    use crate::tokens::space;

    /// The origin chip on a Retina display: a dot, the host and the path sized to the half
    /// point, a lock (#66).
    const CHIP: [(f64, bool); 4] = [(6.0, false), (99.5, true), (60.5, true), (9.0, false)];

    #[test]
    fn text_that_fits_keeps_its_half_points() {
        let (width, widths) = layout(&CHIP, space::MD);

        assert_eq!(widths, vec![6.0, 99.5, 60.5, 9.0]);
        assert_eq!(
            width,
            (space::MD + 175.0 + space::SM * 3.0 + space::LG).ceil()
        );
    }

    #[test]
    fn text_past_the_limit_gives_way_in_whole_points() {
        let parts = [(6.0, false), (400.5, true), (200.5, true), (9.0, false)];
        let (width, widths) = layout(&parts, space::MD);

        assert!((width - MAX_WIDTH).abs() < f64::EPSILON);
        assert!((widths[0] - 6.0).abs() < f64::EPSILON);
        assert!((widths[3] - 9.0).abs() < f64::EPSILON);
        let room = MAX_WIDTH - space::MD - space::LG - space::SM * 3.0 - 15.0;
        assert!(widths[1] + widths[2] <= room);
        assert!(widths[1] + widths[2] > room - 2.0);
        assert!(widths[1] < 400.5 && widths[1].fract() == 0.0);
        assert!(widths[2] < 200.5 && widths[2].fract() == 0.0);
    }
}

//! The E3 sheet (`docs/design/system.md`, Elevation): tinted glass attached to the top edge of
//! the window, the page dimmed 30 % under it, sliding down with `sheet-enter`. A sheet stops the
//! user for a decision a page asked for, so it always animates, even when a key brought it up,
//! and leaves with `fade-fast`.
//!
//! Its content is a column: the header (who asks, set large), lines of text, an optional field,
//! the answers as rows with their keys, and a hint bar. The rows are click targets too; the
//! keys themselves are the caller's to route.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSLineBreakMode, NSTextField, NSView, NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::controls::{StyledText, hint_bar, keycap_group, symbol};
use crate::glass::{Elevation, GlassSurface, Scrim};
use crate::motion::{self, Trigger};
use crate::tokens::{TextRole, color, overlay, radius, space};

/// How an answer row looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    /// What `⏎` or the first number does: the accent tint.
    Primary,
    Plain,
    /// The safe default, named as such.
    Default,
}

/// One answer: its label, the keys that give it, and how it looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetAnswer {
    pub label: String,
    /// In glyphs: `⏎`, `⎋`, `1`, `⌘B`.
    pub keys: String,
    pub emphasis: Emphasis,
}

/// A line of text under the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetLine {
    pub text: String,
    pub role: TextRole,
    /// In the secondary label color.
    pub dimmed: bool,
}

/// What a sheet shows.
pub struct SheetContent {
    /// Who asks, set large: an origin, with its registrable domain in bold.
    pub header: StyledText,
    /// An SF Symbol before the header (`video.fill`), tinted with the accent.
    pub glyph: Option<String>,
    pub lines: Vec<SheetLine>,
    /// A text field holding this text, focused and selected.
    pub field: Option<String>,
    pub answers: Vec<SheetAnswer>,
    pub hints: Vec<(String, String)>,
}

/// Longest a message may run before it is cut, in points.
const MAX_LINES_HEIGHT: CGFloat = 320.0;

/// Height of an answer row.
const ROW_HEIGHT: CGFloat = 32.0;

type Clicked = Rc<dyn Fn(usize)>;

pub struct Ivars {
    clicked: RefCell<Option<Clicked>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `SheetTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSheetTarget"]
    #[ivars = Ivars]
    pub struct SheetTarget;

    unsafe impl NSObjectProtocol for SheetTarget {}

    impl SheetTarget {
        #[unsafe(method(answer:))]
        fn answer(&self, sender: Option<&NSButton>) {
            let clicked = self.ivars().clicked.borrow().clone();
            let index = sender.and_then(|button| usize::try_from(button.tag()).ok());
            if let (Some(clicked), Some(index)) = (clicked, index) {
                clicked(index);
            }
        }
    }
);

impl SheetTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            clicked: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// One E3 sheet; show it with new content each time. Each showing gets a surface and a scrim
/// of its own, so the fade of the one before never hides it.
pub struct Sheet {
    scrim: RefCell<Option<Scrim>>,
    surface: RefCell<Option<GlassSurface>>,
    field: RefCell<Option<Retained<NSTextField>>>,
    target: Retained<SheetTarget>,
    open: Cell<bool>,
    mtm: MainThreadMarker,
}

impl Sheet {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            scrim: RefCell::new(None),
            surface: RefCell::new(None),
            field: RefCell::new(None),
            target: SheetTarget::new(mtm),
            open: Cell::new(false),
            mtm,
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Shows `content` attached to the top of `host`, replacing what the sheet showed; a click
    /// on answer `i` calls `clicked(i)`. The field, if any, takes the keyboard.
    pub fn show(
        &self,
        host: &NSView,
        window: &NSWindow,
        content: &SheetContent,
        clicked: impl Fn(usize) + 'static,
    ) {
        self.remove();
        *self.target.ivars().clicked.borrow_mut() = Some(Rc::new(clicked));
        let width = overlay::SHEET_WIDTH.min(host.bounds().size.width - space::XL * 2.0);
        let surface = GlassSurface::new(Elevation::E3, self.mtm);
        let (height, field) = self.fill(surface.content_view(), content, width);
        let bounds = host.bounds();
        surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - width) / 2.0).round(),
                bounds.size.height - height,
            ),
            NSSize::new(width, height),
        ));
        surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        let scrim = Scrim::new(Elevation::E3, self.mtm);
        if let Some(scrim) = &scrim {
            scrim.view().setFrame(bounds);
            host.addSubview(scrim.view());
            motion::show(scrim.view(), motion::FADE_FAST, Trigger::App);
        }
        *self.scrim.borrow_mut() = scrim;
        host.addSubview(surface.view());
        motion::show(surface.view(), motion::SHEET_ENTER, Trigger::App);
        if let Some(field) = &field {
            window.makeFirstResponder(Some(field));
            // SAFETY: `selectText:` accepts any sender, including none.
            unsafe { field.selectText(None) };
        }
        *self.surface.borrow_mut() = Some(surface);
        *self.field.borrow_mut() = field;
        self.open.set(true);
    }

    /// Fades the sheet and its scrim out.
    pub fn hide(&self) {
        if !self.open.replace(false) {
            return;
        }
        if let Some(surface) = self.surface.borrow().as_ref() {
            motion::hide(surface.view(), motion::FADE_FAST, Trigger::App);
        }
        if let Some(scrim) = self.scrim.borrow().as_ref() {
            motion::hide(scrim.view(), motion::FADE_FAST, Trigger::App);
        }
        *self.target.ivars().clicked.borrow_mut() = None;
    }

    /// The field's text, when the sheet has one.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.field
            .borrow()
            .as_ref()
            .map(|field| field.stringValue().to_string())
    }

    /// The field, to tell whether it has the keyboard.
    #[must_use]
    pub fn field(&self) -> Option<Retained<NSTextField>> {
        self.field.borrow().clone()
    }

    fn remove(&self) {
        if let Some(surface) = self.surface.borrow_mut().take() {
            surface.view().removeFromSuperview();
        }
        if let Some(scrim) = self.scrim.borrow_mut().take() {
            scrim.view().removeFromSuperview();
        }
        *self.field.borrow_mut() = None;
    }

    /// Lays `content` out top-down in `view`, `width` wide; returns the height it takes and the
    /// field.
    fn fill(
        &self,
        view: &NSView,
        content: &SheetContent,
        width: CGFloat,
    ) -> (CGFloat, Option<Retained<NSTextField>>) {
        let mtm = self.mtm;
        let padding = space::PALETTE_PADDING;
        let inner = width - padding * 2.0;
        // Laid out downwards from y = 0, then flipped once the height is known.
        let mut placed: Vec<(Retained<NSView>, NSRect)> = Vec::new();
        let mut y = padding;

        let header = NSTextField::labelWithAttributedString(content.header.string(), mtm);
        header.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        let glyph = content
            .glyph
            .as_deref()
            .and_then(|name| symbol(name, "", TextRole::Title, &color::accent(), mtm));
        let glyph_width = glyph
            .as_ref()
            .map_or(0.0, |glyph| glyph.frame().size.width + space::SM);
        let header_height = header.fittingSize().height.ceil();
        if let Some(glyph) = glyph {
            let size = glyph.frame().size;
            let top = y + ((header_height - size.height) / 2.0).round();
            placed.push((
                glyph.into_super().into_super(),
                NSRect::new(NSPoint::new(padding, top), size),
            ));
        }
        placed.push((
            header.into_super().into_super(),
            NSRect::new(
                NSPoint::new(padding + glyph_width, y),
                NSSize::new(inner - glyph_width, header_height),
            ),
        ));
        y += header_height + space::SM;

        let mut lines_height = 0.0;
        for line in &content.lines {
            let label = wrapping(&line.text, line.role, line.dimmed, inner, mtm);
            let height = label
                .fittingSize()
                .height
                .ceil()
                .min(MAX_LINES_HEIGHT - lines_height)
                .max(0.0);
            lines_height += height + space::XS;
            placed.push((
                label.into_super().into_super(),
                NSRect::new(NSPoint::new(padding, y), NSSize::new(inner, height)),
            ));
            y += height + space::XS;
        }
        y += space::MD;

        let field = content.field.as_deref().map(|text| {
            let field = NSTextField::textFieldWithString(&NSString::from_str(text), mtm);
            field.setFont(Some(&TextRole::RowTitle.font()));
            let height = field.fittingSize().height.ceil();
            placed.push((
                field.clone().into_super().into_super(),
                NSRect::new(NSPoint::new(padding, y), NSSize::new(inner, height)),
            ));
            y += height + space::LG;
            field
        });

        for (index, answer) in content.answers.iter().enumerate() {
            let row = self.answer_row(answer, index, inner);
            placed.push((
                row,
                NSRect::new(NSPoint::new(padding, y), NSSize::new(inner, ROW_HEIGHT)),
            ));
            y += ROW_HEIGHT + space::XS;
        }

        if !content.hints.is_empty() {
            y += space::SM;
            let hints: Vec<(&str, &str)> = content
                .hints
                .iter()
                .map(|(key, meaning)| (key.as_str(), meaning.as_str()))
                .collect();
            let bar = hint_bar(&hints, mtm);
            let height = bar.frame().size.height;
            placed.push((
                bar,
                NSRect::new(NSPoint::new(padding, y), NSSize::new(inner, height)),
            ));
            y += height;
        }
        let height = (y + padding).ceil();
        for (subview, frame) in placed {
            subview.setFrame(NSRect::new(
                NSPoint::new(frame.origin.x, height - frame.origin.y - frame.size.height),
                frame.size,
            ));
            view.addSubview(&subview);
        }
        (height, field)
    }

    /// A row with the answer's label on the left and its keys on the right; a click on it
    /// answers.
    fn answer_row(&self, answer: &SheetAnswer, index: usize, width: CGFloat) -> Retained<NSView> {
        let mtm = self.mtm;
        let row = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, ROW_HEIGHT)),
        );
        row.setWantsLayer(true);
        let primary = answer.emphasis == Emphasis::Primary;
        if let Some(layer) = row.layer() {
            layer.setCornerRadius(radius::ROW);
            let fill = if primary {
                color::selection_fill()
            } else {
                crate::tokens::white(0.06)
            };
            layer.setBackgroundColor(Some(&fill.CGColor()));
        }
        let label = NSTextField::labelWithString(&NSString::from_str(&answer.label), mtm);
        label.setFont(Some(&TextRole::RowTitle.font()));
        label.setTextColor(Some(&color::label()));
        label.sizeToFit();
        let label_height = label.frame().size.height;
        let keys = keycap_group(&answer.keys, primary, mtm);
        let keys_size = keys.frame().size;
        let mut right = width - space::MD - keys_size.width;
        keys.setFrameOrigin(NSPoint::new(
            right,
            ((ROW_HEIGHT - keys_size.height) / 2.0).round(),
        ));
        row.addSubview(&keys);
        if answer.emphasis == Emphasis::Default {
            let note = NSTextField::labelWithString(&NSString::from_str("Default"), mtm);
            note.setFont(Some(&TextRole::Caption.font()));
            note.setTextColor(Some(&color::secondary_label()));
            note.sizeToFit();
            let size = note.frame().size;
            right -= size.width + space::SM;
            note.setFrameOrigin(NSPoint::new(
                right,
                ((ROW_HEIGHT - size.height) / 2.0).round(),
            ));
            row.addSubview(&note);
        }
        label.setFrame(NSRect::new(
            NSPoint::new(space::MD, ((ROW_HEIGHT - label_height) / 2.0).round()),
            NSSize::new((right - space::MD * 2.0).max(0.0), label_height),
        ));
        row.addSubview(&label);
        // SAFETY: the target implements `answer:`, which takes a sender, and the sheet keeps
        // the target alive for as long as its rows.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::new(),
                Some(&self.target),
                Some(sel!(answer:)),
                mtm,
            )
        };
        button.setBordered(false);
        button.setTransparent(true);
        button.setRefusesFirstResponder(true);
        button.setTag(isize::try_from(index).unwrap_or(isize::MAX));
        button.setFrame(row.bounds());
        row.addSubview(&button);
        row
    }
}

/// A sheet's header: who asks, `developer.` `apple.com` `:8443` in SF Mono at the title size
/// with the registrable domain bold, then `then` (` says`) in the title font.
#[must_use]
pub fn header(before: &str, owner: &str, after: &str, then: &str) -> StyledText {
    let size = TextRole::Title.point_size();
    // SAFETY: the weights are immutable `CGFloat`s exported by AppKit.
    let (regular, semibold) = unsafe {
        (
            objc2_app_kit::NSFontWeightRegular,
            objc2_app_kit::NSFontWeightSemibold,
        )
    };
    let mono = objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(size, regular);
    let bold = objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(size, semibold);
    let whole = format!("{before}{owner}{after}{then}");
    let styled = StyledText::new(&whole, &mono, &color::secondary_label());
    let start = utf16(before);
    let owner_length = utf16(owner);
    styled.style(
        objc2_foundation::NSRange::new(start, owner_length),
        &bold,
        &color::label(),
    );
    styled.style(
        objc2_foundation::NSRange::new(start + owner_length + utf16(after), utf16(then)),
        &TextRole::Title.font(),
        &color::label(),
    );
    styled
}

fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A label that wraps at `width`.
fn wrapping(
    text: &str,
    role: TextRole,
    dimmed: bool,
    width: CGFloat,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&role.font()));
    let text = if dimmed {
        color::secondary_label()
    } else {
        color::label()
    };
    label.setTextColor(Some(&text));
    label.setPreferredMaxLayoutWidth(width);
    label.setSelectable(false);
    label
}

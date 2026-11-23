//! The E3 sheet (`docs/design/system.md`, Elevation): tinted glass attached to the top edge of
//! the window, the page dimmed 30 % under it, sliding down with `sheet-enter`. A sheet stops the
//! user for a decision a page asked for, so it always animates, even when a key brought it up,
//! and leaves with `fade-fast`.
//!
//! Its content is a column (`docs/design/mocks/system/permission/`): a tinted icon tile beside
//! the header (who asks, set large) and the lines of text, an optional card, an optional field,
//! and the answers as buttons that carry their keys, so no hint bar repeats them. The buttons
//! are click targets; the keys themselves are the caller's to route.

mod answers;
mod card;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSLineBreakMode, NSSecureTextField, NSTextField, NSView,
    NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, ns_string};

pub use answers::{BUTTON_HEIGHT, keyed_button};
pub use card::{SheetCard, SheetIcon};

use crate::controls::StyledText;
use crate::glass::{Elevation, GlassSurface, Scrim};
use crate::motion::{self, Trigger};
use crate::tokens::{TextRole, color, overlay, space};

/// How an answer looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    /// What `⏎` does: filled with the accent, across the sheet.
    Primary,
    /// A secondary button, paired with the next plain one.
    Plain,
    /// The safe default (`⎋` blocks, refuses): filled red, across the sheet, named as such.
    Default,
    /// What the agent asks of the user next (set up an endpoint, try again): filled with the
    /// agent color. Outside sheets only.
    Agent,
}

/// One answer: its label, the keys that give it, and how it looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetAnswer {
    pub label: String,
    /// In glyphs: `⏎`, `⎋`, `1`, `⌘B`.
    pub keys: String,
    pub emphasis: Emphasis,
    /// An SF Symbol before the label of a filled answer: `nosign` for Block.
    pub symbol: Option<&'static str>,
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
    /// The tile beside the header and the lines.
    pub icon: Option<SheetIcon>,
    pub lines: Vec<SheetLine>,
    /// A card under the lines.
    pub card: Option<SheetCard>,
    /// A text field holding this text, focused and selected.
    pub field: Option<String>,
    pub answers: Vec<SheetAnswer>,
}

/// Longest a message may run before it is cut, in points.
const MAX_LINES_HEIGHT: CGFloat = 320.0;

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

#[derive(Clone, Copy)]
enum Presentation {
    Overlay,
    Window { secure: bool },
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
        let (height, field) = self.fill(
            surface.content_view(),
            content,
            width,
            Presentation::Overlay,
        );
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

    /// Fills a native `AppKit` sheet window with the same E3 surface and buttons.
    pub fn show_in_window(
        &self,
        window: &NSWindow,
        content: &SheetContent,
        secure: bool,
        clicked: impl Fn(usize) + 'static,
    ) {
        self.remove();
        *self.target.ivars().clicked.borrow_mut() = Some(Rc::new(clicked));
        let surface = GlassSurface::new(Elevation::E3, self.mtm);
        let width = overlay::SHEET_WIDTH;
        let (height, field) = self.fill(
            surface.content_view(),
            content,
            width,
            Presentation::Window { secure },
        );
        let size = NSSize::new(width, height);
        surface.set_frame(NSRect::new(NSPoint::ZERO, size));
        window.setContentSize(size);
        window.setContentView(Some(surface.view()));
        if let Some(field) = &field {
            window.setInitialFirstResponder(Some(field));
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
        presentation: Presentation,
    ) -> (CGFloat, Option<Retained<NSTextField>>) {
        let mtm = self.mtm;
        let padding = space::PALETTE_PADDING;
        let inner = width - padding * 2.0;
        // Laid out downwards from y = 0, then flipped once the height is known.
        let mut placed: Vec<(Retained<NSView>, NSRect)> = Vec::new();
        let mut y = padding;

        // The tile, and the header and lines in a column beside it.
        let column = match &content.icon {
            Some(icon) => {
                placed.push((
                    card::tile(icon, mtm),
                    NSRect::new(
                        NSPoint::new(padding, y),
                        NSSize::new(card::TILE, card::TILE),
                    ),
                ));
                padding + card::TILE + space::MD
            }
            None => padding,
        };
        let column_width = width - padding - column;
        let top = y;
        let header = NSTextField::labelWithAttributedString(content.header.string(), mtm);
        header.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        let header_height = header.fittingSize().height.ceil();
        placed.push((
            header.into_super().into_super(),
            NSRect::new(
                NSPoint::new(column, y),
                NSSize::new(column_width, header_height),
            ),
        ));
        y += header_height + space::XS;

        let mut lines_height = 0.0;
        for line in &content.lines {
            let label = wrapping(&line.text, line.role, line.dimmed, column_width, mtm);
            let height = label
                .fittingSize()
                .height
                .ceil()
                .min(MAX_LINES_HEIGHT - lines_height)
                .max(0.0);
            lines_height += height + space::XS;
            placed.push((
                label.into_super().into_super(),
                NSRect::new(NSPoint::new(column, y), NSSize::new(column_width, height)),
            ));
            y += height + space::XS;
        }
        if content.icon.is_some() {
            y = y.max(top + card::TILE + space::XS);
        }
        y += space::MD;

        if let Some(said) = &content.card {
            let card = card::card(said, inner, mtm);
            let height = card.frame().size.height;
            placed.push((
                card,
                NSRect::new(NSPoint::new(padding, y), NSSize::new(inner, height)),
            ));
            y += height + space::LG;
        }

        let field = content.field.as_deref().map(|text| {
            let field = match presentation {
                Presentation::Window { secure: true } => {
                    let field = NSSecureTextField::new(mtm);
                    field.setEditable(true);
                    field.setSelectable(true);
                    field.setPlaceholderString(Some(ns_string!("Security key PIN")));
                    field.into_super()
                }
                _ => NSTextField::textFieldWithString(&NSString::from_str(text), mtm),
            };
            field.setFont(Some(&TextRole::RowTitle.font()));
            let height = field.fittingSize().height.ceil();
            placed.push((
                field.clone().into_super().into_super(),
                NSRect::new(NSPoint::new(padding, y), NSSize::new(inner, height)),
            ));
            y += height + space::LG;
            field
        });

        y = self.place_answers(
            &content.answers,
            &mut placed,
            padding,
            inner,
            y,
            presentation,
        );
        let height = (y - space::SM + padding).ceil();
        for (subview, frame) in placed {
            subview.setFrame(NSRect::new(
                NSPoint::new(frame.origin.x, height - frame.origin.y - frame.size.height),
                frame.size,
            ));
            view.addSubview(&subview);
        }
        (height, field)
    }

    /// Adds the answers' buttons to `placed`, row by row from `y` down, `inner` wide from
    /// `left`; returns the `y` under the last row.
    fn place_answers(
        &self,
        answers: &[SheetAnswer],
        placed: &mut Vec<(Retained<NSView>, NSRect)>,
        left: CGFloat,
        inner: CGFloat,
        mut y: CGFloat,
        presentation: Presentation,
    ) -> CGFloat {
        let target: &AnyObject = self.target.as_ref();
        let emphases: Vec<Emphasis> = answers.iter().map(|answer| answer.emphasis).collect();
        for row in answers::rows(&emphases) {
            let count = CGFloat::from(u32::try_from(row.len()).unwrap_or(1));
            let each = ((inner - space::SM * (count - 1.0)) / count).floor();
            let mut x = left;
            for index in row {
                let Some(answer) = answers.get(index) else {
                    continue;
                };
                let native_keys = matches!(presentation, Presentation::Window { .. });
                let button = answers::button(answer, index, each, target, self.mtm, native_keys);
                placed.push((
                    button,
                    NSRect::new(
                        NSPoint::new(x, y),
                        NSSize::new(each, answers::BUTTON_HEIGHT),
                    ),
                ));
                x += each + space::SM;
            }
            y += answers::BUTTON_HEIGHT + space::SM;
        }
        y
    }
}

/// A sheet's header: who asks, `developer.` `apple.com` `:8443` in SF Mono at the title size
/// with the registrable domain bold, then `then` (` says`) in the title font.
#[must_use]
pub fn header(before: &str, owner: &str, after: &str, then: &str) -> StyledText {
    let size = TextRole::Title.point_size();
    let mono = objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(size, regular());
    let bold = objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(size, semibold());
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

/// A sheet's header in words, not an origin: `before` and `after` in the title font, `name`
/// (an extension's) semibold between them: `Add` **`Bitwarden`**`?`.
#[must_use]
pub fn named_header(before: &str, name: &str, after: &str) -> StyledText {
    let size = TextRole::Title.point_size();
    let whole = format!("{before}{name}{after}");
    let styled = StyledText::new(
        &whole,
        &objc2_app_kit::NSFont::systemFontOfSize_weight(size, regular()),
        &color::label(),
    );
    styled.style(
        objc2_foundation::NSRange::new(utf16(before), utf16(name)),
        &objc2_app_kit::NSFont::systemFontOfSize_weight(size, semibold()),
        &color::label(),
    );
    styled
}

fn regular() -> CGFloat {
    // SAFETY: an immutable `CGFloat` exported by AppKit.
    unsafe { objc2_app_kit::NSFontWeightRegular }
}

fn semibold() -> CGFloat {
    // SAFETY: an immutable `CGFloat` exported by AppKit.
    unsafe { objc2_app_kit::NSFontWeightSemibold }
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

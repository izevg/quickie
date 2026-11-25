//! The answer's text view: read-only, selectable, scrolling. Each [`Edit`] replaces only the
//! end of its text storage, so the layout manager lays out the changed line, not the answer.
//!
//! Links never open on their own. A click on one reaches the panel's delegate, which hands an
//! `http` or `https` URL to the browser (a new tab) and tells the text view it was handled, so
//! it never asks the system to open anything. Automatic link and data detection are off, so
//! no address in the answer becomes a link unless the answer wrote it as one.

use std::cell::RefCell;
use std::rc::Rc;

use crate::tokens::{TextRole, color, space};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
};
use objc2_app_kit::{
    NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName, NSColor, NSFont,
    NSFontAttributeName, NSFontDescriptorSymbolicTraits, NSForegroundColorAttributeName,
    NSLinkAttributeName, NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSScrollView,
    NSStringDrawingOptions, NSTextDelegate, NSTextView, NSTextViewDelegate, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSMutableAttributedString, NSRange, NSRect, NSSize, NSString, NSURL};
use url::Url;

use super::markdown::{Block, Paragraph, Span, Style};
use super::transcript::Edit;

type LinkHandler = Rc<dyn Fn(Url)>;

pub struct Ivars {
    handler: RefCell<Option<LinkHandler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `LinkDelegate` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAnswerLinkDelegate"]
    #[ivars = Ivars]
    pub struct LinkDelegate;

    unsafe impl NSObjectProtocol for LinkDelegate {}

    unsafe impl NSTextDelegate for LinkDelegate {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSTextViewDelegate for LinkDelegate {
        #[unsafe(method(textView:clickedOnLink:atIndex:))]
        fn clicked(&self, _view: &NSTextView, link: &AnyObject, _index: usize) -> bool {
            let address = link
                .downcast_ref::<NSURL>()
                .and_then(NSURL::absoluteString)
                .map(|address| address.to_string())
                .or_else(|| link.downcast_ref::<NSString>().map(NSString::to_string));
            let web = address
                .and_then(|address| Url::parse(&address).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https"));
            let handler = self.ivars().handler.borrow().clone();
            if let (Some(url), Some(handler)) = (web, handler) {
                handler(url);
            }
            // Handled, whatever it was: the text view never opens a link itself.
            true
        }
    }
);

impl LinkDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// Selectable answer text, with its own scroll view or embedded in a transcript.
pub struct AnswerText {
    scroll: Option<Retained<NSScrollView>>,
    text: Retained<NSTextView>,
    delegate: Retained<LinkDelegate>,
    look: RefCell<Look>,
}

impl AnswerText {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let scroll = NSTextView::scrollableTextView(mtm);
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        let text = scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .unwrap_or_else(|| NSTextView::new(mtm));
        Self::configure(text, Some(scroll), mtm)
    }

    /// Text placed directly in a transcript that already handles scrolling.
    #[must_use]
    pub fn embedded(mtm: MainThreadMarker) -> Self {
        let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::ZERO);
        text.setHorizontallyResizable(false);
        text.setVerticallyResizable(false);
        // SAFETY: a getter. The text view owns its default layout container.
        if let Some(container) = unsafe { text.textContainer() } {
            container.setWidthTracksTextView(true);
            container.setHeightTracksTextView(true);
        }
        Self::configure(text, None, mtm)
    }

    fn configure(
        text: Retained<NSTextView>,
        scroll: Option<Retained<NSScrollView>>,
        mtm: MainThreadMarker,
    ) -> Self {
        text.setEditable(false);
        text.setSelectable(true);
        text.setRichText(true);
        text.setDrawsBackground(false);
        text.setAutomaticLinkDetectionEnabled(false);
        text.setAutomaticDataDetectionEnabled(false);
        text.setTextContainerInset(NSSize::new(0.0, 0.0));
        let delegate = LinkDelegate::new(mtm);
        text.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Self {
            scroll,
            text,
            delegate,
            look: RefCell::new(Look::now()),
        }
    }

    /// The view to place in the panel.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.scroll.as_deref().map_or(&self.text, |scroll| scroll)
    }

    /// Runs `handler` with the web link the user clicked.
    pub fn on_link(&self, handler: impl Fn(Url) + 'static) {
        *self.delegate.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// The whole text, as shown, for copying.
    #[must_use]
    pub fn string(&self) -> String {
        self.text.string().to_string().replace('\u{2028}', "\n")
    }

    /// Empties the text, for the next answer.
    pub fn clear(&self) {
        self.text.setString(&NSString::new());
        *self.look.borrow_mut() = Look::now();
    }

    /// Shows `edit`: the text from its start on is replaced. The view follows the end while
    /// the user has not scrolled up.
    pub fn apply(&self, edit: &Edit) {
        // SAFETY: a getter; a text view always has a text storage.
        let Some(storage) = (unsafe { self.text.textStorage() }) else {
            return;
        };
        let length = storage.length();
        let from = edit.from.min(length);
        let following = self.scroll.as_ref().is_some_and(|scroll| {
            let visible = scroll.contentView().documentVisibleRect();
            visible.origin.y + visible.size.height >= self.text.frame().size.height - 4.0
        });
        let replacement = self.look.borrow().attributed(&edit.spans);
        storage.beginEditing();
        storage.replaceCharactersInRange_withAttributedString(
            NSRange::new(from, length - from),
            &replacement,
        );
        storage.endEditing();
        if following {
            self.text
                .scrollRangeToVisible(NSRange::new(storage.length(), 0));
        }
    }

    /// How tall the whole text is when laid out `width` wide: the panel grows to it.
    #[must_use]
    pub fn content_height(&self, width: CGFloat) -> CGFloat {
        // SAFETY: a getter; a text view always has a text storage.
        let Some(storage) = (unsafe { self.text.textStorage() }) else {
            return 0.0;
        };
        if storage.length() == 0 {
            return 0.0;
        }
        // The text container keeps a line fragment padding on each side.
        // SAFETY: a getter; a text view made by `scrollableTextView` has a text container.
        let container = unsafe { self.text.textContainer() };
        let padding = container.map_or(0.0, |container| container.lineFragmentPadding());
        let bounds = storage.boundingRectWithSize_options_context(
            NSSize::new((width - padding * 2.0).max(1.0), 100_000.0),
            NSStringDrawingOptions::UsesLineFragmentOrigin
                | NSStringDrawingOptions::UsesFontLeading,
            None,
        );
        bounds.size.height.ceil()
    }
}

/// The fonts and colors of an answer, looked up once: a font lookup costs more than the edit
/// it styles. Taken again for each answer, so a change of text size reaches the next one.
struct Look {
    body: Retained<NSFont>,
    strong: Retained<NSFont>,
    italic: Retained<NSFont>,
    strong_italic: Retained<NSFont>,
    title: Retained<NSFont>,
    heading: Retained<NSFont>,
    code: Retained<NSFont>,
    note: Retained<NSFont>,
    label: Retained<NSColor>,
    accent: Retained<NSColor>,
    secondary: Retained<NSColor>,
    separator: Retained<NSColor>,
    problem: Retained<NSColor>,
    quote_fill: Retained<NSColor>,
}

impl Look {
    fn now() -> Self {
        let body = TextRole::RowTitle.font();
        let strong = bold(&body);
        Self {
            strong_italic: italic(&strong),
            strong,
            italic: italic(&body),
            title: bold(&TextRole::Title.font()),
            heading: bold(&TextRole::Headline.font()),
            code: TextRole::RowTitle.mono_font(),
            note: TextRole::RowDetail.font(),
            body,
            label: color::label(),
            accent: color::accent(),
            secondary: color::secondary_label(),
            separator: color::separator(),
            problem: color::destructive(),
            quote_fill: color::agent().colorWithAlphaComponent(0.18),
        }
    }

    /// `spans` as one attributed string.
    fn attributed(&self, spans: &[Span]) -> Retained<NSMutableAttributedString> {
        let whole: String = spans.iter().map(|span| span.text.as_str()).collect();
        let string = NSMutableAttributedString::initWithString(
            NSMutableAttributedString::alloc(),
            &NSString::from_str(&whole),
        );
        let mut at = 0;
        let mut paragraph: Option<(usize, Paragraph)> = None;
        for span in spans {
            if let Some(layout) = span.paragraph {
                if let Some((start, previous)) = paragraph {
                    apply_paragraph(&string, NSRange::new(start, at - start), previous);
                }
                paragraph = Some((at, layout));
            }
            let length = span.text.encode_utf16().count();
            self.style(
                &string,
                NSRange::new(at, length),
                &span.style,
                paragraph.map(|(_, layout)| layout.block),
            );
            at += length;
        }
        if let Some((start, layout)) = paragraph {
            apply_paragraph(&string, NSRange::new(start, at - start), layout);
        }
        string
    }

    fn style(
        &self,
        string: &NSMutableAttributedString,
        range: NSRange,
        style: &Style,
        block: Option<Block>,
    ) {
        let heading = match block {
            Some(Block::Title) => Some(&self.title),
            Some(Block::Heading) => Some(&self.heading),
            _ => None,
        };
        let slanted_heading = heading
            .filter(|_| matches!(style, Style::Emphasis | Style::StrongEmphasis))
            .map(|font| italic(font));
        let (font, tint) = match style {
            Style::Body | Style::Quote => (heading.unwrap_or(&self.body), &self.label),
            Style::Strong | Style::Heading => (heading.unwrap_or(&self.strong), &self.label),
            Style::Emphasis => (
                slanted_heading.as_ref().unwrap_or(&self.italic),
                &self.label,
            ),
            Style::StrongEmphasis => (
                slanted_heading.as_ref().unwrap_or(&self.strong_italic),
                &self.label,
            ),
            Style::Code => (&self.code, &self.label),
            Style::Rule => (&self.code, &self.separator),
            Style::Link(_) => (heading.unwrap_or(&self.body), &self.accent),
            Style::Note => (&self.note, &self.secondary),
            Style::Problem => (&self.body, &self.problem),
        };
        // SAFETY: immutable `NSString` constants exported by AppKit.
        let (font_key, color_key) =
            unsafe { (NSFontAttributeName, NSForegroundColorAttributeName) };
        // SAFETY: the font attribute takes an `NSFont`, and the range lies inside the string.
        unsafe { string.addAttribute_value_range(font_key, font, range) };
        // SAFETY: the color attribute takes an `NSColor`, and the range lies inside the string.
        unsafe { string.addAttribute_value_range(color_key, tint, range) };
        match style {
            Style::Link(url) => {
                if let Some(link) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
                    // SAFETY: an immutable `NSString` constant exported by AppKit.
                    let key = unsafe { NSLinkAttributeName };
                    // SAFETY: the link attribute takes an `NSURL`; the range lies inside.
                    unsafe { string.addAttribute_value_range(key, &link, range) };
                }
            }
            Style::Quote => {
                // SAFETY: an immutable `NSString` constant exported by AppKit.
                let key = unsafe { NSBackgroundColorAttributeName };
                // SAFETY: the background attribute takes an `NSColor`; the range lies inside.
                unsafe { string.addAttribute_value_range(key, &self.quote_fill, range) };
            }
            Style::Body
            | Style::Strong
            | Style::Emphasis
            | Style::StrongEmphasis
            | Style::Code
            | Style::Heading
            | Style::Rule
            | Style::Note
            | Style::Problem => {}
        }
    }
}

fn apply_paragraph(string: &NSMutableAttributedString, range: NSRange, layout: Paragraph) {
    let style = NSMutableParagraphStyle::new();
    let list_indent = CGFloat::from(layout.list_depth) * space::LG;
    let quote_indent = CGFloat::from(layout.quote_depth) * space::MD;
    let code_indent = if layout.block == Block::Code {
        space::SM
    } else {
        0.0
    };
    let indent = list_indent + quote_indent + code_indent;
    style.setHeadIndent(indent);
    style.setFirstLineHeadIndent(if layout.marker {
        indent - space::LG
    } else {
        indent
    });
    style.setLineSpacing(space::XS / 2.0);
    let spacing = match layout.block {
        Block::Title | Block::Heading => {
            style.setParagraphSpacingBefore(space::SM);
            space::XS
        }
        Block::Rule => {
            style.setParagraphSpacingBefore(space::XS);
            space::MD
        }
        Block::TableRow => space::XS,
        Block::Body if layout.list_depth != 0 => space::XS,
        Block::Body | Block::Code => space::SM,
    };
    style.setParagraphSpacing(spacing);
    // SAFETY: the attribute takes an `NSParagraphStyle`, and the range is a complete paragraph
    // within the attributed string.
    unsafe { string.addAttribute_value_range(NSParagraphStyleAttributeName, &style, range) };
}

/// `font` in italics; `font` itself when the system has no italic of it.
fn italic(font: &NSFont) -> Retained<NSFont> {
    let descriptor = font.fontDescriptor();
    let traits = descriptor.symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic;
    let descriptor = descriptor.fontDescriptorWithSymbolicTraits(traits);
    NSFont::fontWithDescriptor_size(&descriptor, font.pointSize()).unwrap_or_else(|| font.retain())
}

/// The system font at the size of `font`, semibold.
fn bold(font: &NSFont) -> Retained<NSFont> {
    let size = font.pointSize();
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let weight = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::systemFontOfSize_weight(size, weight)
}

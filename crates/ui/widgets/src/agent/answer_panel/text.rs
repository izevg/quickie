//! The answer's text view: read-only, selectable, scrolling. Each [`Edit`] replaces only the
//! end of its text storage, so the layout manager lays out the changed line, not the answer.
//!
//! Links never open on their own. A click on one reaches the panel's delegate, which hands an
//! `http` or `https` URL to the browser (a new tab) and tells the text view it was handled, so
//! it never asks the system to open anything. Automatic link and data detection are off, so
//! no address in the answer becomes a link unless the answer wrote it as one.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSBackgroundColorAttributeName, NSColor, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSLinkAttributeName, NSScrollView, NSTextDelegate, NSTextView,
    NSTextViewDelegate, NSView,
};
use objc2_foundation::{NSMutableAttributedString, NSRange, NSSize, NSString, NSURL};
use quickie_ui_shared::tokens::{TextRole, color};
use url::Url;

use super::markdown::{Span, Style};
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

/// The scrolling text of an answer.
pub struct AnswerText {
    scroll: Retained<NSScrollView>,
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
        &self.scroll
    }

    /// Runs `handler` with the web link the user clicked.
    pub fn on_link(&self, handler: impl Fn(Url) + 'static) {
        *self.delegate.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// The whole text, as shown, for copying.
    #[must_use]
    pub fn string(&self) -> String {
        self.text.string().to_string()
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
        let following = self.at_end();
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

    /// True when the end of the text is in view, or there is nothing to scroll.
    fn at_end(&self) -> bool {
        let visible = self.scroll.contentView().documentVisibleRect();
        let height = self.text.frame().size.height;
        visible.origin.y + visible.size.height >= height - 4.0
    }
}

/// The fonts and colors of an answer, looked up once: a font lookup costs more than the edit
/// it styles. Taken again for each answer, so a change of text size reaches the next one.
struct Look {
    body: Retained<NSFont>,
    strong: Retained<NSFont>,
    code: Retained<NSFont>,
    note: Retained<NSFont>,
    label: Retained<NSColor>,
    accent: Retained<NSColor>,
    secondary: Retained<NSColor>,
    problem: Retained<NSColor>,
    quote_fill: Retained<NSColor>,
}

impl Look {
    fn now() -> Self {
        let body = TextRole::RowTitle.font();
        Self {
            strong: bold(&body),
            code: TextRole::RowTitle.mono_font(),
            note: TextRole::RowDetail.font(),
            body,
            label: color::label(),
            accent: color::accent(),
            secondary: color::secondary_label(),
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
        for span in spans {
            let length = span.text.encode_utf16().count();
            self.style(&string, NSRange::new(at, length), &span.style);
            at += length;
        }
        string
    }

    fn style(&self, string: &NSMutableAttributedString, range: NSRange, style: &Style) {
        let (font, tint) = match style {
            Style::Body | Style::Quote => (&self.body, &self.label),
            Style::Strong | Style::Heading => (&self.strong, &self.label),
            Style::Code => (&self.code, &self.label),
            Style::Link(_) => (&self.body, &self.accent),
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
            | Style::Code
            | Style::Heading
            | Style::Note
            | Style::Problem => {}
        }
    }
}

/// The system font at the size of `font`, semibold.
fn bold(font: &NSFont) -> Retained<NSFont> {
    let size = font.pointSize();
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let weight = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::systemFontOfSize_weight(size, weight)
}

//! The find bar (`⌘F`): an E2 glass pill at the top right of the window with the query field,
//! the match count ("3 / 12", read as "3 of 12"), previous and next, and close
//! (`docs/design/mocks/navigation/find/`). It is opened from the keyboard, so it appears and
//! disappears instantly.
//!
//! It dims nothing and swallows no click: the user reads the page through the matches while
//! the bar is up, so it floats like an E2 but leaves the page as it is.
//!
//! The bar only shows: the browser runs the find on each edit and on `⏎` / `⇧⏎`, `⌘G` /
//! `⇧⌘G`, and the buttons, and answers with [`FindBar::set_matches`].

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSControlTextDidChangeNotification,
    NSFocusRingType, NSTextAlignment, NSTextField, NSTextInputClient, NSTextView, NSView, NSWindow,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSPoint, NSRange, NSRect, NSSize, NSString,
};
use quickie_domain_browsing::FindMatches;
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Elevation, GlassSurface, StyledText, Trigger, motion, symbol};

/// The bar's size: a pill, its height twice the overlay radius.
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 44.0;
/// Room for "999 of 999".
const COUNT_WIDTH: f64 = 84.0;
/// A round button around a symbol.
const BUTTON: f64 = 24.0;

/// What the bar asks of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindRequest {
    /// The query changed: find it from the current match.
    Search,
    /// The next match; the previous one when `backwards`.
    Step { backwards: bool },
    /// Close the bar.
    Close,
}

type Handler = Rc<dyn Fn(FindRequest)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `FindBarTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieFindBarTarget"]
    #[ivars = Ivars]
    pub struct FindBarTarget;

    unsafe impl NSObjectProtocol for FindBarTarget {}

    impl FindBarTarget {
        #[unsafe(method(previous:))]
        fn previous(&self, _sender: Option<&NSButton>) {
            self.ask(FindRequest::Step { backwards: true });
        }

        #[unsafe(method(next:))]
        fn next(&self, _sender: Option<&NSButton>) {
            self.ask(FindRequest::Step { backwards: false });
        }

        #[unsafe(method(close:))]
        fn close(&self, _sender: Option<&NSButton>) {
            self.ask(FindRequest::Close);
        }
    }
);

impl FindBarTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn ask(&self, request: FindRequest) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }
}

pub struct FindBar {
    surface: GlassSurface,
    field: Retained<NSTextField>,
    count: Retained<NSTextField>,
    target: Retained<FindBarTarget>,
    /// Keeps the field's change observer registered; removed on drop.
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    open: Cell<bool>,
}

impl FindBar {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let target = FindBarTarget::new(mtm);
        let field = NSTextField::new(mtm);
        field.setFont(Some(&TextRole::RowTitle.font()));
        field.setTextColor(Some(&color::label()));
        field.setPlaceholderString(Some(&NSString::from_str("Find in page")));
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setUsesSingleLineMode(true);
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        field.sizeToFit();
        let count = NSTextField::labelWithString(&NSString::new(), mtm);
        count.setFont(Some(&TextRole::RowDetail.mono_font()));
        count.setTextColor(Some(&color::secondary_label()));
        count.setAlignment(NSTextAlignment::Right);
        let observer = observe_edits(&field, &target);
        let bar = Self {
            surface,
            field,
            count,
            target,
            observer,
            open: Cell::new(false),
        };
        bar.lay_out(mtm);
        bar
    }

    /// Runs `handler` with what the user asks: an edit, a step, or closing.
    pub fn on_request(&self, handler: impl Fn(FindRequest) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// True while the query field has the keyboard.
    #[must_use]
    pub fn has_focus(&self) -> bool {
        self.is_open() && self.field.currentEditor().is_some()
    }

    /// True while an input method is composing in the field: ⏎ then commits the composition.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Opens at the top right of `host`, or takes the keyboard back when open; the query is
    /// selected, ready to be typed over.
    pub fn show(&self, host: &NSView, window: &NSWindow) {
        // SAFETY: a getter; the superview is retained for the duration of the comparison.
        let parent = unsafe { self.surface.view().superview() };
        if !self.is_open() || parent.is_none_or(|parent| &*parent != host) {
            self.surface.view().removeFromSuperview();
            self.place(host);
            host.addSubview(self.surface.view());
            motion::show(
                self.surface.view(),
                motion::OVERLAY_ENTER,
                Trigger::Keyboard,
            );
            self.count.setStringValue(&NSString::new());
            self.count.setAccessibilityLabel(None);
            self.open.set(true);
        }
        window.makeFirstResponder(Some(&self.field));
        // SAFETY: `selectText:` accepts any sender, including none.
        unsafe { self.field.selectText(None) };
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        motion::hide(
            self.surface.view(),
            motion::OVERLAY_ENTER,
            Trigger::Keyboard,
        );
        self.surface.view().removeFromSuperview();
        self.open.set(false);
    }

    /// The query.
    #[must_use]
    pub fn query(&self) -> String {
        self.field.stringValue().to_string()
    }

    /// Replaces the query, as a test or a restored find would.
    pub fn set_query(&self, query: &str) {
        self.field.setStringValue(&NSString::from_str(query));
    }

    /// Shows how many matches the query has and which is current, `3 / 17` with the total
    /// dimmed; nothing for an empty query. `VoiceOver` reads `3 of 17`.
    pub fn set_matches(&self, matches: FindMatches) {
        let spoken = if self.query().is_empty() {
            String::new()
        } else {
            matches.to_string()
        };
        let font = TextRole::RowDetail.mono_font();
        let shown = match (matches.current(), matches.total()) {
            _ if spoken.is_empty() => StyledText::new("", &font, &color::label()),
            (Some(current), total) if total > 0 => {
                let current = current.to_string();
                let styled = StyledText::new(
                    &format!("{current} / {total}"),
                    &font,
                    &color::tertiary_label(),
                );
                styled.style(
                    NSRange::new(0, current.encode_utf16().count()),
                    &font,
                    &color::label(),
                );
                styled
            }
            (_, 0) => StyledText::new(&spoken, &font, &color::destructive()),
            _ => StyledText::new(&spoken, &font, &color::secondary_label()),
        };
        self.count.setAttributedStringValue(shown.string());
        self.count
            .setAccessibilityLabel(Some(&NSString::from_str(&spoken)));
    }

    /// The match count as `VoiceOver` reads it: `3 of 12`, `No matches`.
    #[must_use]
    pub fn matches_text(&self) -> String {
        self.count
            .accessibilityLabel()
            .map_or_else(String::new, |label| label.to_string())
    }

    /// The bar's surface, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// Lays the parts out once, left to right: glyph, field, count, previous, next, close.
    fn lay_out(&self, mtm: MainThreadMarker) {
        let content = self.surface.content_view();
        let middle = |height: f64| ((HEIGHT - height) / 2.0).round();
        let mut x = space::LG;
        if let Some(glyph) = symbol(
            "magnifyingglass",
            "Find",
            TextRole::RowTitle,
            &color::accent(),
            mtm,
        ) {
            let size = glyph.frame().size;
            glyph.setFrameOrigin(NSPoint::new(x, middle(size.height)));
            content.addSubview(&glyph);
            x += size.width + space::SM;
        }
        let buttons = [
            ("chevron.up", "Previous match", sel!(previous:)),
            ("chevron.down", "Next match", sel!(next:)),
            ("xmark", "Close", sel!(close:)),
        ];
        let buttons_width = BUTTON * 3.0 + space::XS * 2.0;
        let right = WIDTH - space::SM - buttons_width;
        let field_width = right - space::SM - COUNT_WIDTH - space::SM - x;
        let field_height = self.field.frame().size.height;
        self.field.setFrame(NSRect::new(
            NSPoint::new(x, middle(field_height)),
            NSSize::new(field_width, field_height),
        ));
        content.addSubview(&self.field);
        self.count.sizeToFit();
        let count_height = self.count.frame().size.height.max(field_height);
        self.count.setFrame(NSRect::new(
            NSPoint::new(x + field_width + space::SM, middle(count_height)),
            NSSize::new(COUNT_WIDTH, count_height),
        ));
        content.addSubview(&self.count);
        let mut x = right;
        for (name, description, action) in buttons {
            if let Some(button) = button(name, description, action, &self.target, mtm) {
                button.setFrame(NSRect::new(
                    NSPoint::new(x, middle(BUTTON)),
                    NSSize::new(BUTTON, BUTTON),
                ));
                content.addSubview(&button);
            }
            x += BUTTON + space::XS;
        }
    }

    /// Top right of `host`, `space-lg` from the top and `space-xl` from the right edge.
    fn place(&self, host: &NSView) {
        let bounds = host.bounds();
        let width = WIDTH.min(bounds.size.width - space::XL * 2.0);
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                (bounds.size.width - space::XL - width).round(),
                (bounds.size.height - space::LG - HEIGHT).round(),
            ),
            NSSize::new(width, HEIGHT),
        ));
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
    }
}

impl Drop for FindBar {
    fn drop(&mut self) {
        let observer: &AnyObject = (*self.observer).as_ref();
        // SAFETY: the observer was registered with this center and is removed once.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(observer) };
    }
}

/// Tells `target` of every edit in `field`.
fn observe_edits(
    field: &NSTextField,
    target: &FindBarTarget,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let weak = objc2::rc::Weak::from_retained(&target.retain());
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
        if let Some(target) = weak.load() {
            target.ask(FindRequest::Search);
        }
    });
    // SAFETY: an immutable string constant AppKit defines and never changes.
    let name = unsafe { NSControlTextDidChangeNotification };
    // SAFETY: the object filter is the field, the queue is none, so the block runs on the main
    // thread that posts the notification, where the target lives.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            Some(field),
            None,
            &block,
        )
    }
}

/// A borderless round button showing the symbol `name`, sending `action` to `target`.
fn button(
    name: &str,
    description: &str,
    action: objc2::runtime::Sel,
    target: &FindBarTarget,
    mtm: MainThreadMarker,
) -> Option<Retained<NSButton>> {
    let image = symbol(name, description, TextRole::RowDetail, &color::label(), mtm)?.image()?;
    // SAFETY: `target` implements `action`, which takes a sender, and the bar keeps the target
    // alive for as long as its buttons.
    let button =
        unsafe { NSButton::buttonWithImage_target_action(&image, Some(target), Some(action), mtm) };
    button.setBordered(false);
    button.setContentTintColor(Some(&color::label()));
    // Buttons never take the keyboard from the field.
    button.setRefusesFirstResponder(true);
    Some(button)
}

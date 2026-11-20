//! What every pane is made of: the click target the buttons share, a flipped view to lay out
//! top down, the page with its title in the titlebar's band, a scrolling page for the forms,
//! a row of text with a button for the groups, and the labels and buttons themselves.
//!
//! - [`sidebar`]: the list of panes on the left.
//! - [`row_view`]: how the window's tables draw a row: selected, in conflict, recording.

pub mod row_view;
pub mod sidebar;

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBezelStyle, NSBorderType, NSBox, NSBoxType, NSButton, NSColor, NSFont, NSFontWeightMedium,
    NSImageView, NSLineBreakMode, NSScrollView, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_domain_shields::FilterListId;
use quickie_ui_shared::symbol;
use quickie_ui_shared::tokens::{TextRole, color, space};

use super::SettingsAction;

/// The window's content height; every pane is this tall.
pub const HEIGHT: CGFloat = 600.0;
/// The width of every pane.
pub const PANE_WIDTH: CGFloat = 640.0;
/// The width of a pane's content between its margins.
pub const INNER: CGFloat = PANE_WIDTH - space::XL * 2.0;

type Handler = Rc<dyn Fn(SettingsAction)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `SettingsTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsTarget"]
    #[ivars = Ivars]
    pub struct SettingsTarget;

    unsafe impl NSObjectProtocol for SettingsTarget {}

    impl SettingsTarget {
        #[unsafe(method(makeDefault:))]
        fn make_default(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::MakeDefaultBrowser);
        }

        #[unsafe(method(showDownloads:))]
        fn show_downloads(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::ShowDownloads);
        }

        #[unsafe(method(refreshLists:))]
        fn refresh_lists(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::RefreshLists);
        }

        #[unsafe(method(enableLists:))]
        fn enable_lists(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::EnableLists);
        }

        /// A list's own refresh button: its tag is the list's place in [`FilterListId::ALL`].
        #[unsafe(method(refreshList:))]
        fn refresh_list(&self, sender: Option<&NSButton>) {
            let list = sender
                .and_then(|button| usize::try_from(button.tag()).ok())
                .and_then(|index| FilterListId::ALL.get(index).copied());
            if let Some(list) = list {
                self.ask(SettingsAction::RefreshList(list));
            }
        }

        #[unsafe(method(openKeybindings:))]
        fn open_keybindings(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::OpenKeybindings);
        }

        #[unsafe(method(reloadKeybindings:))]
        fn reload_keybindings(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsAction::ReloadKeybindings);
        }
    }
);

impl SettingsTarget {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// Runs `handler` with what the user asks from now on.
    pub fn set_handler(&self, handler: impl Fn(SettingsAction) + 'static) {
        *self.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn ask(&self, action: SettingsAction) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements, and `Flipped` implements no `Drop`;
    // `isFlipped` is public API meant to be overridden.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsFlipped"]
    pub struct Flipped;

    impl Flipped {
        /// Panes lay out top down.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl Flipped {
    pub fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

/// A pane: its title in the band the titlebar's buttons sit in, a status at the band's right
/// end when the pane has one ("✓ Protection is on"), a hairline under it, and the rest for the
/// pane's content.
pub struct Page {
    view: Retained<Flipped>,
    status: Retained<NSTextField>,
    glyph: RefCell<Option<Retained<NSImageView>>>,
}

impl Page {
    /// The band the titlebar's buttons share with the title.
    pub const HEADER: CGFloat = 52.0;

    pub fn new(title: &str, mtm: MainThreadMarker) -> Self {
        let view = Flipped::new(
            NSRect::new(NSPoint::ZERO, NSSize::new(PANE_WIDTH, HEIGHT)),
            mtm,
        );
        let heading = label(title, TextRole::Headline, &color::label(), mtm);
        let size = heading.frame().size;
        heading.setFrameOrigin(NSPoint::new(
            space::XL,
            ((Self::HEADER - size.height) / 2.0).round(),
        ));
        view.addSubview(&heading);
        view.addSubview(&hairline(
            NSRect::new(
                NSPoint::new(0.0, Self::HEADER),
                NSSize::new(PANE_WIDTH, 1.0),
            ),
            mtm,
        ));
        let status = label("", TextRole::RowDetail, &color::secondary_label(), mtm);
        status.setHidden(true);
        view.addSubview(&status);
        Self {
            view,
            status,
            glyph: RefCell::new(None),
        }
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Shows `text` at the right end of the title band, after the SF Symbol `glyph`, both in
    /// `tint`; `None` takes it away.
    pub fn set_status(&self, status: Option<(&str, &str, &NSColor)>) {
        if let Some(glyph) = self.glyph.borrow_mut().take() {
            glyph.removeFromSuperview();
        }
        let Some((text, glyph_name, tint)) = status else {
            self.status.setHidden(true);
            return;
        };
        set(&self.status, text);
        self.status.setTextColor(Some(tint));
        self.status.sizeToFit();
        self.status.setHidden(false);
        let size = self.status.frame().size;
        let right = PANE_WIDTH - space::XL;
        self.status.setFrameOrigin(NSPoint::new(
            right - size.width,
            ((Self::HEADER - size.height) / 2.0).round(),
        ));
        if let Some(glyph) = symbol(glyph_name, "", TextRole::RowDetail, tint, self.view.mtm()) {
            let glyph_size = glyph.frame().size;
            glyph.setFrameOrigin(NSPoint::new(
                right - size.width - space::XS - glyph_size.width,
                ((Self::HEADER - glyph_size.height) / 2.0).round(),
            ));
            self.view.addSubview(&glyph);
            *self.glyph.borrow_mut() = Some(glyph);
        }
    }

    /// Where the content goes: everything under the header.
    pub fn body() -> NSRect {
        let top = Self::HEADER + 1.0;
        NSRect::new(
            NSPoint::new(0.0, top),
            NSSize::new(PANE_WIDTH, HEIGHT - top),
        )
    }
}

/// A page whose content scrolls: a column [`INNER`] wide, laid out top down by its owner.
pub struct ScrollPage {
    page: Page,
    column: Retained<Flipped>,
}

impl ScrollPage {
    pub fn new(title: &str, mtm: MainThreadMarker) -> Self {
        let page = Page::new(title, mtm);
        let body = Page::body();
        let column = Flipped::new(NSRect::new(NSPoint::ZERO, body.size), mtm);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), body);
        scroll.setDocumentView(Some(&column));
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        page.view().addSubview(&scroll);
        Self { page, column }
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    /// The page's status in the title band; see [`Page::set_status`].
    pub fn set_status(&self, status: Option<(&str, &str, &NSColor)>) {
        self.page.set_status(status);
    }

    /// The column the content goes in; it starts at `space-xl` from the top and the sides.
    pub fn column(&self) -> &NSView {
        &self.column
    }

    /// The column is `content` tall plus its bottom margin, and scrolls when that is more
    /// than the page shows.
    pub fn set_content_height(&self, content: CGFloat) {
        let height = (content + space::XL).max(Page::body().size.height);
        self.column
            .setFrameSize(NSSize::new(PANE_WIDTH, height.ceil()));
    }
}

/// Stacks views top down in a flipped column, `space-xl` in from its edges.
pub struct Column {
    pub top: CGFloat,
}

impl Column {
    pub fn new() -> Self {
        Self { top: space::XL }
    }

    /// The point where the next view goes.
    pub fn origin(&self) -> NSPoint {
        NSPoint::new(space::XL, self.top)
    }

    /// Places `view` at the full inner width, `height` tall.
    pub fn full(&mut self, view: &NSView, height: CGFloat) {
        view.setFrame(NSRect::new(self.origin(), NSSize::new(INNER, height)));
        self.top += height;
    }

    /// A section's heading, and the small gap under it.
    pub fn heading(&mut self, heading: &NSTextField) {
        self.full(heading, heading.frame().size.height);
        self.top += space::SM;
    }

    pub fn gap(&mut self, height: CGFloat) {
        self.top += height;
    }
}

/// A row in a group: an optional symbol, a line of text that wraps, and an optional button at
/// the trailing edge. Its views go in the group it is added to.
pub struct InfoRow {
    symbol: Option<Retained<NSImageView>>,
    text: Retained<NSTextField>,
    button: Option<Retained<NSButton>>,
}

impl InfoRow {
    /// The least a row is tall: one line of a regular push button and its margins.
    pub const MIN_HEIGHT: CGFloat = 44.0;

    pub fn new(
        symbol_name: Option<&str>,
        button: Option<Retained<NSButton>>,
        mtm: MainThreadMarker,
    ) -> Self {
        let symbol = symbol_name
            .and_then(|name| symbol(name, "", TextRole::Headline, &color::secondary_label(), mtm));
        Self {
            symbol,
            text: paragraph("", mtm),
            button,
        }
    }

    /// The row's text, in `font`.
    pub fn text(&self) -> &NSTextField {
        &self.text
    }

    pub fn button(&self) -> Option<&NSButton> {
        self.button.as_deref()
    }

    /// Adds the row's views to `group`.
    pub fn add_to(&self, group: &NSView) {
        if let Some(symbol) = &self.symbol {
            group.addSubview(symbol);
        }
        group.addSubview(&self.text);
        if let Some(button) = &self.button {
            group.addSubview(button);
        }
    }

    fn text_frame(&self, width: CGFloat) -> (CGFloat, CGFloat) {
        let left = self
            .symbol
            .as_ref()
            .map_or(0.0, |symbol| symbol.frame().size.width + space::MD);
        let right = self
            .button
            .as_ref()
            .map_or(0.0, |button| button.frame().size.width + space::MD);
        (left, (width - left - right).max(0.0))
    }

    /// How tall the row is at `width`.
    pub fn height(&self, width: CGFloat) -> CGFloat {
        let (_, text_width) = self.text_frame(width);
        (wrapped_height(&self.text, text_width) + space::MD * 2.0).max(Self::MIN_HEIGHT)
    }

    /// Places the views in `frame`, a row of the group.
    pub fn place(&self, frame: NSRect) {
        let (left, text_width) = self.text_frame(frame.size.width);
        let middle =
            |height: CGFloat| frame.origin.y + ((frame.size.height - height) / 2.0).round();
        if let Some(symbol) = &self.symbol {
            let size = symbol.frame().size;
            symbol.setFrameOrigin(NSPoint::new(frame.origin.x, middle(size.height)));
        }
        let text_height = wrapped_height(&self.text, text_width);
        self.text.setFrame(NSRect::new(
            NSPoint::new(frame.origin.x + left, middle(text_height)),
            NSSize::new(text_width, text_height),
        ));
        if let Some(button) = &self.button {
            let size = button.frame().size;
            button.setFrameOrigin(NSPoint::new(
                frame.origin.x + frame.size.width - size.width,
                middle(size.height),
            ));
        }
    }
}

/// A horizontal hairline.
pub fn hairline(frame: NSRect, mtm: MainThreadMarker) -> Retained<NSBox> {
    let line = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    line.setBoxType(NSBoxType::Separator);
    line
}

/// A one-line label in `role` and `tint`, sized to fit.
pub fn label(
    text: &str,
    role: TextRole,
    tint: &NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

/// A section's heading over a group: `caption`, medium, secondary.
pub fn heading(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = label(text, TextRole::Caption, &color::secondary_label(), mtm);
    // SAFETY: `NSFontWeightMedium` is an immutable `CGFloat` exported by AppKit.
    let medium = unsafe { NSFontWeightMedium };
    field.setFont(Some(&NSFont::systemFontOfSize_weight(
        TextRole::Caption.point_size(),
        medium,
    )));
    field.sizeToFit();
    field
}

/// A label that wraps, in `row-title` and the label color.
pub fn paragraph(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&TextRole::RowTitle.font()));
    field.setTextColor(Some(&color::label()));
    field.setSelectable(false);
    field
}

/// The height `field` needs to wrap within `width`.
pub fn wrapped_height(field: &NSTextField, width: CGFloat) -> CGFloat {
    field
        .cell()
        .map_or(field.frame().size.height, |cell| {
            cell.cellSizeForBounds(NSRect::new(NSPoint::ZERO, NSSize::new(width, 10_000.0)))
                .height
        })
        .ceil()
}

pub fn set(field: &NSTextField, text: &str) {
    field.setStringValue(&NSString::from_str(text));
}

/// A push button that sends `action` to `target`.
pub fn button(
    title: &str,
    action: Sel,
    target: &SettingsTarget,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    let target: &AnyObject = target.as_ref();
    // SAFETY: the target is weak; the screen keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.sizeToFit();
    button
}

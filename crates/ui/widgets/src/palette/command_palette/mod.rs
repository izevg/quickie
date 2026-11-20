//! The command palette (`⇧⌘P`, `⌘K`, `⌘P`): an E2 glass surface 720 wide at 15 % from the top
//! (`docs/design/mocks/navigation/{commands,go-to}`). The input row shows the mode as a prompt
//! (`>` in the accent, a magnifier when going to a page), the query, and an `⎋ esc` keycap,
//! above a hairline; the answers follow in sections with a header and a count each, or a line
//! that says nothing matched; a strip of the prefixes closes it, the current one in the accent.
//! It is opened from the keyboard, so it appears and disappears instantly.
//!
//! The palette only shows: the browser answers each edit with [`CommandPalette::set_items`] and
//! runs what the selected row targets. Its text is the prompt's prefix and the query together,
//! as the palette use case reads it; typing a prefix into the empty field moves it to the
//! prompt, and `⌫` in the empty field takes it away.

mod field;
mod rows;
mod symbols;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSFocusRingType, NSFont, NSImageView,
    NSLineBreakMode, NSTextAlignment, NSTextField, NSTextInputClient, NSTextView, NSView, NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use quickie_application::PaletteItem;
use quickie_domain_browsing::Timestamp;
use quickie_domain_palette::{Mode, Query};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{
    Elevation, GlassSurface, HintStrip, KeycapStyle, ListSection, ListView, Plate, Scrim, Trigger,
    keycap, motion, symbol,
};
use url::Url;

use field::FieldDelegate;
pub use rows::{Favicon, Group, groups, nothing_found, row};

/// At most this many rows show at once; the rest scroll.
pub const VISIBLE_ROWS: usize = 8;

/// How many answers the browser asks for: the list shows [`VISIBLE_ROWS`] and scrolls the rest.
/// An answer this long may have been cut, so its count reads as a floor.
pub const ANSWERS: usize = 50;

/// The prefixes the hint strip names, in the order of their modes.
const HINTS: [(&str, &str); 5] = [
    (">", "commands"),
    ("@", "headings"),
    ("#", "tags"),
    ("?", "agent"),
    ("!", "search"),
];

/// The hints that act on the selected row.
const SELECT: [(&str, &str); 1] = [("⏎", "select")];

/// What the input row is: the prompt, the query field, the strip that names the prefixes.
struct Input {
    prefix: Cell<Option<char>>,
    glyph: Retained<NSTextField>,
    search: Option<Retained<NSImageView>>,
    field: Retained<NSTextField>,
    strip: HintStrip,
}

impl Input {
    /// Prefix and query, as the palette use case reads them.
    fn text(&self) -> String {
        let query = self.field.stringValue().to_string();
        self.prefix
            .get()
            .map_or(query.clone(), |prefix| format!("{prefix}{query}"))
    }

    /// Splits `text` into the prompt's prefix and the field's query.
    fn set_text(&self, text: &str) {
        let mut chars = text.chars();
        let prefix = chars.next().filter(|&c| Mode::from_prefix(c).is_some());
        let query = if prefix.is_some() {
            chars.as_str()
        } else {
            text
        };
        self.field.setStringValue(&NSString::from_str(query));
        self.set_prefix(prefix);
    }

    /// A prefix typed at the start of the field while there is none moves to the prompt.
    /// True when it did.
    fn absorb(&self) -> bool {
        if self.prefix.get().is_some() {
            return false;
        }
        let text = self.field.stringValue().to_string();
        if text.chars().next().and_then(Mode::from_prefix).is_none() {
            return false;
        }
        self.set_text(&text);
        true
    }

    /// `⌫` in the empty field: the prompt's prefix goes. True when there was one.
    fn erase(&self) -> bool {
        if self.prefix.get().is_none() {
            return false;
        }
        self.set_prefix(None);
        true
    }

    fn set_prefix(&self, prefix: Option<char>) {
        self.prefix.set(prefix);
        let mode = prefix.and_then(Mode::from_prefix).unwrap_or(Mode::GoTo);
        self.glyph
            .setStringValue(&NSString::from_str(&prefix.map_or_default(String::from)));
        self.glyph.sizeToFit();
        self.glyph.setHidden(prefix.is_none());
        if let Some(search) = &self.search {
            search.setHidden(prefix.is_some());
        }
        self.field
            .setPlaceholderString(Some(&NSString::from_str(placeholder(mode))));
        self.strip
            .set_active(prefix.and_then(|p| HINTS.iter().position(|(key, _)| key.starts_with(p))));
    }
}

/// What the empty field invites in each mode.
fn placeholder(mode: Mode) -> &'static str {
    match mode {
        Mode::GoTo => "Go to a tab, bookmark, or page",
        Mode::Commands => "Run a command",
        Mode::Headings => "Go to a heading",
        Mode::Tags => "Find bookmarks by tag",
        Mode::Agent => "Ask about this page",
        Mode::SearchShortcut => "Search with a keyword: !gh quickie",
    }
}

pub struct CommandPalette {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    input: Rc<Input>,
    escape: Retained<NSView>,
    rule: Retained<Plate>,
    delegate: Retained<FieldDelegate>,
    list: ListView,
    empty: Retained<NSTextField>,
    /// The position in the last answer of each row, in list order.
    order: Rc<RefCell<Vec<usize>>>,
    favicon: RefCell<Rc<Favicon>>,
    open: Cell<bool>,
}

impl CommandPalette {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let field = NSTextField::new(mtm);
        field.setFont(Some(&TextRole::PaletteInput.font()));
        field.setTextColor(Some(&color::label()));
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setUsesSingleLineMode(true);
        field.setAccessibilityLabel(Some(&NSString::from_str("Palette query")));
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        field.sizeToFit();
        let glyph = NSTextField::labelWithString(&NSString::from_str(">"), mtm);
        glyph.setFont(Some(&prompt_font()));
        glyph.setTextColor(Some(&color::accent()));
        let search = symbol(
            "magnifyingglass",
            "Go to",
            TextRole::PaletteInput,
            &color::secondary_label(),
            mtm,
        );
        let input = Rc::new(Input {
            prefix: Cell::new(None),
            glyph,
            search,
            field,
            strip: HintStrip::new(&HINTS, &SELECT, mtm),
        });
        input.set_prefix(None);
        let delegate = FieldDelegate::new(mtm);
        // SAFETY: the field holds its delegate weakly; the palette keeps it alive as long as
        // the field.
        unsafe {
            input
                .field
                .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        let escape = keycap("⎋ esc", KeycapStyle::Plain, mtm);
        escape.setAccessibilityLabel(Some(&NSString::from_str("Escape closes")));
        let rule = Plate::new(Some(color::separator), None, 0.0, mtm);
        let empty = NSTextField::labelWithString(&NSString::new(), mtm);
        empty.setFont(Some(&TextRole::RowTitle.font()));
        empty.setTextColor(Some(&color::secondary_label()));
        empty.setAlignment(NSTextAlignment::Center);
        empty.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        let list = ListView::new(mtm);
        let content = surface.content_view();
        for view in [
            &*input.field as &NSView,
            &input.glyph,
            &escape,
            &rule,
            list.view(),
            &empty,
            input.strip.view(),
        ] {
            content.addSubview(view);
        }
        if let Some(search) = &input.search {
            content.addSubview(search);
        }
        let palette = Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            input,
            escape,
            rule,
            delegate,
            list,
            empty,
            order: Rc::new(RefCell::new(Vec::new())),
            favicon: RefCell::new(Rc::new(|_: &Url| None)),
            open: Cell::new(false),
        };
        let input = Rc::downgrade(&palette.input);
        palette
            .delegate
            .on_erase_empty(move || input.upgrade().is_some_and(|input| input.erase()));
        let input = Rc::downgrade(&palette.input);
        palette.delegate.on_absorb(move || {
            if let Some(input) = input.upgrade()
                && input.absorb()
            {
                caret_to_end(&input.field);
            }
        });
        palette
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens over `host` with `text` in the field and the caret after it, ready to type on.
    /// The caller answers with [`Self::set_items`].
    pub fn show(&self, host: &NSView, window: &NSWindow, text: &str) {
        self.input.set_text(text);
        if !self.is_open() {
            if let Some(scrim) = &self.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(self.surface.view());
            motion::show(
                self.surface.view(),
                motion::OVERLAY_ENTER,
                Trigger::Keyboard,
            );
            self.open.set(true);
        }
        self.layout(host);
        window.makeFirstResponder(Some(&self.input.field));
        caret_to_end(&self.input.field);
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
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.open.set(false);
    }

    /// What the user typed, prefix included.
    #[must_use]
    pub fn text(&self) -> String {
        self.input.text()
    }

    /// Replaces the query, the caret after it, as when a tag row narrows it to `#tag `. The
    /// caller answers the new query.
    pub fn set_text(&self, text: &str) {
        self.input.set_text(text);
        caret_to_end(&self.input.field);
    }

    /// Where row pictures come from: the favicon kept for a page's site, if any.
    pub fn set_favicons(&self, favicon: impl Fn(&Url) -> Option<Rc<[u8]>> + 'static) {
        *self.favicon.borrow_mut() = Rc::new(favicon);
    }

    /// Shows the answer to the current query at `now`, in sections, the first row selected.
    /// The answer is `items`, best first, as the palette use case gives it.
    pub fn set_items(&self, items: &[PaletteItem], now: Timestamp) {
        let text = self.text();
        let query = Query::parse(&text);
        let groups = groups(items, query.mode(), query.text(), items.len() >= ANSWERS);
        let favicon = Rc::clone(&self.favicon.borrow());
        let mut order = Vec::with_capacity(items.len());
        let sections = groups
            .into_iter()
            .map(|group| {
                order.extend(&group.members);
                ListSection {
                    title: group.title.to_owned(),
                    counter: group.counter,
                    rows: group
                        .members
                        .iter()
                        .map(|&index| row(&items[index], now, &*favicon))
                        .collect(),
                }
            })
            .collect();
        *self.order.borrow_mut() = order;
        self.list.set_sections(sections);
        let message = if items.is_empty() {
            nothing_found(query.mode(), query.text())
        } else {
            None
        };
        self.empty
            .setStringValue(&NSString::from_str(message.as_deref().unwrap_or_default()));
        self.empty.setHidden(message.is_none());
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let host = unsafe { self.surface.view().superview() };
        if let Some(host) = host {
            self.layout(&host);
        }
    }

    /// Position, in the last [`Self::set_items`], of the selected row.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.list
            .selected()
            .and_then(|row| self.order.borrow().get(row).copied())
    }

    /// Moves the selection down by `delta` rows (up when negative), wrapping around.
    pub fn move_selection(&self, delta: isize) {
        self.list.move_selection(delta);
    }

    /// True while an input method is composing in the field: ⏎ then commits the composition.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.input
            .field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Runs `changed` after each edit in the field.
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        self.delegate.on_change(changed);
    }

    /// Runs `activate` with the position, in the last answer, of a clicked row.
    pub fn on_click(&self, activate: impl Fn(usize) + 'static) {
        let order = Rc::clone(&self.order);
        self.list.on_click(move |row| {
            let item = order.borrow().get(row).copied();
            if let Some(item) = item {
                activate(item);
            }
        });
    }

    /// Sizes the surface to its rows and keeps its top edge at 15 % of the host's height:
    /// the input row and its hairline, the list or the line that says nothing matched, then
    /// the hint strip.
    fn layout(&self, host: &NSView) {
        let bounds = host.bounds();
        let width = overlay::PALETTE_WIDTH.min(bounds.size.width - space::XL * 2.0);
        let padding = space::PALETTE_PADDING;
        let field_height = self.input.field.frame().size.height;
        let input_height = field_height + space::LG * 2.0;
        let strip = HintStrip::HEIGHT;
        let top = bounds.size.height * (1.0 - overlay::PALETTE_TOP);
        // The list scrolls rather than run past the window's bottom edge.
        let room = (top - space::XL - input_height - 1.0 - strip - space::SM * 2.0).max(0.0);
        let shown = self.list.len().min(VISIBLE_ROWS);
        let list_height = self.list.height_within(shown, room);
        let empty_height = self.empty.fittingSize().height.ceil();
        let body = if shown > 0 {
            list_height + space::SM * 2.0
        } else if self.empty.isHidden() {
            0.0
        } else {
            empty_height + space::XL * 2.0
        };
        let height = input_height + 1.0 + body + strip;
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - width) / 2.0).round(),
                (top - height).round(),
            ),
            NSSize::new(width, height),
        ));
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        self.input.strip.set_frame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, strip),
        ));
        self.list.view().setFrame(NSRect::new(
            NSPoint::new(space::SM, strip + space::SM),
            NSSize::new(width - space::SM * 2.0, list_height),
        ));
        self.list.view().setHidden(shown == 0);
        self.empty.setFrame(NSRect::new(
            NSPoint::new(padding, strip + space::XL),
            NSSize::new(width - padding * 2.0, empty_height),
        ));
        self.rule.setFrame(NSRect::new(
            NSPoint::new(0.0, strip + body),
            NSSize::new(width, 1.0),
        ));
        self.rule.setHidden(body == 0.0);

        // The input row, from the left: the prompt, the field; on the right the `esc` cap.
        let row_bottom = strip + body + 1.0;
        let middle = |size: NSSize| (row_bottom + (input_height - size.height) / 2.0).floor();
        let prompt: Option<&NSView> = if self.input.prefix.get().is_some() {
            Some(&self.input.glyph)
        } else {
            self.input
                .search
                .as_deref()
                .map(|search| -> &NSView { search })
        };
        let prompt_width = prompt.map_or(0.0, |view| view.frame().size.width);
        if let Some(view) = prompt {
            view.setFrameOrigin(NSPoint::new(padding, middle(view.frame().size)));
        }
        let escape = self.escape.frame().size;
        self.escape
            .setFrameOrigin(NSPoint::new(width - padding - escape.width, middle(escape)));
        let field_x = padding + prompt_width + space::MD;
        self.input.field.setFrame(NSRect::new(
            NSPoint::new(field_x, row_bottom + space::LG),
            NSSize::new(
                (width - padding - escape.width - space::MD - field_x).max(0.0),
                field_height,
            ),
        ));
    }
}

fn caret_to_end(field: &NSTextField) {
    if let Some(editor) = field.currentEditor() {
        let end = field.stringValue().length();
        editor.setSelectedRange(NSRange::new(end, 0));
    }
}

/// The prompt's prefix: SF Mono at the input's size, semibold.
fn prompt_font() -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold: CGFloat = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::monospacedSystemFontOfSize_weight(TextRole::PaletteInput.point_size(), semibold)
}

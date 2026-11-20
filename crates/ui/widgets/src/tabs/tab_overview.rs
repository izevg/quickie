//! The tab overview: an E2 glass grid of the window's tabs in opening order, each a card with
//! the page's last picture. A pinch in on the page opens it and the cards follow the fingers;
//! the palette command and its key open it at once. Typing filters the cards, the arrows move
//! the selection, a click or `⏎` switches; the browser decides what those do.
//!
//! - [`grid`]: columns, card size, visible rows, arrow moves.
//! - [`pinch`]: magnify deltas to progress, and how a release settles.
//! - [`filter`]: which cards the typed words keep.
//! - [`card`]: one tab's card.

pub mod card;
pub mod filter;
pub mod grid;
pub mod pinch;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSClickGestureRecognizer, NSControlTextDidChangeNotification, NSFocusRingType,
    NSGestureRecognizer, NSImage, NSLineBreakMode, NSTextAlignment, NSTextField, NSTextInputClient,
    NSTextView, NSView, NSWindow,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, hint_bar, motion};

use card::Card;
use grid::{Layout, Move, Room};
use pinch::{Pinch, Step};

/// One tab as the overview shows it.
#[derive(Clone, Debug, Default)]
pub struct OverviewTab {
    pub title: String,
    pub host: String,
    /// The pinned slot, `1` … `9`.
    pub slot: Option<u8>,
    /// The page's last picture; `None` for a tab not loaded or not drawn yet.
    pub thumbnail: Option<Retained<NSImage>>,
}

/// What a click on a card runs, with the card's position in the tabs shown.
type Activate = Rc<dyn Fn(usize)>;

const HINTS: [(&str, &str); 4] = [
    ("←→↑↓", "select"),
    ("⏎", "switch"),
    ("⌘W", "close tab"),
    ("⎋", "close"),
];

pub struct TabOverview {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    field: Retained<NSTextField>,
    count: Retained<NSTextField>,
    hints: Retained<NSView>,
    grid: Retained<NSView>,
    cards: RefCell<Vec<Card>>,
    /// Title and host of each card, for the filter.
    words: RefCell<Vec<(String, String)>>,
    /// The cards the filter keeps, by position in the tabs shown.
    kept: RefCell<Vec<usize>>,
    /// The selection, as a position in `kept`.
    selected: Cell<usize>,
    /// The tab the user last chose, by position in the tabs shown: it is selected again when
    /// a filter that hid it is taken back.
    chosen: Cell<Option<usize>>,
    layout: Cell<Option<(Layout, usize)>>,
    /// Takes keys: shown, or following a pinch that has not settled closed.
    open: Cell<bool>,
    pinch: Cell<Option<Pinch>>,
    activate: RefCell<Option<Activate>>,
    _edits: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    _click: Retained<ClickTarget>,
    mtm: MainThreadMarker,
}

impl TabOverview {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let field = filter_field(mtm);
        let count = NSTextField::labelWithString(&NSString::new(), mtm);
        count.setFont(Some(&TextRole::RowDetail.font()));
        count.setTextColor(Some(&color::secondary_label()));
        count.setAlignment(NSTextAlignment::Right);
        let hints = hint_bar(&HINTS, mtm);
        let grid = NSView::new(mtm);
        for view in [&*field, &*count, &*grid, &*hints] {
            surface.content_view().addSubview(view);
        }
        Rc::new_cyclic(|this: &Weak<Self>| {
            let edits = observe_edits(&field, this.clone());
            let click = ClickTarget::new(this.clone(), mtm);
            // SAFETY: the recognizer calls `clicked:` on its target, which takes the sender;
            // the overview keeps the target alive as long as the grid.
            let recognizer = unsafe {
                NSClickGestureRecognizer::initWithTarget_action(
                    NSClickGestureRecognizer::alloc(mtm),
                    Some(&click),
                    Some(sel!(clicked:)),
                )
            };
            grid.addGestureRecognizer(&recognizer);
            Self {
                scrim: Scrim::new(Elevation::E2, mtm),
                surface,
                field,
                count,
                hints,
                grid,
                cards: RefCell::new(Vec::new()),
                words: RefCell::new(Vec::new()),
                kept: RefCell::new(Vec::new()),
                selected: Cell::new(0),
                chosen: Cell::new(None),
                layout: Cell::new(None),
                open: Cell::new(false),
                pinch: Cell::new(None),
                activate: RefCell::new(None),
                _edits: edits,
                _click: click,
                mtm,
            }
        })
    }

    /// True while the overview takes keys.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Runs `activate` with the position of a clicked card in the tabs last shown.
    pub fn on_activate(&self, activate: impl Fn(usize) + 'static) {
        *self.activate.borrow_mut() = Some(Rc::new(activate));
    }

    /// Opens over `host` on `tabs` with `selected` chosen. During a pinch it appears where the
    /// fingers are; otherwise at once for the keyboard, or with `overlay-enter` for a click.
    pub fn show(
        &self,
        host: &NSView,
        window: &NSWindow,
        tabs: &[OverviewTab],
        selected: usize,
        trigger: Trigger,
    ) {
        self.field.setStringValue(&NSString::new());
        self.set_tabs(tabs, selected);
        // SAFETY: a getter; the superview is retained for the duration of the check.
        let mounted = unsafe { self.surface.view().superview() }.is_some();
        if let Some(scrim) = &self.scrim {
            scrim.view().setFrame(host.bounds());
            scrim.view().setHidden(false);
            if !mounted {
                host.addSubview(scrim.view());
            }
        }
        if !mounted {
            host.addSubview(self.surface.view());
        }
        self.open.set(true);
        self.layout(host);
        window.makeFirstResponder(Some(&self.field));
        match self.pinch.get() {
            Some(pinch) if pinch.end().is_some() => self.follow(pinch_progress(pinch)),
            _ => {
                self.stop_gesture();
                motion::show(self.surface.view(), motion::OVERLAY_ENTER, trigger);
            }
        }
    }

    /// Replaces the cards, as after a tab closed, keeping what is typed.
    pub fn set_tabs(&self, tabs: &[OverviewTab], selected: usize) {
        for card in self.cards.borrow_mut().drain(..) {
            card.view().removeFromSuperview();
        }
        let cards: Vec<Card> = tabs.iter().map(|tab| Card::new(tab, self.mtm)).collect();
        for card in &cards {
            self.grid.addSubview(card.view());
        }
        *self.cards.borrow_mut() = cards;
        *self.words.borrow_mut() = tabs
            .iter()
            .map(|tab| (tab.title.clone(), tab.host.clone()))
            .collect();
        self.refilter(Some(selected));
    }

    /// Takes the overview away at once; or, from a click, with `fade-fast`.
    pub fn hide(&self, trigger: Trigger) {
        self.open.set(false);
        self.pinch.set(None);
        self.stop_gesture();
        if trigger == Trigger::Keyboard {
            self.unmount();
            return;
        }
        motion::hide(self.surface.view(), motion::FADE_FAST, trigger);
        if let Some(scrim) = &self.scrim {
            motion::hide(scrim.view(), motion::FADE_FAST, trigger);
        }
    }

    /// The selected card, by position in the tabs last shown; `None` when the filter keeps
    /// nothing.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.kept.borrow().get(self.selected.get()).copied()
    }

    /// An arrow key.
    pub fn move_selection(&self, by: Move) {
        let Some((layout, _)) = self.layout.get() else {
            return;
        };
        let count = self.kept.borrow().len();
        self.selected
            .set(grid::step(self.selected.get(), count, layout.columns, by));
        self.chosen.set(self.selected());
        self.relayout();
    }

    /// Shows a new picture of the tab at `index`.
    pub fn set_thumbnail(&self, index: usize, image: Option<&NSImage>) {
        if let Some(card) = self.cards.borrow().get(index) {
            card.set_picture(image);
        }
    }

    /// True while an input method is composing in the filter: `⏎` then commits the text.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Fingers came down on the trackpad: a pinch starts from where the overview is now.
    pub fn pinch_began(&self) {
        let from = motion::presented_progress(self.surface.view()).unwrap_or(if self.is_open() {
            1.0
        } else {
            0.0
        });
        self.pinch.set(Some(Pinch::begin(from)));
    }

    /// The fingers moved. While the overview is on screen it follows them; [`Step::Follow`]
    /// with the overview closed asks the caller to [`Self::show`] it.
    pub fn pinch_changed(&self, magnification: f64, time: f64) -> Step {
        let Some(mut pinch) = self.pinch.get() else {
            return Step::Page;
        };
        let step = pinch.change(magnification, time);
        self.pinch.set(Some(pinch));
        if let Step::Follow(progress) = step
            && self.is_open()
        {
            self.follow(progress);
        }
        step
    }

    /// The fingers lifted. Answers whether the overview settles open, `None` when the pinch
    /// never reached it; closed, it stops taking keys now and leaves when the spring rests.
    pub fn pinch_ended(self: &Rc<Self>) -> Option<bool> {
        let settle = self.pinch.take()?.end()?;
        if !self.is_open() {
            return None;
        }
        self.open.set(settle.open);
        let motion = (settle.progress, settle.velocity);
        let this = Rc::downgrade(self);
        motion::settle(self.surface.view(), motion, settle.open, true, move || {
            if let Some(this) = this.upgrade()
                && !this.is_open()
                && this.pinch.get().is_none()
            {
                this.unmount();
            }
        });
        if let Some(scrim) = &self.scrim {
            motion::settle(scrim.view(), motion, settle.open, false, || {});
        }
        Some(settle.open)
    }

    fn follow(&self, progress: f64) {
        motion::follow(self.surface.view(), progress, true);
        if let Some(scrim) = &self.scrim {
            motion::follow(scrim.view(), progress, false);
        }
    }

    fn stop_gesture(&self) {
        motion::stop_gesture(self.surface.view());
        if let Some(scrim) = &self.scrim {
            motion::stop_gesture(scrim.view());
        }
    }

    fn unmount(&self) {
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.stop_gesture();
    }

    /// Keeps the cards the typed words match, the selection on `keep` (a tab position), or
    /// else on the tab last chosen, if it is kept; otherwise on the first card.
    fn refilter(&self, keep: Option<usize>) {
        let query = self.field.stringValue().to_string();
        let kept = {
            let words = self.words.borrow();
            filter::kept(
                words
                    .iter()
                    .map(|(title, host)| (title.as_str(), host.as_str())),
                &query,
            )
        };
        if keep.is_some() {
            self.chosen.set(keep);
        }
        let selected = self
            .chosen
            .get()
            .and_then(|tab| kept.iter().position(|&kept| kept == tab))
            .unwrap_or(0);
        let total = self.words.borrow().len();
        let count = match (kept.len(), total) {
            (shown, total) if shown < total => format!("{shown} of {total} tabs"),
            (_, 1) => "1 tab".to_owned(),
            (_, total) => format!("{total} tabs"),
        };
        self.count.setStringValue(&NSString::from_str(&count));
        *self.kept.borrow_mut() = kept;
        self.selected.set(selected);
        self.relayout();
    }

    fn relayout(&self) {
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let host = unsafe { self.surface.view().superview() };
        if let Some(host) = host {
            self.layout(&host);
        }
    }

    /// Sizes the grid to the kept cards, centers the surface in `host`, and places the rows
    /// around the selection.
    fn layout(&self, host: &NSView) {
        let bounds = host.bounds().size;
        let padding = space::PALETTE_PADDING;
        let header = self
            .field
            .frame()
            .size
            .height
            .max(self.count.frame().size.height);
        let hints = self.hints.frame().size.height;
        let chrome = padding * 2.0 + header + hints + space::MD * 2.0;
        let width = overlay::OVERVIEW_MAX_WIDTH.min(bounds.width - space::XL * 2.0);
        let room = Room {
            size: NSSize::new(
                width - padding * 2.0,
                bounds.height * overlay::OVERVIEW_MAX_HEIGHT - chrome,
            ),
            gap: space::LG,
            card_width: (overlay::THUMBNAIL_MIN_WIDTH, overlay::THUMBNAIL_MAX_WIDTH),
            chrome: card::chrome(),
        };
        let kept = self.kept.borrow();
        let layout = grid::layout(kept.len(), &room);
        let rows = layout.visible_rows(kept.len(), self.selected.get());
        let grid_size = layout.size(kept.len(), room.gap);
        self.layout.set(Some((layout, rows.start)));
        let inner = grid_size
            .width
            .max(overlay::OVERVIEW_MIN_WIDTH - padding * 2.0);
        let size = NSSize::new(inner + padding * 2.0, chrome + grid_size.height);
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.width - size.width) / 2.0).round(),
                ((bounds.height - size.height) / 2.0).round(),
            ),
            size,
        ));
        self.hints.setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(inner, hints),
        ));
        self.grid.setFrame(NSRect::new(
            NSPoint::new(
                padding + ((inner - grid_size.width) / 2.0).floor(),
                padding + hints + space::MD,
            ),
            grid_size,
        ));
        let top = size.height - padding - header;
        let count_width = self.count.frame().size.width.max(space::XL * 4.0);
        self.count.setFrame(NSRect::new(
            NSPoint::new(padding + inner - count_width, top),
            NSSize::new(count_width, header),
        ));
        self.field.setFrame(NSRect::new(
            NSPoint::new(padding, top),
            NSSize::new(inner - count_width - space::MD, header),
        ));
        let cards = self.cards.borrow();
        let selected = kept.get(self.selected.get()).copied();
        for card in cards.iter() {
            card.view().setHidden(true);
        }
        for (position, &index) in kept.iter().enumerate() {
            let Some(card) = cards.get(index) else {
                continue;
            };
            let row = position / layout.columns.max(1);
            if !rows.contains(&row) {
                continue;
            }
            let origin = layout.origin(position, rows.start, grid_size.height, room.gap);
            card.set_frame(NSRect::new(origin, layout.card));
            card.set_selected(Some(index) == selected);
            card.view().setHidden(false);
        }
    }

    /// A click in the grid at `point`, in the grid's coordinates.
    fn clicked(&self, point: NSPoint) {
        let Some((layout, first_row)) = self.layout.get() else {
            return;
        };
        let kept = self.kept.borrow().clone();
        let height = self.grid.frame().size.height;
        let Some(position) = layout.hit(point, kept.len(), first_row, height, space::LG) else {
            return;
        };
        let activate = self.activate.borrow().clone();
        if let (Some(activate), Some(&index)) = (activate, kept.get(position)) {
            activate(index);
        }
    }
}

/// Where a pinch in progress has the overview.
fn pinch_progress(pinch: Pinch) -> f64 {
    pinch.end().map_or(0.0, |settle| settle.progress)
}

fn filter_field(mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::new(mtm);
    field.setFont(Some(&TextRole::RowTitle.font()));
    field.setTextColor(Some(&color::label()));
    field.setPlaceholderString(Some(&NSString::from_str("Type to filter tabs")));
    field.setBezeled(false);
    field.setBordered(false);
    field.setDrawsBackground(false);
    field.setFocusRingType(NSFocusRingType::None);
    field.setUsesSingleLineMode(true);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

/// Refilters on every edit in `field`. The observer lives as long as the returned token.
fn observe_edits(
    field: &NSTextField,
    overview: Weak<TabOverview>,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
        if let Some(overview) = overview.upgrade() {
            overview.refilter(None);
        }
    });
    // SAFETY: an immutable string constant AppKit defines and never changes.
    let name = unsafe { NSControlTextDidChangeNotification };
    // SAFETY: the field posts on the main thread, where the block runs (no queue) and where
    // the overview it captures lives.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            Some(field),
            None,
            &block,
        )
    }
}

pub struct ClickIvars {
    overview: Weak<TabOverview>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ClickTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieTabOverviewClick"]
    #[ivars = ClickIvars]
    pub struct ClickTarget;

    impl ClickTarget {
        #[unsafe(method(clicked:))]
        fn clicked(&self, recognizer: &NSGestureRecognizer) {
            let Some(overview) = self.ivars().overview.upgrade() else {
                return;
            };
            let point = recognizer.locationInView(Some(&overview.grid));
            overview.clicked(point);
        }
    }

    unsafe impl NSObjectProtocol for ClickTarget {}
);

impl ClickTarget {
    fn new(overview: Weak<TabOverview>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClickIvars { overview });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

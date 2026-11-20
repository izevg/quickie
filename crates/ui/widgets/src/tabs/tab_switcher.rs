//! The `⌃⇥` switcher: an E2 glass strip of the cycle's tabs as 16:10 thumbnail cards in
//! most-recently-used order, the selection ringed in the accent. As many cards show as the
//! window fits, at most six; the strip scrolls to keep the selection in view. While `⌃` is
//! held, typed letters filter the cards (a filter row above them names the query and how many
//! tabs it keeps), and a footer names the keys and counts the tabs.
//!
//! It shows only after a hold, and the keyboard drives it, so it appears, moves and disappears
//! instantly. The browser owns the cycle: it asks what the typed words keep and tells the
//! switcher what is selected. Cards exist only for the tabs on screen, from a pool.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBox, NSBoxType, NSFont, NSImage, NSImageView, NSLineBreakMode,
    NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, hint_bar, motion, symbol};

use quickie_ui_shared::tab_card::{self as card, Badge, Card, HALO, Look, TabCard, filter};

const HINTS: [(&str, &str); 3] = [("release ⌃", "to switch"), ("W", "close"), ("⇧", "reverse")];

/// The most cards on screen at once.
const MOST_CARDS: usize = 6;
/// The narrowest and widest a card's picture is drawn.
const PICTURE_WIDTH: (CGFloat, CGFloat) = (160.0, 220.0);
/// Between two cards, besides their halos: pictures stand `space-lg` apart.
const GAP: CGFloat = space::LG - HALO * 2.0;

pub struct TabSwitcher {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    /// The filter row: a magnifier, what is typed, and how many tabs it keeps.
    magnifier: Option<Retained<NSImageView>>,
    query_label: Retained<NSTextField>,
    filtered: Badge,
    /// Says so when the filter keeps nothing.
    empty: Retained<NSTextField>,
    line: Retained<NSBox>,
    hints: Retained<NSView>,
    total: Retained<NSTextField>,
    /// The pool: cards for the tabs on screen, and spare ones hidden.
    cards: RefCell<Vec<Card>>,
    /// The tab each card of the pool shows, by position in `tabs`.
    on_card: RefCell<Vec<Option<usize>>>,
    tabs: RefCell<Vec<TabCard>>,
    query: RefCell<String>,
    /// The tabs the filter keeps, by position in `tabs`.
    kept: RefCell<Vec<usize>>,
    /// The selection, by position in `tabs`.
    selected: Cell<Option<usize>>,
    /// The first kept tab on screen, by position in `kept`.
    first: Cell<usize>,
    open: Cell<bool>,
    mtm: MainThreadMarker,
}

impl TabSwitcher {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let magnifier = symbol(
            "magnifyingglass",
            "Filter",
            TextRole::RowTitle,
            &color::accent(),
            mtm,
        );
        let query_label = text(&query_font(), &color::label(), mtm);
        let filtered = Badge::tinted(mtm);
        let empty = text(&TextRole::RowTitle.font(), &color::secondary_label(), mtm);
        empty.setAlignment(NSTextAlignment::Center);
        let line = NSBox::new(mtm);
        line.setBoxType(NSBoxType::Separator);
        let hints = hint_bar(&HINTS, mtm);
        let total = text(
            &TextRole::Caption.mono_font(),
            &color::secondary_label(),
            mtm,
        );
        total.setAlignment(NSTextAlignment::Right);
        let content = surface.content_view();
        if let Some(magnifier) = &magnifier {
            content.addSubview(magnifier);
        }
        for view in [
            &*query_label,
            filtered.view(),
            &*empty,
            &*line,
            &*hints,
            &*total,
        ] {
            content.addSubview(view);
        }
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            magnifier,
            query_label,
            filtered,
            empty,
            line,
            hints,
            total,
            cards: RefCell::new(Vec::new()),
            on_card: RefCell::new(Vec::new()),
            tabs: RefCell::new(Vec::new()),
            query: RefCell::new(String::new()),
            kept: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            first: Cell::new(0),
            open: Cell::new(false),
            mtm,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Shows `tabs` over `host`, centered, with nothing typed and `selected` ringed.
    pub fn show(&self, host: &NSView, tabs: Vec<TabCard>, selected: Option<usize>) {
        self.query.borrow_mut().clear();
        self.first.set(0);
        self.set_tabs(tabs, selected);
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
        self.render(host);
    }

    /// Replaces the tabs, as after one closed, keeping what is typed. Answers the positions
    /// the filter keeps, for the cycle to visit.
    pub fn set_tabs(&self, tabs: Vec<TabCard>, selected: Option<usize>) -> Vec<usize> {
        *self.tabs.borrow_mut() = tabs;
        self.on_card.borrow_mut().fill(None);
        self.selected.set(selected);
        self.refilter()
    }

    /// Rings `selected`, scrolling the strip when it is out of view.
    pub fn select(&self, selected: Option<usize>) {
        self.selected.set(selected);
        self.rerender();
    }

    /// Adds `typed` to the filter. Answers the positions it keeps, for the cycle to visit; the
    /// browser then says what is selected.
    pub fn type_text(&self, typed: &str) -> Vec<usize> {
        self.query.borrow_mut().push_str(typed);
        self.refilter()
    }

    /// Takes the last character back from the filter; answers as [`Self::type_text`].
    pub fn erase(&self) -> Vec<usize> {
        self.query.borrow_mut().pop();
        self.refilter()
    }

    /// What is typed.
    #[must_use]
    pub fn query(&self) -> String {
        self.query.borrow().clone()
    }

    /// Shows a new picture of the tab at `index`.
    pub fn set_thumbnail(&self, index: usize, image: Option<&NSImage>) {
        if let Some(tab) = self.tabs.borrow_mut().get_mut(index) {
            tab.thumbnail = image.map(Message::retain);
        }
        let on_card = self.on_card.borrow();
        if let Some(slot) = on_card.iter().position(|&shown| shown == Some(index))
            && let Some(card) = self.cards.borrow().get(slot)
        {
            card.set_picture(image);
        }
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

    fn refilter(&self) -> Vec<usize> {
        let kept = {
            let tabs = self.tabs.borrow();
            filter::kept(
                tabs.iter()
                    .map(|tab| (tab.title.as_str(), tab.host.as_str())),
                &self.query.borrow(),
            )
        };
        self.kept.borrow_mut().clone_from(&kept);
        self.rerender();
        kept
    }

    fn rerender(&self) {
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let host = unsafe { self.surface.view().superview() };
        if let Some(host) = host.filter(|_| self.is_open()) {
            self.render(&host);
        }
    }

    fn render(&self, host: &NSView) {
        let padding = space::PALETTE_PADDING;
        let room = host.bounds().size.width - space::XL * 2.0 - padding * 2.0;
        let (capacity, picture) = fit(room);
        let card = card::strip_size(picture);
        let kept = self.kept.borrow();
        let at = self
            .selected
            .get()
            .and_then(|selected| kept.iter().position(|&tab| tab == selected));
        let first = scroll(self.first.get(), kept.len(), at, capacity);
        self.first.set(first);
        let shown = kept.len().saturating_sub(first).min(capacity);
        let row = row_width(shown.max(1), card.width);
        let filtering = !self.query.borrow().is_empty();
        let footer = self.lay_out_text(filtering, kept.len());
        // As wide as all the tabs need, so the strip keeps its width while typing narrows it.
        let total = self.tabs.borrow().len();
        let inner = row_width(total.min(capacity).max(1), card.width)
            .max(footer.width)
            .ceil();
        let row_left = padding + ((inner - row) / 2.0).floor();
        let mut y = padding;
        self.hints.setFrame(NSRect::new(
            NSPoint::new(padding, y),
            NSSize::new(self.hints.frame().size.width, footer.height),
        ));
        let total = self.total.frame().size;
        self.total.setFrame(NSRect::new(
            NSPoint::new(padding + inner - total.width, y),
            NSSize::new(total.width, footer.height),
        ));
        y += footer.height + space::MD;
        self.line.setFrame(NSRect::new(
            NSPoint::new(padding, y),
            NSSize::new(inner, 1.0),
        ));
        y += 1.0 + space::MD;
        self.place_cards(&kept, first, shown, NSPoint::new(row_left, y), card);
        self.empty.setHidden(shown > 0);
        let empty = self.empty.frame().size;
        self.empty.setFrame(NSRect::new(
            NSPoint::new(padding, y + ((card.height - empty.height) / 2.0).floor()),
            NSSize::new(inner, empty.height),
        ));
        y += card.height;
        if filtering {
            y += space::SM;
            y += self.place_filter_row(padding + HALO, y);
        }
        y += padding;
        self.place(host, NSSize::new(inner + padding * 2.0, y));
    }

    /// Sets the footer's and the filter row's words; answers the footer's height and the width
    /// it needs.
    fn lay_out_text(&self, filtering: bool, kept: usize) -> NSSize {
        let count = self.tabs.borrow().len();
        let noun = if count == 1 { "tab" } else { "tabs" };
        self.total
            .setStringValue(&NSString::from_str(&format!("{count} {noun} · MRU order")));
        self.total.sizeToFit();
        let query = self.query.borrow();
        self.query_label.setStringValue(&NSString::from_str(&query));
        self.query_label.sizeToFit();
        self.filtered.set_text(
            if filtering {
                format!("{kept} filtered")
            } else {
                String::new()
            }
            .as_str(),
        );
        self.query_label.setHidden(!filtering);
        if let Some(magnifier) = &self.magnifier {
            magnifier.setHidden(!filtering);
        }
        self.empty
            .setStringValue(&NSString::from_str(&format!("No tab matches “{query}”")));
        self.empty.sizeToFit();
        let hints = self.hints.frame().size;
        let total = self.total.frame().size;
        NSSize::new(
            hints.width + space::XL + total.width,
            hints.height.max(total.height).ceil(),
        )
    }

    /// The filter row at `(x, y)`; answers its height.
    fn place_filter_row(&self, x: CGFloat, y: CGFloat) -> CGFloat {
        let query = self.query_label.frame().size;
        let badge = self.filtered.view().frame().size;
        let icon = self
            .magnifier
            .as_ref()
            .map_or(NSSize::new(0.0, 0.0), |icon| icon.frame().size);
        let height = query.height.max(badge.height).max(icon.height).ceil();
        let middle = |item: CGFloat| y + ((height - item) / 2.0).floor();
        let mut left = x;
        if let Some(magnifier) = &self.magnifier {
            magnifier.setFrameOrigin(NSPoint::new(left, middle(icon.height)));
            left += icon.width + space::SM;
        }
        self.query_label
            .setFrameOrigin(NSPoint::new(left, middle(query.height)));
        left += query.width + space::SM;
        self.filtered
            .view()
            .setFrameOrigin(NSPoint::new(left, middle(badge.height)));
        height
    }

    /// Fills the pool's cards with the `shown` kept tabs from `first`, left to right from
    /// `origin`.
    fn place_cards(
        &self,
        kept: &[usize],
        first: usize,
        shown: usize,
        origin: NSPoint,
        size: NSSize,
    ) {
        self.fit_pool(shown);
        let cards = self.cards.borrow();
        let tabs = self.tabs.borrow();
        let mut on_card = self.on_card.borrow_mut();
        let selected = self.selected.get();
        for (slot, card) in cards.iter().enumerate() {
            let Some(&index) = kept.get(first + slot).filter(|_| slot < shown) else {
                card.view().setHidden(true);
                on_card[slot] = None;
                continue;
            };
            if on_card[slot] != Some(index)
                && let Some(tab) = tabs.get(index)
            {
                card.show(tab);
                on_card[slot] = Some(index);
            }
            card.set_selected(selected == Some(index));
            #[expect(clippy::cast_precision_loss, reason = "at most six cards")]
            let x = origin.x + slot as CGFloat * (size.width + GAP);
            card.set_frame(NSRect::new(NSPoint::new(x, origin.y), size));
            card.view().setHidden(false);
        }
    }

    /// Grows the pool to at least `count` cards; spare ones stay for the next time.
    fn fit_pool(&self, count: usize) {
        let mut cards = self.cards.borrow_mut();
        while cards.len() < count {
            let card = Card::new(Look::Strip, self.mtm);
            card.view().setHidden(true);
            self.surface.content_view().addSubview(card.view());
            cards.push(card);
            self.on_card.borrow_mut().push(None);
        }
    }

    /// Sizes the strip to `size` and centers it in `host`.
    fn place(&self, host: &NSView, size: NSSize) {
        let bounds = host.bounds();
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - size.width) / 2.0).round(),
                ((bounds.size.height - size.height) / 2.0).round(),
            ),
            size,
        ));
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
    }
}

/// How many cards fit across `room`, at most [`MOST_CARDS`] and at least one, and how wide
/// their pictures are then: as wide as they share the room, up to the widest.
fn fit(room: CGFloat) -> (usize, CGFloat) {
    let narrowest = PICTURE_WIDTH.0 + HALO * 2.0;
    let fits = ((room + GAP) / (narrowest + GAP)).floor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "floored and clamped to a handful of cards"
    )]
    let count = fits.clamp(1.0, to_float(MOST_CARDS)) as usize;
    let share = (room - GAP * (to_float(count) - 1.0)) / to_float(count) - HALO * 2.0;
    (count, share.clamp(PICTURE_WIDTH.0, PICTURE_WIDTH.1).floor())
}

/// The width of a row of `count` cards `card` wide.
fn row_width(count: usize, card: CGFloat) -> CGFloat {
    let count = to_float(count);
    count * card + (count - 1.0) * GAP
}

/// The first of `len` kept tabs on screen when `capacity` show: it stays at `first` while the
/// selection `at` is in view, and moves just enough to bring it in otherwise.
fn scroll(first: usize, len: usize, at: Option<usize>, capacity: usize) -> usize {
    let capacity = capacity.max(1);
    let last_first = len.saturating_sub(capacity);
    let first = match at {
        Some(at) if at < first => at,
        Some(at) if at >= first + capacity => at + 1 - capacity,
        _ => first,
    };
    first.min(last_first)
}

#[expect(clippy::cast_precision_loss, reason = "a handful of cards")]
fn to_float(count: usize) -> CGFloat {
    count as CGFloat
}

fn query_font() -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::monospacedSystemFontOfSize_weight(TextRole::RowTitle.point_size(), semibold)
}

fn text(
    font: &NSFont,
    tint: &objc2_app_kit::NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::new(), mtm);
    label.setFont(Some(font));
    label.setTextColor(Some(tint));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

#[cfg(test)]
mod tests {
    use super::{GAP, MOST_CARDS, PICTURE_WIDTH, fit, row_width, scroll};
    use quickie_ui_shared::tab_card::HALO;

    #[test]
    fn as_many_cards_fit_as_the_room_takes_up_to_six() {
        let card = PICTURE_WIDTH.0 + HALO * 2.0;
        assert_eq!(fit(card).0, 1);
        assert_eq!(fit(card * 2.0 + GAP - 1.0).0, 1);
        assert_eq!(fit(card * 2.0 + GAP).0, 2);
        assert_eq!(fit(10_000.0).0, MOST_CARDS);
        assert_eq!(fit(10.0).0, 1);
    }

    #[test]
    fn pictures_share_the_room_between_the_narrowest_and_the_widest() {
        assert!((fit(936.0).1 - 172.0).abs() < f64::EPSILON);
        assert!((fit(10_000.0).1 - PICTURE_WIDTH.1).abs() < f64::EPSILON);
        assert!((fit(10.0).1 - PICTURE_WIDTH.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_row_is_its_cards_and_the_gaps_between_them() {
        assert!((row_width(1, 100.0) - 100.0).abs() < f64::EPSILON);
        assert!((row_width(3, 100.0) - (300.0 + GAP * 2.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn the_strip_stays_put_while_the_selection_is_in_view() {
        assert_eq!(scroll(0, 10, Some(1), 4), 0);
        assert_eq!(scroll(3, 10, Some(6), 4), 3);
        assert_eq!(scroll(3, 10, Some(3), 4), 3);
    }

    #[test]
    fn the_strip_moves_just_enough_to_show_the_selection() {
        assert_eq!(scroll(0, 10, Some(4), 4), 1);
        assert_eq!(scroll(0, 10, Some(9), 4), 6);
        assert_eq!(scroll(5, 10, Some(2), 4), 2);
        assert_eq!(scroll(5, 10, Some(0), 4), 0);
    }

    #[test]
    fn the_strip_never_scrolls_past_its_last_cards() {
        assert_eq!(scroll(8, 10, None, 4), 6);
        assert_eq!(scroll(3, 3, Some(1), 4), 0);
        assert_eq!(scroll(2, 0, None, 0), 0);
    }
}

//! The list: an `NSTableView` in a scroll view, fed by a data source that diffs each new set of
//! rows against the one on screen, so a query that narrows its answer removes rows instead of
//! reloading the table. Rows may come in sections, each under a header.
//!
//! The table never takes the keyboard or draws its own selection: the overlay's field keeps
//! focus, moves the selection with [`ListView::move_selection`], and the row draws the accent
//! fill itself. A click activates a row. Positions the list reports (the selection, a click)
//! count rows only, never headers.

mod cell;
mod diff;
mod section;

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSBorderType, NSColor, NSControlTextEditingDelegate, NSScrollView,
    NSScrollerStyle, NSTableColumn, NSTableRowView, NSTableView, NSTableViewAnimationOptions,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate,
    NSTableViewSelectionHighlightStyle, NSTableViewStyle, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSIndexSet, NSInteger, NSMutableIndexSet, NSRect, NSSize, NSString};

use cell::Metrics;
pub use diff::{Patch, diff};
use section::Entry;
pub use section::ListSection;

/// How a row's symbol is tinted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconTint {
    /// Most rows.
    #[default]
    Secondary,
    /// What the agent does.
    Agent,
    /// What stands out: a search.
    Accent,
}

/// What a row shows before its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowIcon {
    /// An SF Symbol, by name (`arrow.clockwise`).
    Symbol { name: String, tint: IconTint },
    /// An image the system decodes: a page's favicon.
    Picture(Rc<[u8]>),
}

impl RowIcon {
    /// The symbol `name` in the secondary tint.
    #[must_use]
    pub fn symbol(name: &str) -> Self {
        Self::Symbol {
            name: name.to_owned(),
            tint: IconTint::Secondary,
        }
    }
}

/// One row: what it shows, and the key that says it is the same row as before. A row without
/// `detail` is one line; with it, two.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListRow {
    /// Equal keys are the same row across updates.
    pub key: u64,
    /// The main line.
    pub title: String,
    /// Character positions of `title` to highlight, ascending.
    pub title_matches: Vec<u32>,
    /// The second line, in SF Mono, middle-truncated: a page's address.
    pub detail: String,
    /// Character positions of `detail` to highlight, ascending: an address the user typed into.
    pub detail_matches: Vec<u32>,
    /// Characters of `detail` set in semibold and the primary colour: a URL's host.
    pub detail_strong: Range<u32>,
    /// A chord in glyphs (`⇧⌘P`), drawn as keycaps on the right.
    pub keycaps: Option<String>,
    /// Before the title: a symbol on a one-line row, the tile's picture on a two-line one.
    pub icon: Option<RowIcon>,
    /// A category after the title, in uppercase SF Mono (`NAVIGATION`).
    pub tag: Option<String>,
    /// A note after the title, when there is no tag: `5 min ago`.
    pub note: Option<String>,
    /// A note on the right, before the keycaps: `Visited 6 times`, `Switch to tab`.
    pub trailing: Option<String>,
    /// A filled accent badge on the right: the slot a tab is pinned to, `⌘3`.
    pub badge: Option<String>,
}

/// The list view. Add [`ListView::view`] to an overlay and size it with
/// [`ListView::height_for`]. It does not autoresize: a glass surface resizes its content in a
/// later layout pass, which would shift a frame the overlay has already set.
pub struct ListView {
    scroll: Retained<NSScrollView>,
    table: Retained<NSTableView>,
    source: Retained<ListSource>,
}

impl ListView {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let metrics = Metrics::measure(mtm);
        let source = ListSource::new(metrics, mtm);
        let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), NSRect::default());
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            &NSString::from_str("row"),
        );
        table.addTableColumn(&column);
        table.setHeaderView(None);
        table.setStyle(NSTableViewStyle::Plain);
        table.setSelectionHighlightStyle(NSTableViewSelectionHighlightStyle::None);
        table.setBackgroundColor(&NSColor::clearColor());
        table.setIntercellSpacing(NSSize::new(0.0, 0.0));
        table.setRowHeight(metrics.two_lines());
        table.setColumnAutoresizingStyle(
            NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
        );
        table.setRefusesFirstResponder(true);
        // SAFETY: the table holds its data source and delegate weakly; `ListView` keeps the
        // source alive for as long as the table.
        unsafe { table.setDataSource(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above.
        unsafe { table.setDelegate(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: the target outlives the table (see above), and `rowClicked:` takes a sender.
        unsafe { table.setTarget(Some(&source)) };
        // SAFETY: `ListSource` implements `rowClicked:`.
        unsafe { table.setAction(Some(sel!(rowClicked:))) };

        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&table));
        scroll.setDrawsBackground(false);
        scroll.contentView().setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        Self {
            scroll,
            table,
            source,
        }
    }

    /// The scroll view to add to an overlay.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.scroll
    }

    /// The height of a two-line row.
    #[must_use]
    pub fn row_height(&self) -> CGFloat {
        self.source.ivars().metrics.two_lines()
    }

    /// The height that shows the first `rows` rows whole, with the headers among them.
    #[must_use]
    pub fn height_for(&self, rows: usize) -> CGFloat {
        self.height_within(rows, CGFloat::INFINITY)
    }

    /// As [`Self::height_for`], but no taller than `room`: only the entries that fit whole.
    #[must_use]
    pub fn height_within(&self, rows: usize, room: CGFloat) -> CGFloat {
        let ivars = self.source.ivars();
        let entries = ivars.entries.borrow();
        let heights = entries
            .iter()
            .map(|entry| (ivars.height_of(entry), entry.row().is_some()));
        fit(heights, rows, room)
    }

    /// How many rows there are, headers not counted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source.ivars().rows.borrow().len()
    }

    /// True when there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Shows `rows` without headers in place of the current ones; see [`Self::set_sections`].
    pub fn set_rows(&self, rows: Vec<ListRow>) {
        self.set_sections(vec![ListSection {
            rows,
            ..ListSection::default()
        }]);
    }

    /// Shows `sections` in place of the current rows and selects the first row. Only what
    /// changed is touched: entries that went are removed, new ones inserted, kept ones with new
    /// content redrawn.
    pub fn set_sections(&self, sections: Vec<ListSection>) {
        let ivars = self.source.ivars();
        let selected_key = self
            .selected_entry()
            .and_then(|index| ivars.entries.borrow().get(index).map(Entry::key));
        let fresh = section::entries(sections);
        let patch = {
            let mut current = ivars.entries.borrow_mut();
            let patch = diff(&current, &fresh, Entry::key);
            *current = fresh;
            patch
        };
        let rows: Vec<usize> = ivars
            .entries
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.row().map(|_| index))
            .collect();
        *ivars.rows.borrow_mut() = rows;
        let first = (!self.is_empty()).then_some(0);
        ivars.selected.set(first);
        self.table.beginUpdates();
        self.table.removeRowsAtIndexes_withAnimation(
            &index_set(&patch.removed),
            NSTableViewAnimationOptions::EffectNone,
        );
        self.table.insertRowsAtIndexes_withAnimation(
            &index_set(&patch.inserted),
            NSTableViewAnimationOptions::EffectNone,
        );
        self.table.endUpdates();
        let mut redraw = patch.changed;
        redraw.extend(self.selected_entry());
        redraw.extend(selected_key.and_then(|key| {
            ivars
                .entries
                .borrow()
                .iter()
                .position(|entry| entry.key() == key)
        }));
        self.redraw(&redraw);
        if self.table.numberOfRows() > 0 {
            self.table.scrollRowToVisible(0);
        }
    }

    /// The selected row.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.source.ivars().selected.get()
    }

    /// Moves the selection `delta` rows down (up when negative), wrapping around the ends.
    pub fn move_selection(&self, delta: isize) {
        let len = self.len();
        let Some(current) = self.selected() else {
            return;
        };
        let before = self.selected_entry();
        let next = wrap(current, delta, len);
        self.source.ivars().selected.set(Some(next));
        let after = self.selected_entry();
        self.redraw(&before.into_iter().chain(after).collect::<Vec<_>>());
        if let Some(entry) = after {
            // The header of a section's first row comes into view with it.
            let header = self.source.ivars().header_before(entry);
            for index in header.into_iter().chain([entry]) {
                self.table
                    .scrollRowToVisible(NSInteger::try_from(index).unwrap_or_default());
            }
        }
    }

    /// Runs `activate` with the row a click lands on.
    pub fn on_click(&self, activate: impl Fn(usize) + 'static) {
        *self.source.ivars().on_click.borrow_mut() = Some(Rc::new(activate));
    }

    /// The table position of the selected row.
    fn selected_entry(&self) -> Option<usize> {
        let ivars = self.source.ivars();
        ivars
            .selected
            .get()
            .and_then(|row| ivars.rows.borrow().get(row).copied())
    }

    fn redraw(&self, entries: &[usize]) {
        let len = self.source.ivars().entries.borrow().len();
        let entries: Vec<usize> = entries.iter().copied().filter(|&e| e < len).collect();
        if entries.is_empty() {
            return;
        }
        let set = index_set(&entries);
        self.table.noteHeightOfRowsWithIndexesChanged(&set);
        self.table
            .reloadDataForRowIndexes_columnIndexes(&set, &NSIndexSet::indexSetWithIndex(0));
    }
}

/// The height of the leading entries, each `(height, is a row)`, up to the `rows`-th row and
/// no taller than `room`; a header that would end the run is left out with nothing under it.
fn fit(entries: impl Iterator<Item = (CGFloat, bool)>, rows: usize, room: CGFloat) -> CGFloat {
    let mut height = 0.0;
    let mut kept = 0.0;
    let mut shown = 0;
    for (entry, is_row) in entries {
        if shown == rows || height + entry > room {
            break;
        }
        height += entry;
        if is_row {
            shown += 1;
            kept = height;
        }
    }
    kept
}

/// `current` moved by `delta` in a list of `len`, wrapping around both ends.
fn wrap(current: usize, delta: isize, len: usize) -> usize {
    let (Ok(len_i), Ok(current_i)) = (isize::try_from(len), isize::try_from(current)) else {
        return current;
    };
    if len_i == 0 {
        return current;
    }
    usize::try_from((current_i + delta).rem_euclid(len_i)).unwrap_or(current)
}

fn index_set(indexes: &[usize]) -> Retained<NSIndexSet> {
    let set = NSMutableIndexSet::new();
    for &index in indexes {
        set.addIndex(index);
    }
    Retained::into_super(set)
}

/// What a click on a row runs, with the row's position.
type Activate = Rc<dyn Fn(usize)>;

struct Ivars {
    /// Headers and rows, in table order.
    entries: RefCell<Vec<Entry>>,
    /// The table position of each row, in order.
    rows: RefCell<Vec<usize>>,
    /// The selected row, counted among rows only.
    selected: Cell<Option<usize>>,
    metrics: Metrics,
    on_click: RefCell<Option<Activate>>,
}

impl Ivars {
    fn height_of(&self, entry: &Entry) -> CGFloat {
        match entry {
            Entry::Header { .. } => section::header_height(self.metrics.caption),
            Entry::Row(row) => self.metrics.height_of(row),
        }
    }

    /// The header right above table position `entry`, if there is one.
    fn header_before(&self, entry: usize) -> Option<usize> {
        let before = entry.checked_sub(1)?;
        matches!(
            self.entries.borrow().get(before),
            Some(Entry::Header { .. })
        )
        .then_some(before)
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ListSource` implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieListSource"]
    #[ivars = Ivars]
    struct ListSource;

    unsafe impl NSObjectProtocol for ListSource {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSTableViewDataSource for ListSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            NSInteger::try_from(self.ivars().entries.borrow().len()).unwrap_or(NSInteger::MAX)
        }
    }

    unsafe impl NSControlTextEditingDelegate for ListSource {}

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl NSTableViewDelegate for ListSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            table: &NSTableView,
            column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            self.cell(table, column, row)
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view(&self, _table: &NSTableView, row: NSInteger) -> Option<Retained<NSTableRowView>> {
            self.row_view_for(row)
        }

        #[unsafe(method(tableView:heightOfRow:))]
        fn height_of_row(&self, _table: &NSTableView, row: NSInteger) -> CGFloat {
            let ivars = self.ivars();
            usize::try_from(row)
                .ok()
                .and_then(|index| {
                    ivars
                        .entries
                        .borrow()
                        .get(index)
                        .map(|entry| ivars.height_of(entry))
                })
                .unwrap_or(ivars.metrics.two_lines())
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, _table: &NSTableView, _row: NSInteger) -> bool {
            false
        }
    }

    impl ListSource {
        #[unsafe(method(rowClicked:))]
        fn row_clicked(&self, table: &NSTableView) {
            let Ok(entry) = usize::try_from(table.clickedRow()) else {
                return;
            };
            let Some(row) = self.ivars().rows.borrow().iter().position(|&e| e == entry) else {
                return;
            };
            let activate = self.ivars().on_click.borrow().clone();
            if let Some(activate) = activate {
                activate(row);
            }
        }
    }
);

impl ListSource {
    fn new(metrics: Metrics, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            entries: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            metrics,
            on_click: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// A plain row view that tells `VoiceOver` what its entry shows.
    fn row_view_for(&self, row: NSInteger) -> Option<Retained<NSTableRowView>> {
        let index = usize::try_from(row).ok()?;
        let label = match self.ivars().entries.borrow().get(index)? {
            Entry::Header { title, counter, .. } => section::spoken(title, counter),
            Entry::Row(row) => cell::spoken(row),
        };
        let view = NSTableRowView::new(self.mtm());
        view.setAccessibilityLabel(Some(&NSString::from_str(&label)));
        Some(view)
    }

    fn cell(
        &self,
        table: &NSTableView,
        column: Option<&NSTableColumn>,
        row: NSInteger,
    ) -> Option<Retained<NSView>> {
        let index = usize::try_from(row).ok()?;
        let ivars = self.ivars();
        let entries = ivars.entries.borrow();
        let entry = entries.get(index)?;
        let width = column.map_or_else(|| table.bounds().size.width, NSTableColumn::width);
        let height = ivars.height_of(entry);
        Some(match entry {
            Entry::Header {
                title,
                counter,
                first,
            } => section::header(title, counter, *first, width, height, self.mtm()),
            Entry::Row(item) => {
                let selected = ivars
                    .selected
                    .get()
                    .and_then(|row| ivars.rows.borrow().get(row).copied())
                    == Some(index);
                cell::cell(item, selected, width, ivars.metrics, self.mtm())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{fit, wrap};

    const HEADER: (f64, bool) = (28.0, false);
    const ROW: (f64, bool) = (40.0, true);

    #[test]
    fn the_height_counts_rows_and_the_headers_among_them() {
        let entries = [HEADER, ROW, ROW, HEADER, ROW];

        assert!((fit(entries.into_iter(), 2, f64::INFINITY) - 108.0).abs() < f64::EPSILON);
        assert!((fit(entries.into_iter(), 3, f64::INFINITY) - 176.0).abs() < f64::EPSILON);
        assert!((fit(entries.into_iter(), 0, f64::INFINITY)).abs() < f64::EPSILON);
    }

    #[test]
    fn only_whole_entries_fit_and_a_header_never_ends_the_list() {
        let entries = [HEADER, ROW, ROW, HEADER, ROW];

        assert!((fit(entries.into_iter(), 8, 147.0) - 108.0).abs() < f64::EPSILON);
        assert!((fit(entries.into_iter(), 8, 30.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn the_selection_wraps_around_both_ends() {
        assert_eq!(wrap(0, 1, 3), 1);
        assert_eq!(wrap(2, 1, 3), 0);
        assert_eq!(wrap(0, -1, 3), 2);
        assert_eq!(wrap(1, -1, 3), 0);
    }

    #[test]
    fn an_empty_list_keeps_what_it_has() {
        assert_eq!(wrap(0, 1, 0), 0);
    }
}

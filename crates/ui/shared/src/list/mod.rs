//! The list: an `NSTableView` in a scroll view, fed by a data source that diffs each new set of
//! rows against the one on screen, so a query that narrows its answer removes rows instead of
//! reloading the table.
//!
//! The table never takes the keyboard or draws its own selection: the overlay's field keeps
//! focus, moves the selection with [`ListView::move_selection`], and the row draws the accent
//! fill itself. A click activates a row.

mod cell;
mod diff;

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBorderType, NSColor, NSControlTextEditingDelegate, NSScrollView, NSScrollerStyle,
    NSTableColumn, NSTableView, NSTableViewAnimationOptions, NSTableViewColumnAutoresizingStyle,
    NSTableViewDataSource, NSTableViewDelegate, NSTableViewSelectionHighlightStyle,
    NSTableViewStyle, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSIndexSet, NSInteger, NSMutableIndexSet, NSRect, NSSize, NSString};

use cell::Metrics;
pub use diff::{Patch, diff};

/// One row: what it shows, and the key that says it is the same row as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListRow {
    /// Equal keys are the same row across updates.
    pub key: u64,
    /// The main line.
    pub title: String,
    /// Character positions of `title` to highlight, ascending.
    pub title_matches: Vec<u32>,
    /// The second line, in SF Mono.
    pub detail: String,
    /// Character positions of `detail` to highlight, ascending.
    pub detail_matches: Vec<u32>,
    /// Characters of `detail` set in semibold and the primary colour: a URL's host.
    pub detail_strong: Range<u32>,
    /// A chord in glyphs (`⇧⌘P`), drawn as keycaps on the right.
    pub keycaps: Option<String>,
    /// A word or two on the right, in the caption style: what the row is (`Switch to tab`).
    pub badge: Option<String>,
}

/// The list view. Add [`ListView::view`] to an overlay and size it with
/// [`ListView::row_height`]. It does not autoresize: a glass surface resizes its content in a
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
        table.setRowHeight(metrics.row_height());
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

    /// The height of every row.
    #[must_use]
    pub fn row_height(&self) -> CGFloat {
        self.source.ivars().metrics.row_height()
    }

    /// How many rows there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source.ivars().rows.borrow().len()
    }

    /// True when there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Shows `rows` in place of the current ones and selects the first. Only what changed is
    /// touched: rows that went are removed, new ones inserted, kept ones with new content
    /// redrawn.
    pub fn set_rows(&self, rows: Vec<ListRow>) {
        let ivars = self.source.ivars();
        let selected_key = ivars
            .selected
            .get()
            .and_then(|index| ivars.rows.borrow().get(index).map(|row| row.key));
        let patch = {
            let mut current = ivars.rows.borrow_mut();
            let patch = diff(&current, &rows, |row| row.key);
            *current = rows;
            patch
        };
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
        redraw.extend(first);
        redraw.extend(
            selected_key.and_then(|key| ivars.rows.borrow().iter().position(|row| row.key == key)),
        );
        self.redraw(&redraw);
        if first.is_some() {
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
        let next = wrap(current, delta, len);
        self.source.ivars().selected.set(Some(next));
        self.redraw(&[current, next]);
        self.table
            .scrollRowToVisible(NSInteger::try_from(next).unwrap_or_default());
    }

    /// Runs `activate` with the row a click lands on.
    pub fn on_click(&self, activate: impl Fn(usize) + 'static) {
        *self.source.ivars().on_click.borrow_mut() = Some(Rc::new(activate));
    }

    fn redraw(&self, rows: &[usize]) {
        let len = self.len();
        let rows: Vec<usize> = rows.iter().copied().filter(|&row| row < len).collect();
        if rows.is_empty() {
            return;
        }
        self.table.reloadDataForRowIndexes_columnIndexes(
            &index_set(&rows),
            &NSIndexSet::indexSetWithIndex(0),
        );
    }
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
    rows: RefCell<Vec<ListRow>>,
    selected: Cell<Option<usize>>,
    metrics: Metrics,
    on_click: RefCell<Option<Activate>>,
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
            NSInteger::try_from(self.ivars().rows.borrow().len()).unwrap_or(NSInteger::MAX)
        }
    }

    unsafe impl NSControlTextEditingDelegate for ListSource {}

    // SAFETY: both methods match the protocol's signatures.
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

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, _table: &NSTableView, _row: NSInteger) -> bool {
            false
        }
    }

    impl ListSource {
        #[unsafe(method(rowClicked:))]
        fn row_clicked(&self, table: &NSTableView) {
            let Ok(row) = usize::try_from(table.clickedRow()) else {
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
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            metrics,
            on_click: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn cell(
        &self,
        table: &NSTableView,
        column: Option<&NSTableColumn>,
        row: NSInteger,
    ) -> Option<Retained<NSView>> {
        let index = usize::try_from(row).ok()?;
        let rows = self.ivars().rows.borrow();
        let item = rows.get(index)?;
        let width = column.map_or_else(|| table.bounds().size.width, NSTableColumn::width);
        let selected = self.ivars().selected.get() == Some(index);
        Some(cell::cell(
            item,
            selected,
            width,
            self.ivars().metrics,
            self.mtm(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::wrap;

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

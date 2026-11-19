//! The overview's grid as numbers: how many columns, how big a card, which rows show, and
//! where the arrow keys move the selection. Pure, so it is tested without `AppKit`.

use std::ops::Range;

use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSSize};

/// What the grid is fitted into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// The space for the cards.
    pub size: NSSize,
    /// Between cards, both ways.
    pub gap: CGFloat,
    /// The narrowest and widest a card's thumbnail may be.
    pub card_width: (CGFloat, CGFloat),
    /// A card's height besides its 16:10 thumbnail: title and host.
    pub chrome: CGFloat,
}

/// The grid for some number of cards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub columns: usize,
    /// Rows that fit on screen; more scroll.
    pub rows: usize,
    pub card: NSSize,
}

/// An arrow key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
}

impl Room {
    fn card(&self, width: CGFloat) -> NSSize {
        NSSize::new(width, (width * 10.0 / 16.0 + self.chrome).round())
    }

    /// The widest card width `columns` cards fit at.
    fn width_for(&self, columns: usize) -> CGFloat {
        let columns = to_float(columns);
        let width = (self.size.width - self.gap * (columns - 1.0)) / columns;
        width.min(self.card_width.1).floor()
    }

    /// How many rows of `card` fit in the height.
    fn rows_for(&self, card: NSSize) -> usize {
        let rows = ((self.size.height + self.gap) / (card.height + self.gap)).floor();
        to_count(rows).max(1)
    }
}

/// The biggest cards that show all `count` of them at once; when even the narrowest cards do
/// not fit, as many columns of them as the width takes, and the rows scroll.
#[must_use]
pub fn layout(count: usize, room: &Room) -> Layout {
    let count = count.max(1);
    let widest = to_count(((room.size.width + room.gap) / (room.card_width.0 + room.gap)).floor())
        .clamp(1, count);
    for columns in 1..=widest {
        let card = room.card(room.width_for(columns));
        let needed = count.div_ceil(columns);
        let rows = room.rows_for(card);
        if needed <= rows {
            return Layout {
                columns,
                rows: needed,
                card,
            };
        }
    }
    let card = room.card(room.width_for(widest));
    Layout {
        columns: widest,
        rows: room.rows_for(card).min(count.div_ceil(widest)),
        card,
    }
}

impl Layout {
    /// The size the cards take on screen.
    #[must_use]
    pub fn size(&self, count: usize, gap: CGFloat) -> NSSize {
        let columns = to_float(self.columns.min(count.max(1)));
        let rows = to_float(self.rows);
        NSSize::new(
            columns * self.card.width + (columns - 1.0) * gap,
            rows * self.card.height + (rows - 1.0).max(0.0) * gap,
        )
    }

    /// The rows on screen when card `selected` must be: from the top, scrolled just enough.
    #[must_use]
    pub fn visible_rows(&self, count: usize, selected: usize) -> Range<usize> {
        let total = count.div_ceil(self.columns.max(1));
        let shown = self.rows.min(total);
        let row = selected / self.columns.max(1);
        let first = (row + 1).saturating_sub(shown).min(total - shown);
        first..first + shown
    }

    /// Where card `index` sits, its row `first_row` at the top of a grid `height` tall; the
    /// origin is the bottom left, as in an unflipped view.
    #[must_use]
    pub fn origin(&self, index: usize, first_row: usize, height: CGFloat, gap: CGFloat) -> NSPoint {
        let columns = self.columns.max(1);
        let column = to_float(index % columns);
        let row = to_float((index / columns).saturating_sub(first_row));
        NSPoint::new(
            column * (self.card.width + gap),
            height - (row + 1.0) * self.card.height - row * gap,
        )
    }

    /// The card under `point` in a grid `height` tall showing rows from `first_row`.
    #[must_use]
    pub fn hit(
        &self,
        point: NSPoint,
        count: usize,
        first_row: usize,
        height: CGFloat,
        gap: CGFloat,
    ) -> Option<usize> {
        let column = (point.x / (self.card.width + gap)).floor();
        let row = ((height - point.y) / (self.card.height + gap)).floor();
        if column < 0.0 || row < 0.0 || to_count(row) >= self.rows {
            return None;
        }
        let (column, row) = (to_count(column), to_count(row) + first_row);
        let index = row * self.columns + column;
        let origin = self.origin(index, first_row, height, gap);
        let inside =
            column < self.columns && point.x <= origin.x + self.card.width && point.y >= origin.y;
        (inside && index < count).then_some(index)
    }
}

/// Where an arrow key takes the selection in a grid of `count` cards, `columns` wide. Moves
/// stop at the edges; down from a row above a shorter last row lands on the last card.
#[must_use]
pub fn step(selected: usize, count: usize, columns: usize, by: Move) -> usize {
    let last = count.saturating_sub(1);
    let columns = columns.max(1);
    let selected = selected.min(last);
    match by {
        Move::Left => selected.saturating_sub(1),
        Move::Right => (selected + 1).min(last),
        Move::Up => selected.checked_sub(columns).unwrap_or(selected),
        Move::Down if selected + columns <= last => selected + columns,
        Move::Down if selected / columns < last / columns => last,
        Move::Down => selected,
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "card and column counts, exact in a float"
)]
fn to_float(count: usize) -> CGFloat {
    count as CGFloat
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a floored, non-negative count of cards or rows"
)]
fn to_count(value: CGFloat) -> usize {
    value.max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use objc2_foundation::{NSPoint, NSSize};

    use super::{Layout, Move, Room, layout, step};

    fn room(width: f64, height: f64) -> Room {
        Room {
            size: NSSize::new(width, height),
            gap: 16.0,
            card_width: (200.0, 320.0),
            chrome: 50.0,
        }
    }

    #[test]
    fn a_few_tabs_get_the_widest_cards_in_one_row_if_it_is_short() {
        let grid = layout(2, &room(1300.0, 300.0));

        assert_eq!(grid.columns, 2);
        assert_eq!(grid.rows, 1);
        assert_eq!(grid.card, NSSize::new(320.0, 250.0));
    }

    #[test]
    fn a_tall_room_stacks_the_fewest_columns_that_fit() {
        let grid = layout(4, &room(1300.0, 2000.0));

        assert_eq!((grid.columns, grid.rows), (1, 4));
        assert_eq!(
            grid.size(4, 16.0),
            NSSize::new(320.0, 4.0 * 250.0 + 3.0 * 16.0)
        );
    }

    #[test]
    fn many_tabs_shrink_the_cards_before_they_scroll() {
        // Four columns need five rows of 246-high cards, more than 900 holds; five columns of
        // 247-wide cards need four rows of 204, which fit.
        let grid = layout(20, &room(1300.0, 900.0));

        assert_eq!(grid.columns, 5);
        assert_eq!(grid.rows, 4);
        assert_eq!(grid.card.width, 247.0);
        assert_eq!(grid.card.height, (247.0_f64 * 10.0 / 16.0 + 50.0).round());
    }

    #[test]
    fn too_many_tabs_scroll_in_the_narrowest_cards() {
        let grid = layout(100, &room(1300.0, 700.0));

        assert_eq!(grid.columns, 6);
        assert_eq!(grid.card.width, 203.0);
        assert_eq!(grid.rows, 3);
        assert_eq!(grid.visible_rows(100, 0), 0..3);
        assert_eq!(grid.visible_rows(100, 17), 0..3);
        assert_eq!(grid.visible_rows(100, 18), 1..4);
        assert_eq!(grid.visible_rows(100, 99), 14..17);
    }

    #[test]
    fn a_narrow_room_still_has_one_column_and_one_row() {
        let grid = layout(3, &room(100.0, 10.0));

        assert_eq!((grid.columns, grid.rows), (1, 1));
        assert_eq!(grid.card.width, 100.0);
        assert_eq!(layout(0, &room(800.0, 800.0)).columns, 1);
    }

    #[test]
    fn cards_fill_rows_from_the_top_left() {
        let grid = Layout {
            columns: 3,
            rows: 2,
            card: NSSize::new(100.0, 50.0),
        };

        assert_eq!(grid.origin(0, 0, 116.0, 16.0), NSPoint::new(0.0, 66.0));
        assert_eq!(grid.origin(4, 0, 116.0, 16.0), NSPoint::new(116.0, 0.0));
        assert_eq!(grid.origin(4, 1, 116.0, 16.0), NSPoint::new(116.0, 66.0));
        assert_eq!(grid.size(2, 16.0), NSSize::new(216.0, 116.0));
    }

    #[test]
    fn a_click_finds_its_card_and_gaps_find_none() {
        let grid = Layout {
            columns: 3,
            rows: 2,
            card: NSSize::new(100.0, 50.0),
        };
        let hit = |x, y| grid.hit(NSPoint::new(x, y), 5, 0, 116.0, 16.0);

        assert_eq!(hit(10.0, 110.0), Some(0));
        assert_eq!(hit(120.0, 10.0), Some(4));
        assert_eq!(hit(240.0, 10.0), None, "no sixth card");
        assert_eq!(hit(105.0, 110.0), None, "between columns");
        assert_eq!(hit(10.0, 60.0), None, "between rows");
        assert_eq!(hit(-1.0, 110.0), None);
        assert_eq!(hit(10.0, 120.0), None, "above the grid");
        assert_eq!(hit(400.0, 110.0), None, "right of the grid");
        assert_eq!(
            grid.hit(NSPoint::new(10.0, 110.0), 9, 1, 116.0, 16.0),
            Some(3)
        );
    }

    #[test]
    fn arrows_move_through_the_grid_and_stop_at_its_edges() {
        // 3 columns, 7 cards: rows 0-2, 3-5, 6.
        assert_eq!(step(0, 7, 3, Move::Left), 0);
        assert_eq!(step(1, 7, 3, Move::Left), 0);
        assert_eq!(step(2, 7, 3, Move::Right), 3);
        assert_eq!(step(6, 7, 3, Move::Right), 6);
        assert_eq!(step(4, 7, 3, Move::Up), 1);
        assert_eq!(step(1, 7, 3, Move::Up), 1);
        assert_eq!(step(1, 7, 3, Move::Down), 4);
        assert_eq!(step(3, 7, 3, Move::Down), 6);
        assert_eq!(step(5, 7, 3, Move::Down), 6, "onto the shorter last row");
        assert_eq!(step(6, 7, 3, Move::Down), 6);
        assert_eq!(step(2, 3, 3, Move::Down), 2);
    }

    #[test]
    fn a_stale_or_empty_selection_stays_in_range() {
        assert_eq!(step(9, 3, 3, Move::Left), 1);
        assert_eq!(step(0, 0, 3, Move::Right), 0);
        assert_eq!(step(0, 4, 0, Move::Down), 1);
    }
}

//! Where the mini player rests: one of the window's four corners, a margin from its edges and
//! clear of the window buttons at the top. A drag ends in the corner the player was flung
//! toward. Pure geometry, so it is tested without a window.

use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::tokens::space;

/// The player's size in a window wide enough: 16:9, as most videos are.
pub const SIZE: NSSize = NSSize::new(320.0, 180.0);

/// The narrowest the player gets in a narrow window.
const MIN_WIDTH: f64 = 160.0;

/// How far it stays from the window's sides and bottom.
pub const MARGIN: f64 = space::XL;

/// How far it stays from the window's top: clear of the window buttons, which float over the
/// page's top left corner.
pub const TOP_MARGIN: f64 = space::XL * 2.0;

/// How far ahead a fling carries the player, in seconds of its speed at release: a quick flick
/// toward a corner lands there even from the middle of the window.
const PROJECTION: f64 = 0.2;

/// A corner of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    fn left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }

    fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }

    /// The corner whose quarter of a window of `bounds` holds `point`; the middle lines go to
    /// the right and the bottom.
    #[must_use]
    pub fn nearest(bounds: NSSize, point: NSPoint, flipped: bool) -> Self {
        let left = point.x < bounds.width / 2.0;
        let top = if flipped {
            point.y < bounds.height / 2.0
        } else {
            point.y > bounds.height / 2.0
        };
        match (top, left) {
            (true, true) => Self::TopLeft,
            (true, false) => Self::TopRight,
            (false, true) => Self::BottomLeft,
            (false, false) => Self::BottomRight,
        }
    }
}

/// The player's size in a window of `bounds`: [`SIZE`], narrowed to fit between the margins,
/// never below [`MIN_WIDTH`], always 16:9.
#[must_use]
pub fn size_in(bounds: NSSize) -> NSSize {
    let width = SIZE
        .width
        .min(bounds.width - MARGIN * 2.0)
        .max(MIN_WIDTH)
        .round();
    NSSize::new(width, (width * SIZE.height / SIZE.width).round())
}

/// Where a player of `size` rests in `corner` of a window of `bounds`. In a `flipped` window,
/// y grows downward.
#[must_use]
pub fn frame(bounds: NSSize, size: NSSize, corner: Corner, flipped: bool) -> NSRect {
    let x = if corner.left() {
        MARGIN
    } else {
        bounds.width - MARGIN - size.width
    };
    let from_top = if corner.top() {
        TOP_MARGIN
    } else {
        bounds.height - MARGIN - size.height
    };
    let y = if flipped {
        from_top
    } else {
        bounds.height - from_top - size.height
    };
    NSRect::new(NSPoint::new(x.round(), y.round()), size)
}

/// Where a player whose center is at `center` heads for, moving at `velocity` points per
/// second when released.
#[must_use]
pub fn projected(center: NSPoint, velocity: NSPoint) -> NSPoint {
    NSPoint::new(
        center.x + velocity.x * PROJECTION,
        center.y + velocity.y * PROJECTION,
    )
}

/// The offset of a drag, cut so that a player at `frame` stays whole inside `bounds`.
#[must_use]
pub fn within(bounds: NSSize, frame: NSRect, offset: NSPoint) -> NSPoint {
    let clamp = |at: f64, length: f64, room: f64, by: f64| {
        let lowest = -at;
        let highest = (room - length - at).max(lowest);
        by.clamp(lowest, highest)
    };
    NSPoint::new(
        clamp(frame.origin.x, frame.size.width, bounds.width, offset.x),
        clamp(frame.origin.y, frame.size.height, bounds.height, offset.y),
    )
}

#[cfg(test)]
mod tests {
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    use super::{Corner, MARGIN, SIZE, TOP_MARGIN, frame, projected, size_in, within};

    const WINDOW: NSSize = NSSize::new(1000.0, 600.0);

    #[test]
    fn each_corner_keeps_the_margins_and_clears_the_window_buttons() {
        let at = |corner| frame(WINDOW, SIZE, corner, false).origin;

        assert_eq!(at(Corner::BottomLeft), NSPoint::new(MARGIN, MARGIN));
        assert_eq!(
            at(Corner::BottomRight),
            NSPoint::new(1000.0 - 24.0 - 320.0, 24.0)
        );
        assert_eq!(
            at(Corner::TopLeft),
            NSPoint::new(24.0, 600.0 - 48.0 - 180.0)
        );
        assert_eq!(at(Corner::TopRight), NSPoint::new(656.0, 372.0));
        assert!((TOP_MARGIN - 48.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_flipped_window_counts_from_the_top() {
        let at = |corner| frame(WINDOW, SIZE, corner, true).origin;

        assert_eq!(at(Corner::TopLeft), NSPoint::new(24.0, 48.0));
        assert_eq!(
            at(Corner::BottomRight),
            NSPoint::new(656.0, 600.0 - 24.0 - 180.0)
        );
    }

    #[test]
    fn the_nearest_corner_is_the_quarter_the_point_is_in() {
        let nearest = |x, y| Corner::nearest(WINDOW, NSPoint::new(x, y), false);

        assert_eq!(nearest(100.0, 500.0), Corner::TopLeft);
        assert_eq!(nearest(900.0, 500.0), Corner::TopRight);
        assert_eq!(nearest(100.0, 100.0), Corner::BottomLeft);
        assert_eq!(nearest(900.0, 100.0), Corner::BottomRight);
        assert_eq!(nearest(500.0, 300.0), Corner::BottomRight);
        assert_eq!(nearest(499.0, 301.0), Corner::TopLeft);
    }

    #[test]
    fn in_a_flipped_window_the_top_is_small_y() {
        let nearest = |x, y| Corner::nearest(WINDOW, NSPoint::new(x, y), true);

        assert_eq!(nearest(100.0, 100.0), Corner::TopLeft);
        assert_eq!(nearest(900.0, 500.0), Corner::BottomRight);
        assert_eq!(nearest(500.0, 300.0), Corner::BottomRight);
    }

    #[test]
    fn a_fling_carries_the_player_ahead() {
        let ahead = projected(NSPoint::new(500.0, 300.0), NSPoint::new(-1000.0, 500.0));

        assert_eq!(ahead, NSPoint::new(300.0, 400.0));
        assert_eq!(
            Corner::nearest(WINDOW, ahead, false),
            Corner::TopLeft,
            "flung up and left from the middle"
        );
    }

    #[test]
    fn a_narrow_window_narrows_the_player_but_not_below_its_least() {
        assert_eq!(size_in(WINDOW), SIZE);
        assert_eq!(
            size_in(NSSize::new(288.0, 400.0)),
            NSSize::new(240.0, 135.0)
        );
        assert_eq!(size_in(NSSize::new(100.0, 400.0)), NSSize::new(160.0, 90.0));
    }

    #[test]
    fn a_drag_stops_at_the_window_s_edges() {
        let player = NSRect::new(NSPoint::new(656.0, 24.0), SIZE);

        assert_eq!(
            within(WINDOW, player, NSPoint::new(100.0, -100.0)),
            NSPoint::new(24.0, -24.0)
        );
        assert_eq!(
            within(WINDOW, player, NSPoint::new(-900.0, 900.0)),
            NSPoint::new(-656.0, 396.0)
        );
        assert_eq!(
            within(WINDOW, player, NSPoint::new(-10.0, 10.0)),
            NSPoint::new(-10.0, 10.0)
        );
    }

    #[test]
    fn a_player_wider_than_the_window_stays_at_its_left_edge() {
        let player = NSRect::new(NSPoint::new(24.0, 24.0), NSSize::new(160.0, 90.0));

        assert_eq!(
            within(NSSize::new(100.0, 400.0), player, NSPoint::new(50.0, 0.0)),
            NSPoint::new(-24.0, 0.0)
        );
    }
}

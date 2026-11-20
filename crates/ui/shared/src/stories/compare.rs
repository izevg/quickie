//! Perceptual comparison of a render with its reference, in pure Rust.
//!
//! Two pixels match when their colour difference, measured in YIQ the way `pixelmatch`
//! measures it, stays under [`Tolerance::delta`]. A pixel that differs is forgiven when it is
//! only shifted: each image has a matching pixel within one pixel of the other's. That absorbs
//! anti-aliasing and glyph hinting moving an edge by a pixel. What is left must not exceed
//! [`Tolerance::max_pixels`], which is small enough that a changed digit fails.

/// An image as 8-bit RGBA rows, top row first, alpha premultiplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// How different a render may be from its reference and still pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// The largest colour difference two pixels may have and still match, 0…1: 0.1 is the
    /// difference between two neighbouring anti-aliasing steps, far below a colour change.
    pub delta: f64,
    /// At most this share of the pixels may differ beyond a one-pixel shift …
    pub share: f64,
    /// … and never fewer than this many, so small images tolerate a stray pixel or two.
    pub floor: usize,
}

/// The tolerance every story is held to.
pub const TOLERANCE: Tolerance = Tolerance {
    delta: 0.1,
    share: 0.000_1,
    floor: 8,
};

impl Tolerance {
    /// How many pixels of an image with `total` pixels may differ.
    #[must_use]
    pub fn max_pixels(&self, total: usize) -> usize {
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a pixel count and a share in 0…1; the product floors into a count"
        )]
        let share = (total as f64 * self.share).floor() as usize;
        share.max(self.floor)
    }
}

/// What a comparison found.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Within tolerance; `differing` pixels differ beyond a shift (at most the allowance).
    Same { differing: usize },
    /// Out of tolerance; `diff` marks differing pixels red and forgiven ones yellow over a
    /// faded copy of the reference.
    Different {
        differing: usize,
        allowed: usize,
        diff: Pixels,
    },
    /// The render has another size than the reference.
    Resized {
        expected: (usize, usize),
        actual: (usize, usize),
    },
}

/// Compares `actual` with the reference `expected`.
#[must_use]
pub fn compare(expected: &Pixels, actual: &Pixels, tolerance: &Tolerance) -> Verdict {
    if (expected.width, expected.height) != (actual.width, actual.height) {
        return Verdict::Resized {
            expected: (expected.width, expected.height),
            actual: (actual.width, actual.height),
        };
    }
    let limit = MAX_DELTA * tolerance.delta * tolerance.delta;
    let mut diff = faded(expected);
    let mut differing = 0;
    for y in 0..expected.height {
        for x in 0..expected.width {
            let (e, a) = (expected.at(x, y), actual.at(x, y));
            if delta(e, a) <= limit {
                continue;
            }
            let shifted = near(actual, x, y, e, limit) && near(expected, x, y, a, limit);
            let mark = if shifted {
                [255, 200, 0, 255]
            } else {
                differing += 1;
                [255, 0, 0, 255]
            };
            let i = (y * expected.width + x) * 4;
            diff.rgba[i..i + 4].copy_from_slice(&mark);
        }
    }
    let allowed = tolerance.max_pixels(expected.width * expected.height);
    if differing <= allowed {
        Verdict::Same { differing }
    } else {
        Verdict::Different {
            differing,
            allowed,
            diff,
        }
    }
}

/// The largest possible YIQ difference (`pixelmatch`'s constant).
const MAX_DELTA: f64 = 35_215.0;

/// True when `image` has a pixel matching `color` within one pixel of (`x`, `y`).
fn near(image: &Pixels, x: usize, y: usize, color: [u8; 4], limit: f64) -> bool {
    let xs = x.saturating_sub(1)..=(x + 1).min(image.width - 1);
    xs.into_iter().any(|nx| {
        let ys = y.saturating_sub(1)..=(y + 1).min(image.height - 1);
        ys.into_iter()
            .any(|ny| delta(image.at(nx, ny), color) <= limit)
    })
}

/// The squared, weighted YIQ difference of two premultiplied pixels laid over white.
fn delta(a: [u8; 4], b: [u8; 4]) -> f64 {
    if a == b {
        return 0.0;
    }
    let (ya, ia, qa) = yiq(a);
    let (yb, ib, qb) = yiq(b);
    let (dy, di, dq) = (ya - yb, ia - ib, qa - qb);
    0.5053 * dy * dy + 0.299 * di * di + 0.1957 * dq * dq
}

fn yiq(pixel: [u8; 4]) -> (f64, f64, f64) {
    let white = 255.0 - f64::from(pixel[3]);
    let [r, g, b] = [0, 1, 2].map(|c| f64::from(pixel[c]) + white);
    (
        r * 0.298_895_31 + g * 0.586_622_47 + b * 0.114_482_23,
        r * 0.595_977_99 - g * 0.274_176_10 - b * 0.321_801_89,
        r * 0.211_470_17 - g * 0.522_617_11 + b * 0.311_146_94,
    )
}

/// The reference in grey at a quarter of its contrast, the backdrop of a diff image.
fn faded(image: &Pixels) -> Pixels {
    let mut rgba = Vec::with_capacity(image.rgba.len());
    for pixel in image.rgba.as_chunks::<4>().0 {
        let (luma, _, _) = yiq(*pixel);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a luma in 0…255, faded toward white, stays a byte"
        )]
        let grey = (255.0 - (255.0 - luma.clamp(0.0, 255.0)) * 0.25).round() as u8;
        rgba.extend_from_slice(&[grey, grey, grey, 255]);
    }
    Pixels {
        width: image.width,
        height: image.height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::{Pixels, TOLERANCE, Tolerance, Verdict, compare};

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn image(width: usize, height: usize, paint: impl Fn(usize, usize) -> [u8; 4]) -> Pixels {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&paint(x, y));
            }
        }
        Pixels {
            width,
            height,
            rgba,
        }
    }

    /// A black vertical bar two pixels wide at `left` on white.
    fn bar(left: usize) -> Pixels {
        image(40, 40, |x, _| {
            if (left..left + 2).contains(&x) {
                BLACK
            } else {
                WHITE
            }
        })
    }

    #[test]
    fn equal_images_are_the_same() {
        assert_eq!(
            compare(&bar(10), &bar(10), &TOLERANCE),
            Verdict::Same { differing: 0 }
        );
    }

    #[test]
    fn a_one_pixel_shift_is_forgiven() {
        assert_eq!(
            compare(&bar(10), &bar(11), &TOLERANCE),
            Verdict::Same { differing: 0 }
        );
    }

    #[test]
    fn a_two_pixel_shift_is_a_difference() {
        let verdict = compare(&bar(10), &bar(13), &TOLERANCE);

        assert!(
            matches!(
                verdict,
                Verdict::Different {
                    differing: 160,
                    allowed: 8,
                    ..
                }
            ),
            "{verdict:?}"
        );
    }

    #[test]
    fn a_faint_change_stays_within_the_delta() {
        let grey = |level: u8| image(8, 8, move |_, _| [level, level, level, 255]);

        assert_eq!(
            compare(&grey(200), &grey(204), &TOLERANCE),
            Verdict::Same { differing: 0 }
        );
        assert!(matches!(
            compare(&grey(200), &grey(120), &TOLERANCE),
            Verdict::Different { differing: 64, .. }
        ));
    }

    #[test]
    fn a_few_stray_pixels_pass_and_more_fail() {
        let speckled = |count: usize| {
            image(40, 40, move |x, y| {
                if y == 20 && x % 4 == 0 && x / 4 < count {
                    BLACK
                } else {
                    WHITE
                }
            })
        };
        let plain = image(40, 40, |_, _| WHITE);

        assert_eq!(
            compare(&plain, &speckled(8), &TOLERANCE),
            Verdict::Same { differing: 8 }
        );
        assert!(matches!(
            compare(&plain, &speckled(9), &TOLERANCE),
            Verdict::Different {
                differing: 9,
                allowed: 8,
                ..
            }
        ));
    }

    #[test]
    fn the_diff_marks_differences_red_and_shifts_yellow() {
        let Verdict::Different { diff, .. } = compare(&bar(10), &bar(13), &TOLERANCE) else {
            panic!("expected a difference");
        };
        let at = |x: usize| {
            let i = (5 * diff.width + x) * 4;
            [diff.rgba[i], diff.rgba[i + 1], diff.rgba[i + 2]]
        };

        assert_eq!(at(10), [255, 0, 0]);
        assert_eq!(at(0), [255, 255, 255]);
    }

    #[test]
    fn a_resized_render_is_reported_with_both_sizes() {
        let small = image(4, 4, |_, _| WHITE);
        let large = image(4, 5, |_, _| WHITE);

        assert_eq!(
            compare(&small, &large, &TOLERANCE),
            Verdict::Resized {
                expected: (4, 4),
                actual: (4, 5)
            }
        );
    }

    #[test]
    fn transparency_is_judged_over_white() {
        let clear = image(4, 4, |_, _| [0, 0, 0, 0]);
        let white = image(4, 4, |_, _| WHITE);

        assert_eq!(
            compare(&clear, &white, &TOLERANCE),
            Verdict::Same { differing: 0 }
        );
    }

    #[test]
    fn the_allowance_grows_with_the_image() {
        let tolerance = Tolerance {
            delta: 0.1,
            share: 0.001,
            floor: 8,
        };

        assert_eq!(tolerance.max_pixels(1_000), 8);
        assert_eq!(tolerance.max_pixels(100_000), 100);
        assert_eq!(TOLERANCE.max_pixels(700 * 400), 28);
    }
}

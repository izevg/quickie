//! The fence a page travels in, and the cleaning that keeps a page from forging it or hiding
//! text in it.
//!
//! A fence line is `⟦page N MARKER⟧` or `⟦/page N MARKER⟧`, where the [`FenceMarker`] is drawn
//! at random for each request, so a page cannot predict the exact line it would have to write.
//! It could not write it anyway: [`clean`] turns `⟦`, `⟧`, and every bracket that looks like them
//! into plain `[` and `]`.

use std::fmt::{self, Write};

/// Opens a fence line. Page content never contains it, as [`clean`] replaces it.
pub(super) const FENCE_OPEN: char = '⟦';
/// Closes a fence line; see [`FENCE_OPEN`].
pub(super) const FENCE_CLOSE: char = '⟧';

/// Brackets that look like the fence's, or like plain square brackets, in some font: each pair
/// is folded to `[` and `]` so a page cannot draw a fence line a model would take for ours.
const OPEN_LOOKALIKES: &[char] = &[
    FENCE_OPEN, '〚', '⟬', '⸨', '［', '【', '〖', '⁅', '⦋', '⦍', '⦏',
];
/// The closing halves of [`OPEN_LOOKALIKES`].
const CLOSE_LOOKALIKES: &[char] = &[
    FENCE_CLOSE,
    '〛',
    '⟭',
    '⸩',
    '］',
    '】',
    '〗',
    '⁆',
    '⦌',
    '⦎',
    '⦐',
];

/// A code drawn at random for one request, in both fence lines of every page, so a page
/// cannot predict them. Twelve hexadecimal digits from 48 random bits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FenceMarker(String);

impl FenceMarker {
    /// The marker made of `bytes`, which must come from a source of randomness the page cannot
    /// see or influence.
    #[must_use]
    pub fn from_random(bytes: [u8; 6]) -> Self {
        let mut hex = String::with_capacity(2 * bytes.len());
        for byte in bytes {
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }

    /// The marker as it appears in the fence lines.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FenceMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Page text made safe to fence:
///
/// - brackets that look like the fence's become plain `[` and `]`;
/// - control characters go, and so do invisible format characters (zero-width characters,
///   bidirectional controls, Unicode tag characters), which a model reads and a person does not
///   see, so a page cannot hide instructions in them;
/// - whitespace collapses to a space, a line break, or a paragraph break, which also saves
///   tokens.
pub(super) fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    let mut breaks = 0_usize;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
            breaks += usize::from(c == '\n');
            continue;
        }
        if c.is_control() || is_format(c) {
            continue;
        }
        if space && !out.is_empty() {
            out.push_str(match breaks {
                0 => " ",
                1 => "\n",
                _ => "\n\n",
            });
        }
        space = false;
        breaks = 0;
        out.push(fold(c));
    }
    out
}

/// A fence lookalike as a plain bracket; any other character as it is.
fn fold(c: char) -> char {
    if OPEN_LOOKALIKES.contains(&c) {
        '['
    } else if CLOSE_LOOKALIKES.contains(&c) {
        ']'
    } else {
        c
    }
}

/// True for the invisible format characters of Unicode (general category Cf) that a page could
/// hide text with or reorder text by. Whitespace among them, such as U+2028, never gets here.
fn is_format(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

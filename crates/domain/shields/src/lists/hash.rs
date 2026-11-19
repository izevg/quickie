//! A fingerprint of a fetched list's content.

use std::fmt;

/// The 64-bit FNV-1a hash of a list's content: tells a changed list from an unchanged one, so
/// an unchanged list is neither saved nor compiled again, and the rules the engine compiled can
/// be matched to the content they came from. Stable across builds and platforms, unlike the
/// standard library's hasher; not a defense against a forged list, which https is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(u64);

impl ContentHash {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// The hash of `content`.
    #[must_use]
    pub fn of(content: &[u8]) -> Self {
        Self(content.iter().fold(Self::OFFSET_BASIS, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(Self::PRIME)
        }))
    }

    /// Wraps a raw value, as read from storage.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for storage.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Sixteen lowercase hex digits.
impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

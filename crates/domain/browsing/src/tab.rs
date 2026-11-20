//! A tab and the value objects around it: cookie jars, pinned slots, the origin of its page,
//! and the matches of a find on it.

mod find;
mod origin;

use quickie_domain_kernel::TabId;
use url::Url;

pub use find::FindMatches;
pub use origin::{Origin, Transport, is_cross_origin, registrable_domain};

/// Identifies a throwaway jar: an ephemeral cookie and storage partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JarId(u64);

impl JarId {
    /// Wraps a raw value, as read from storage.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for storage.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Where a tab keeps its cookies and storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Jar {
    /// The profile's own storage: logins persist.
    Main,
    /// An ephemeral partition that dies with the last tab using it. Popups opened from a
    /// throwaway tab share it, so sign-in flows keep working inside the jar.
    Throwaway(JarId),
}

/// The jar for a tab being opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JarChoice {
    /// The profile's own storage.
    Main,
    /// A fresh throwaway jar.
    NewThrowaway,
    /// The jar of the opener tab: a popup or a ⌘-click stays in its opener's jar.
    SameAs(TabId),
}

/// Whether a newly opened tab becomes active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// Becomes the active tab of its window.
    Foreground,
    /// Joins the back of the most-recently-used order, so it never replaces "previous tab".
    Background,
}

/// A live tab, as the browsing context sees it. The engine owns the page; this is what persists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    /// Never reused, kept through the trash, so a reopened tab finds its navigation history.
    pub id: TabId,
    /// The committed URL.
    pub url: Url,
    /// The page title; empty until the engine reports one.
    pub title: String,
    /// Where cookies and storage live.
    pub jar: Jar,
}

impl Tab {
    /// The page as a Markdown link, `[title](url)` (`⌥⌘C`). A page without a title is named
    /// by its URL. Brackets and backslashes in the title are escaped, and parentheses in the
    /// URL are percent-encoded, so the link survives any Markdown renderer.
    #[must_use]
    pub fn markdown_link(&self) -> String {
        let name = if self.title.trim().is_empty() {
            self.url.as_str()
        } else {
            self.title.trim()
        };
        let mut text = String::with_capacity(name.len());
        for c in name.chars() {
            if matches!(c, '\\' | '[' | ']') {
                text.push('\\');
            }
            text.push(c);
        }
        let target = self.url.as_str().replace('(', "%28").replace(')', "%29");
        format!("[{text}]({target})")
    }
}

/// One of the nine pinned slots, `⌘1` … `⌘9`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PinnedSlot(u8);

impl PinnedSlot {
    /// Number of slots.
    pub const COUNT: usize = 9;

    /// The slot number, 1 to 9, as on the key.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Zero-based position, for an array of [`Self::COUNT`] slots.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize - 1
    }

    /// The slot at a zero-based position, if there is one.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        let n = u8::try_from(index).ok()?.checked_add(1)?;
        Self::try_from(n).ok()
    }
}

/// A slot number outside 1 to 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("pinned slot {0} is not between 1 and 9")]
pub struct SlotOutOfRange(pub u8);

impl TryFrom<u8> for PinnedSlot {
    type Error = SlotOutOfRange;

    fn try_from(n: u8) -> Result<Self, Self::Error> {
        if (1..=9).contains(&n) {
            Ok(Self(n))
        } else {
            Err(SlotOutOfRange(n))
        }
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_kernel::{Profile, TabId};
    use url::Url;

    use super::{Jar, JarId, PinnedSlot, SlotOutOfRange, Tab};

    fn tab(url: &str, title: &str) -> Tab {
        Tab {
            id: TabId::new(Profile::Main, 1),
            url: Url::parse(url).unwrap(),
            title: title.to_owned(),
            jar: Jar::Main,
        }
    }

    #[test]
    fn a_markdown_link_names_the_page_by_its_title() {
        let page = tab("https://example.com/a", "  Example page ");

        assert_eq!(
            page.markdown_link(),
            "[Example page](https://example.com/a)"
        );
    }

    #[test]
    fn a_page_without_a_title_is_named_by_its_url() {
        for title in ["", "   "] {
            let page = tab("https://example.com/", title);

            assert_eq!(
                page.markdown_link(),
                "[https://example.com/](https://example.com/)"
            );
        }
    }

    #[test]
    fn a_markdown_link_escapes_what_would_break_it() {
        let page = tab(
            "https://en.wikipedia.org/wiki/Rust_(language)",
            r"Rust [lang] \ docs",
        );

        assert_eq!(
            page.markdown_link(),
            r"[Rust \[lang\] \\ docs](https://en.wikipedia.org/wiki/Rust_%28language%29)"
        );
    }

    #[test]
    fn slots_one_to_nine_exist() {
        let first = PinnedSlot::try_from(1);
        let last = PinnedSlot::try_from(9);

        assert_eq!(first.map(PinnedSlot::get), Ok(1));
        assert_eq!(last.map(PinnedSlot::get), Ok(9));
    }

    #[test]
    fn slots_outside_one_to_nine_are_rejected() {
        assert_eq!(PinnedSlot::try_from(0), Err(SlotOutOfRange(0)));
        assert_eq!(PinnedSlot::try_from(10), Err(SlotOutOfRange(10)));
    }

    #[test]
    fn index_is_zero_based() {
        let slot = PinnedSlot::try_from(3).unwrap();

        assert_eq!(slot.index(), 2);
        assert_eq!(PinnedSlot::from_index(2), Some(slot));
    }

    #[test]
    fn from_index_stops_after_the_last_slot() {
        assert_eq!(PinnedSlot::from_index(8).map(PinnedSlot::get), Some(9));
        assert_eq!(PinnedSlot::from_index(9), None);
        assert_eq!(PinnedSlot::from_index(usize::MAX), None);
    }

    #[test]
    fn jar_id_round_trips() {
        assert_eq!(JarId::new(5).get(), 5);
    }
}

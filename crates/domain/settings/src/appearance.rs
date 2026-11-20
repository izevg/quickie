//! How the browser window shows its page.

/// How the page sits in its window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageLayout {
    /// The page fills the window edge to edge: the default, as *Content first* asks.
    #[default]
    EdgeToEdge,
    /// The page is a card over the window's background, inset on every side, and the inset is
    /// the window's chrome (*Inset page*).
    Inset,
}

/// What the user chose about how the browser looks. A fresh profile has the defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appearance {
    page: PageLayout,
}

/// The appearance as plain data, for the store to write and read back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppearanceSnapshot {
    /// *Inset page* is on.
    pub inset_page: bool,
}

impl Appearance {
    /// The defaults: the page edge to edge.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How the page sits in its window.
    #[must_use]
    pub fn page(self) -> PageLayout {
        self.page
    }

    /// Lays the page out as `page`; returns whether that changed anything.
    pub fn set_page(&mut self, page: PageLayout) -> bool {
        let changed = self.page != page;
        self.page = page;
        changed
    }

    /// Rebuilds the appearance from a snapshot. Every snapshot is a valid appearance.
    #[must_use]
    pub fn restore(snapshot: AppearanceSnapshot) -> Self {
        let page = if snapshot.inset_page {
            PageLayout::Inset
        } else {
            PageLayout::EdgeToEdge
        };
        Self { page }
    }

    /// The appearance as plain data; `restore(snapshot())` gives back an equal appearance.
    #[must_use]
    pub fn snapshot(self) -> AppearanceSnapshot {
        AppearanceSnapshot {
            inset_page: self.page == PageLayout::Inset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Appearance, AppearanceSnapshot, PageLayout};

    #[test]
    fn a_fresh_profile_shows_the_page_edge_to_edge() {
        assert_eq!(Appearance::new().page(), PageLayout::EdgeToEdge);
        assert!(!Appearance::new().snapshot().inset_page);
    }

    #[test]
    fn setting_the_layout_says_whether_it_changed() {
        let mut appearance = Appearance::new();

        assert!(appearance.set_page(PageLayout::Inset));
        assert_eq!(appearance.page(), PageLayout::Inset);
        assert!(!appearance.set_page(PageLayout::Inset));
        assert!(appearance.set_page(PageLayout::EdgeToEdge));
        assert_eq!(appearance.page(), PageLayout::EdgeToEdge);
        assert!(!appearance.set_page(PageLayout::EdgeToEdge));
    }

    #[test]
    fn round_trips_through_its_snapshot() {
        let inset = Appearance::restore(AppearanceSnapshot { inset_page: true });
        let edge = Appearance::restore(AppearanceSnapshot { inset_page: false });

        assert_eq!(inset.page(), PageLayout::Inset);
        assert_eq!(edge.page(), PageLayout::EdgeToEdge);
        assert_eq!(inset.snapshot(), AppearanceSnapshot { inset_page: true });
        assert_eq!(edge.snapshot(), AppearanceSnapshot { inset_page: false });
    }
}

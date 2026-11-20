//! How the browser looks: the user's appearance choices, saved.
//!
//! The same transaction rule as the other use cases: a change is saved first, and only then
//! does it replace the appearance, so a failed save changes nothing.

use quickie_domain_settings::{Appearance, PageLayout};

use crate::{AppError, AppearanceRepo};

/// The appearance use cases over one profile.
#[derive(Debug)]
pub struct AppearanceSettings<R> {
    appearance: Appearance,
    repo: R,
}

impl<R: AppearanceRepo> AppearanceSettings<R> {
    /// Loads the saved appearance.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    pub fn start(repo: R) -> Result<Self, AppError> {
        let appearance = Appearance::restore(repo.load_appearance()?);
        Ok(Self { appearance, repo })
    }

    /// The appearance now.
    #[must_use]
    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Lays the page out as `page` and saves it; returns whether that changed anything.
    ///
    /// # Errors
    ///
    /// The store failed; nothing changed.
    pub fn set_page(&mut self, page: PageLayout) -> Result<bool, AppError> {
        let mut next = self.appearance;
        if !next.set_page(page) {
            return Ok(false);
        }
        self.repo.save_appearance(next.snapshot())?;
        self.appearance = next;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_settings::{AppearanceSnapshot, PageLayout};

    use super::AppearanceSettings;
    use crate::{AppearanceRepo, StoreError};

    /// Keeps what was saved last, like a store would; counts saves, and fails on request.
    #[derive(Debug, Default)]
    struct FakeAppearanceRepo {
        saved: AppearanceSnapshot,
        saves: usize,
        load_fails: bool,
        save_fails: bool,
    }

    impl AppearanceRepo for FakeAppearanceRepo {
        fn load_appearance(&self) -> Result<AppearanceSnapshot, StoreError> {
            if self.load_fails {
                return Err(StoreError("load".to_owned()));
            }
            Ok(self.saved)
        }

        fn save_appearance(&mut self, appearance: AppearanceSnapshot) -> Result<(), StoreError> {
            if self.save_fails {
                return Err(StoreError("save".to_owned()));
            }
            self.saves += 1;
            self.saved = appearance;
            Ok(())
        }
    }

    #[test]
    fn starts_with_what_was_saved() {
        let repo = FakeAppearanceRepo {
            saved: AppearanceSnapshot { inset_page: true },
            ..FakeAppearanceRepo::default()
        };

        let settings = AppearanceSettings::start(repo).unwrap();

        assert_eq!(settings.appearance().page(), PageLayout::Inset);
    }

    #[test]
    fn a_store_that_cannot_be_read_fails_the_start() {
        let repo = FakeAppearanceRepo {
            load_fails: true,
            ..FakeAppearanceRepo::default()
        };

        assert!(AppearanceSettings::start(repo).is_err());
    }

    #[test]
    fn a_change_is_saved_and_kept() {
        let mut settings = AppearanceSettings::start(FakeAppearanceRepo::default()).unwrap();

        assert_eq!(settings.set_page(PageLayout::Inset), Ok(true));

        assert_eq!(settings.appearance().page(), PageLayout::Inset);
        assert_eq!(settings.repo.saved, AppearanceSnapshot { inset_page: true });
        assert_eq!(settings.repo.saves, 1);
    }

    #[test]
    fn choosing_what_is_already_there_saves_nothing() {
        let mut settings = AppearanceSettings::start(FakeAppearanceRepo::default()).unwrap();

        assert_eq!(settings.set_page(PageLayout::EdgeToEdge), Ok(false));

        assert_eq!(settings.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let repo = FakeAppearanceRepo {
            save_fails: true,
            ..FakeAppearanceRepo::default()
        };
        let mut settings = AppearanceSettings::start(repo).unwrap();

        assert!(settings.set_page(PageLayout::Inset).is_err());

        assert_eq!(settings.appearance().page(), PageLayout::EdgeToEdge);
    }
}

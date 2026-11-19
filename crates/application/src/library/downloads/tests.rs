use quickie_domain_browsing::{Jar, JarId, Timestamp};
use quickie_domain_library::{
    CorruptDownloads, Download, DownloadError, DownloadId, DownloadState, Downloads,
    DownloadsSnapshot,
};
use url::Url;

use super::INTERRUPTED;
use crate::fakes::{FakeClock, FakeLibraryRepo};
use crate::library::{CorruptLibrary, Library};
use crate::{AppError, LibrarySnapshot, StoreError};

type Subject = Library<FakeLibraryRepo, FakeClock>;

fn url(name: &str) -> Url {
    Url::parse(&format!("https://files.test/{name}")).unwrap()
}

fn throwaway() -> Jar {
    Jar::Throwaway(JarId::new(4))
}

fn started() -> Subject {
    Library::start(FakeLibraryRepo::default(), FakeClock::default()).unwrap()
}

fn start_with(downloads: DownloadsSnapshot) -> Subject {
    let repo = FakeLibraryRepo {
        saved: LibrarySnapshot {
            downloads: Some(downloads),
            ..LibrarySnapshot::default()
        },
        ..FakeLibraryRepo::default()
    };
    Library::start(repo, FakeClock::default()).unwrap()
}

fn begin(library: &mut Subject, name: &str, jar: Jar) -> DownloadId {
    library
        .download_started(url(name), name.to_owned(), jar)
        .unwrap()
}

fn state(library: &Subject, id: DownloadId) -> Option<DownloadState> {
    library
        .downloads()
        .get(id)
        .map(|download| download.state.clone())
}

/// The ids of the downloads the store has, in order.
fn saved_ids(library: &Subject) -> Vec<DownloadId> {
    library
        .repo
        .saved
        .downloads
        .as_ref()
        .map_or_else(Vec::new, |saved| {
            saved.downloads.iter().map(|download| download.id).collect()
        })
}

fn fail_saves(library: &mut Subject) {
    library.repo.save_fails = true;
}

fn store_failure() -> AppError {
    AppError::Store(StoreError("save".to_owned()))
}

mod starting {
    use super::*;

    fn download(id: u64, state: DownloadState) -> Download {
        Download {
            id: DownloadId::new(id),
            url: url("a.zip"),
            file_name: "a.zip".to_owned(),
            state,
            started_at: Timestamp::from_millis(1),
        }
    }

    #[test]
    fn a_first_launch_has_no_downloads() {
        let library = started();

        assert_eq!(library.downloads(), &Downloads::new());
    }

    #[test]
    fn saved_downloads_come_back_and_unfinished_ones_failed() {
        let running = DownloadState::InProgress {
            received: 5,
            total: Some(9),
        };

        let library = start_with(DownloadsSnapshot {
            downloads: vec![download(0, DownloadState::Finished), download(1, running)],
            next_id: 2,
        });

        assert_eq!(
            state(&library, DownloadId::new(0)),
            Some(DownloadState::Finished)
        );
        assert_eq!(
            state(&library, DownloadId::new(1)),
            Some(DownloadState::Failed(INTERRUPTED.to_owned()))
        );
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn corrupt_downloads_start_empty_and_say_why() {
        let library = start_with(DownloadsSnapshot {
            downloads: vec![download(3, DownloadState::Finished)],
            next_id: 3,
        });

        assert_eq!(
            library.corrupt_on_start(),
            [CorruptLibrary::Downloads(CorruptDownloads::IdNotAllocated(
                DownloadId::new(3)
            ))]
        );
        assert_eq!(library.downloads(), &Downloads::new());
    }
}

mod starting_a_download {
    use super::*;

    #[test]
    fn a_main_jar_download_is_listed_at_the_clock_and_saved() {
        let mut library = started();
        library.clock.0.set(70);

        let id = begin(&mut library, "a.zip", Jar::Main);

        let download = library.downloads().get(id).cloned().unwrap();
        assert_eq!(download.started_at, Timestamp::from_millis(70));
        assert_eq!(download.file_name, "a.zip");
        assert_eq!(download.url, url("a.zip"));
        assert!(!library.is_session_only(id));
        assert_eq!(library.repo.saves, 1);
        assert_eq!(saved_ids(&library), [id]);
    }

    #[test]
    fn a_throwaway_jar_download_is_listed_but_never_saved() {
        let mut library = started();

        let id = begin(&mut library, "secret.pdf", throwaway());

        assert!(library.downloads().get(id).is_some());
        assert!(library.is_session_only(id));
        assert_eq!(library.repo.saves, 0);

        let main = begin(&mut library, "b.zip", Jar::Main);

        assert_eq!(saved_ids(&library), [main]);
        assert_eq!(
            library.repo.saved.downloads.as_ref().map(|s| s.next_id),
            Some(2)
        );
    }

    #[test]
    fn a_failed_save_lists_nothing() {
        let mut library = started();
        fail_saves(&mut library);

        let result = library.download_started(url("a.zip"), "a.zip".to_owned(), Jar::Main);

        assert_eq!(result, Err(store_failure()));
        assert_eq!(library.downloads(), &Downloads::new());
    }
}

mod moving {
    use super::*;

    #[test]
    fn progress_is_kept_in_memory_only() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);

        library.download_progressed(id, 4, Some(8)).unwrap();

        assert_eq!(
            state(&library, id),
            Some(DownloadState::InProgress {
                received: 4,
                total: Some(8)
            })
        );
        assert_eq!(library.repo.saves, 1);
    }

    #[test]
    fn progress_of_an_unknown_download_is_an_error() {
        let mut library = started();
        let ghost = DownloadId::new(9);

        assert_eq!(
            library.download_progressed(ghost, 1, None),
            Err(AppError::Download(DownloadError::Unknown(ghost)))
        );
    }

    #[test]
    fn each_ending_is_saved() {
        let mut library = started();
        let finished = begin(&mut library, "a.zip", Jar::Main);
        let failed = begin(&mut library, "b.zip", Jar::Main);
        let cancelled = begin(&mut library, "c.zip", Jar::Main);

        library.download_finished(finished).unwrap();
        library
            .download_failed(failed, "disk full".to_owned())
            .unwrap();
        library.cancel_download(cancelled).unwrap();

        assert_eq!(state(&library, finished), Some(DownloadState::Finished));
        assert_eq!(
            state(&library, failed),
            Some(DownloadState::Failed("disk full".to_owned()))
        );
        assert_eq!(state(&library, cancelled), Some(DownloadState::Cancelled));
        assert_eq!(library.repo.saves, 6);
        let saved = library.repo.saved.downloads.clone().unwrap();
        assert_eq!(Downloads::restore(saved).as_ref(), Ok(library.downloads()));
    }

    #[test]
    fn a_throwaway_download_ends_without_a_save() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", throwaway());

        library.download_finished(id).unwrap();

        assert_eq!(state(&library, id), Some(DownloadState::Finished));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn an_ended_download_does_not_end_again() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.cancel_download(id).unwrap();

        assert_eq!(
            library.download_failed(id, "late".to_owned()),
            Err(AppError::Download(DownloadError::Ended(id)))
        );
        assert_eq!(state(&library, id), Some(DownloadState::Cancelled));
        assert_eq!(library.repo.saves, 2);
    }

    #[test]
    fn a_failed_save_keeps_the_download_running() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);
        fail_saves(&mut library);

        assert_eq!(library.download_finished(id), Err(store_failure()));
        assert_eq!(
            state(&library, id),
            Some(DownloadState::InProgress {
                received: 0,
                total: None
            })
        );
    }
}

mod retrying {
    use super::*;

    #[test]
    fn a_retry_takes_the_ended_download_off_and_saves() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.download_failed(id, "reset".to_owned()).unwrap();

        let retried = library.retry_download(id).unwrap();

        assert_eq!(retried.url, url("a.zip"));
        assert_eq!(library.downloads().get(id), None);
        assert_eq!(library.repo.saves, 3);
        assert_eq!(saved_ids(&library), []);
    }

    #[test]
    fn a_running_download_is_not_retried() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);

        assert_eq!(
            library.retry_download(id),
            Err(AppError::Download(DownloadError::Running(id)))
        );
        assert!(library.downloads().get(id).is_some());
    }

    #[test]
    fn a_throwaway_retry_forgets_the_download_without_a_save() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", throwaway());
        library.cancel_download(id).unwrap();

        library.retry_download(id).unwrap();

        assert!(!library.is_session_only(id));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_keeps_the_download_listed() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.cancel_download(id).unwrap();
        fail_saves(&mut library);

        assert_eq!(library.retry_download(id), Err(store_failure()));
        assert!(library.downloads().get(id).is_some());
    }
}

mod clearing {
    use super::*;

    #[test]
    fn clearing_removes_the_ended_downloads_and_saves() {
        let mut library = started();
        let running = begin(&mut library, "a.zip", Jar::Main);
        let done = begin(&mut library, "b.zip", Jar::Main);
        library.download_finished(done).unwrap();

        let cleared = library.clear_finished_downloads().unwrap();

        assert_eq!(cleared.iter().map(|d| d.id).collect::<Vec<_>>(), [done]);
        assert_eq!(saved_ids(&library), [running]);
        assert_eq!(library.repo.saves, 4);
    }

    #[test]
    fn clearing_nothing_saves_nothing() {
        let mut library = started();
        begin(&mut library, "a.zip", Jar::Main);

        let cleared = library.clear_finished_downloads().unwrap();

        assert_eq!(cleared, []);
        assert_eq!(library.repo.saves, 1);
    }

    #[test]
    fn cleared_throwaway_downloads_are_forgotten() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", throwaway());
        library.download_finished(id).unwrap();

        library.clear_finished_downloads().unwrap();

        assert!(!library.is_session_only(id));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_clears_nothing() {
        let mut library = started();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.download_finished(id).unwrap();
        fail_saves(&mut library);

        assert_eq!(library.clear_finished_downloads(), Err(store_failure()));
        assert!(library.downloads().get(id).is_some());
    }
}

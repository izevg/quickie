use std::path::PathBuf;

use quickie_domain_browsing::{Jar, JarId, Profile, TabId, Timestamp};
use quickie_domain_library::{
    CorruptDownloads, Download, DownloadError, DownloadId, DownloadState, Downloads,
    DownloadsSnapshot,
};
use url::Url;

use super::{DownloadMoved, INTERRUPTED};
use crate::fakes::{FakeClock, FakeLibraryRepo};
use crate::library::{CorruptLibrary, Library};
use crate::{AppError, DownloadEvent, DownloadToken, EngineError, LibrarySnapshot, StoreError};

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

fn tab(serial: u64) -> TabId {
    TabId::new(Profile::Main, serial)
}

/// Records what the library asked of the engine's downloads; refuses every fetch when
/// `refuses`.
#[derive(Debug, Default)]
struct FakeDownloads {
    cancelled: Vec<DownloadToken>,
    fetched: Vec<(TabId, Url)>,
    refuses: bool,
}

impl crate::Downloads for FakeDownloads {
    fn set_directory(&mut self, _directory: PathBuf) {}

    fn download(&mut self, tab: TabId, url: &Url) -> Result<(), EngineError> {
        if self.refuses {
            return Err(EngineError("no page".to_owned()));
        }
        self.fetched.push((tab, url.clone()));
        Ok(())
    }

    fn cancel(&mut self, token: DownloadToken) {
        self.cancelled.push(token);
    }

    fn save_pdf(&mut self, _tab: TabId) -> Result<(), EngineError> {
        Ok(())
    }
}

/// The engine started download `token` of `name` from tab `serial`'s page in `jar`.
fn started_event(token: u64, serial: u64, name: &str, jar: Jar) -> DownloadEvent {
    DownloadEvent::Started {
        token: DownloadToken::new(token),
        tab: tab(serial),
        jar,
        url: url(name),
        file: PathBuf::from("/downloads").join(name),
    }
}

/// The engine gave up on download `token`.
fn failed(token: u64) -> DownloadEvent {
    DownloadEvent::Failed {
        token: DownloadToken::new(token),
        reason: "reset".to_owned(),
    }
}

/// Lists download `token` of `name`, as the engine started it from tab `serial`'s page.
fn fetch(
    library: &mut Subject,
    engine: &mut FakeDownloads,
    token: u64,
    serial: u64,
    name: &str,
    jar: Jar,
) -> DownloadId {
    match library.download_moved(started_event(token, serial, name, jar), engine) {
        Ok(DownloadMoved::Started { id, .. }) => id,
        other => panic!("{other:?} is no start"),
    }
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
        library
            .cancel_download(cancelled, &mut FakeDownloads::default())
            .unwrap();

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
        library
            .cancel_download(id, &mut FakeDownloads::default())
            .unwrap();

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

    /// Every tab is open.
    fn open(_: TabId) -> bool {
        true
    }

    /// No tab is open.
    fn closed(_: TabId) -> bool {
        false
    }

    #[test]
    fn a_retry_takes_the_ended_download_off_and_fetches_it_from_its_tab() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.download_failed(id, "reset".to_owned()).unwrap();

        let retried = library.retry_download(id, &mut engine, open, Some(tab(2)));

        assert_eq!(retried, Ok(true));
        assert_eq!(engine.fetched, [(tab(1), url("a.zip"))]);
        assert_eq!(library.downloads().get(id), None);
        assert_eq!(library.repo.saves, 3);
        assert_eq!(saved_ids(&library), []);
    }

    #[test]
    fn a_download_whose_tab_closed_is_fetched_from_the_current_tab() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.download_finished(id).unwrap();

        let retried = library.retry_download(id, &mut engine, closed, Some(tab(2)));

        assert_eq!(retried, Ok(true));
        assert_eq!(engine.fetched, [(tab(2), url("a.zip"))]);
    }

    #[test]
    fn a_download_of_an_earlier_run_is_fetched_from_the_current_tab() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.download_finished(id).unwrap();

        let retried = library.retry_download(id, &mut engine, open, Some(tab(2)));

        assert_eq!(retried, Ok(true));
        assert_eq!(engine.fetched, [(tab(2), url("a.zip"))]);
    }

    #[test]
    fn without_a_tab_to_fetch_from_nothing_changes() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.download_finished(id).unwrap();

        let retried = library.retry_download(id, &mut engine, open, None);

        assert_eq!(retried, Ok(false));
        assert_eq!(engine.fetched, []);
        assert_eq!(state(&library, id), Some(DownloadState::Finished));
    }

    #[test]
    fn a_running_download_is_not_retried() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = begin(&mut library, "a.zip", Jar::Main);

        assert_eq!(
            library.retry_download(id, &mut engine, open, Some(tab(1))),
            Err(AppError::Download(DownloadError::Running(id)))
        );
        assert!(library.downloads().get(id).is_some());
        assert_eq!(engine.fetched, []);
    }

    #[test]
    fn a_throwaway_retry_fetches_from_its_own_tab_and_forgets_without_a_save() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 3, "a.zip", throwaway());
        library.cancel_download(id, &mut engine).unwrap();

        let retried = library.retry_download(id, &mut engine, open, Some(tab(1)));

        assert_eq!(retried, Ok(true));
        assert_eq!(engine.fetched, [(tab(3), url("a.zip"))]);
        assert!(!library.is_session_only(id));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_throwaway_download_whose_tab_closed_is_not_fetched_elsewhere() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 3, "a.zip", throwaway());
        library.download_finished(id).unwrap();

        let retried = library.retry_download(id, &mut engine, closed, Some(tab(1)));

        assert_eq!(retried, Ok(false));
        assert_eq!(engine.fetched, []);
        assert!(library.is_session_only(id));
        assert_eq!(state(&library, id), Some(DownloadState::Finished));
    }

    #[test]
    fn the_old_fetch_reports_nothing_once_retried() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.download_failed(id, "reset".to_owned()).unwrap();
        library.retry_download(id, &mut engine, open, None).unwrap();

        let late = library.download_moved(failed(5), &mut engine);

        assert_eq!(late, Ok(DownloadMoved::Unlisted));
    }

    #[test]
    fn a_fetch_the_engine_refuses_is_reported_and_off_the_list() {
        let mut library = started();
        let mut engine = FakeDownloads {
            refuses: true,
            ..FakeDownloads::default()
        };
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.download_finished(id).unwrap();

        let retried = library.retry_download(id, &mut engine, open, None);

        assert_eq!(
            retried,
            Err(AppError::Engine(EngineError("no page".to_owned())))
        );
        assert_eq!(library.downloads().get(id), None);
    }

    #[test]
    fn a_failed_save_keeps_the_download_listed() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = begin(&mut library, "a.zip", Jar::Main);
        library.cancel_download(id, &mut engine).unwrap();
        fail_saves(&mut library);

        assert_eq!(
            library.retry_download(id, &mut engine, open, Some(tab(1))),
            Err(store_failure())
        );
        assert!(library.downloads().get(id).is_some());
        assert_eq!(engine.fetched, []);
    }
}

mod reported {
    use super::*;

    fn moved(library: &mut Subject, event: DownloadEvent) -> Result<DownloadMoved, AppError> {
        library.download_moved(event, &mut FakeDownloads::default())
    }

    #[test]
    fn a_started_download_is_listed_under_its_file_s_name() {
        let mut library = started();
        let mut engine = FakeDownloads::default();

        let moved = library.download_moved(started_event(5, 1, "a.zip", Jar::Main), &mut engine);

        let id = DownloadId::new(0);
        assert_eq!(
            moved,
            Ok(DownloadMoved::Started {
                id,
                file_name: "a.zip".to_owned()
            })
        );
        let download = library.downloads().get(id).cloned().unwrap();
        assert_eq!(download.url, url("a.zip"));
        assert_eq!(download.file_name, "a.zip");
        assert!(!library.is_session_only(id));
        assert_eq!(engine.cancelled, []);
    }

    #[test]
    fn a_throwaway_start_is_listed_for_the_session_only() {
        let mut library = started();

        let moved = moved(&mut library, started_event(5, 1, "a.zip", throwaway()));

        let Ok(DownloadMoved::Started { id, .. }) = moved else {
            panic!("{moved:?} is no start");
        };
        assert!(library.is_session_only(id));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_start_that_cannot_be_listed_is_cancelled() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        fail_saves(&mut library);

        let moved = library.download_moved(started_event(5, 1, "a.zip", Jar::Main), &mut engine);

        assert_eq!(moved, Err(store_failure()));
        assert_eq!(engine.cancelled, [DownloadToken::new(5)]);
        assert_eq!(library.downloads(), &Downloads::new());
    }

    #[test]
    fn progress_finish_and_failure_move_the_listed_download() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let first = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        let second = fetch(&mut library, &mut engine, 6, 1, "b.zip", Jar::Main);

        let progressed = moved(
            &mut library,
            DownloadEvent::Progress {
                token: DownloadToken::new(5),
                received: 3,
                total: Some(9),
            },
        );

        assert_eq!(progressed, Ok(DownloadMoved::Progressed(first)));
        assert_eq!(
            state(&library, first),
            Some(DownloadState::InProgress {
                received: 3,
                total: Some(9)
            })
        );

        let finished = moved(
            &mut library,
            DownloadEvent::Finished {
                token: DownloadToken::new(5),
            },
        );
        let failed = moved(&mut library, failed(6));

        assert_eq!(finished, Ok(DownloadMoved::Finished(first)));
        assert_eq!(failed, Ok(DownloadMoved::Failed(second)));
        assert_eq!(state(&library, first), Some(DownloadState::Finished));
        assert_eq!(
            state(&library, second),
            Some(DownloadState::Failed("reset".to_owned()))
        );
    }

    #[test]
    fn a_download_not_listed_moves_nothing() {
        let mut library = started();

        let late = [
            DownloadEvent::Progress {
                token: DownloadToken::new(5),
                received: 1,
                total: None,
            },
            DownloadEvent::Finished {
                token: DownloadToken::new(5),
            },
            failed(5),
        ]
        .map(|event| moved(&mut library, event));

        assert_eq!(late, [const { Ok(DownloadMoved::Unlisted) }; 3]);
    }

    #[test]
    fn the_engine_s_last_word_on_a_cancelled_download_moves_nothing() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.cancel_download(id, &mut engine).unwrap();

        let finished = moved(
            &mut library,
            DownloadEvent::Finished {
                token: DownloadToken::new(5),
            },
        );
        let progressed = moved(
            &mut library,
            DownloadEvent::Progress {
                token: DownloadToken::new(5),
                received: 1,
                total: None,
            },
        );

        assert_eq!(finished, Ok(DownloadMoved::Unlisted));
        assert_eq!(progressed, Ok(DownloadMoved::Unlisted));
        assert_eq!(state(&library, id), Some(DownloadState::Cancelled));
    }

    #[test]
    fn a_failed_save_at_the_end_is_reported() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        fail_saves(&mut library);

        let finished = moved(
            &mut library,
            DownloadEvent::Finished {
                token: DownloadToken::new(5),
            },
        );

        assert_eq!(finished, Err(store_failure()));
    }

    #[test]
    fn a_refused_download_lists_nothing() {
        let mut library = started();

        let refused = moved(
            &mut library,
            DownloadEvent::Refused {
                token: DownloadToken::new(5),
                reason: "no folder".to_owned(),
            },
        );

        assert_eq!(
            refused,
            Ok(DownloadMoved::Refused {
                reason: "no folder".to_owned()
            })
        );
        assert_eq!(library.downloads(), &Downloads::new());
    }

    #[test]
    fn a_saved_page_is_listed_finished() {
        let mut library = started();

        let saved = moved(
            &mut library,
            DownloadEvent::Saved {
                token: DownloadToken::new(5),
                tab: tab(1),
                jar: throwaway(),
                url: url("page"),
                file: PathBuf::from("/downloads/Page.pdf"),
            },
        );

        let id = DownloadId::new(0);
        assert_eq!(
            saved,
            Ok(DownloadMoved::Saved {
                id,
                file_name: "Page.pdf".to_owned()
            })
        );
        assert_eq!(state(&library, id), Some(DownloadState::Finished));
        assert!(library.is_session_only(id));
    }

    #[test]
    fn a_saved_page_that_cannot_be_listed_is_reported() {
        let mut library = started();
        fail_saves(&mut library);

        let saved = moved(
            &mut library,
            DownloadEvent::Saved {
                token: DownloadToken::new(5),
                tab: tab(1),
                jar: Jar::Main,
                url: url("page"),
                file: PathBuf::from("/downloads/Page.pdf"),
            },
        );

        assert_eq!(saved, Err(store_failure()));
    }

    #[test]
    fn cancelling_stops_the_engine_s_download() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);

        library.cancel_download(id, &mut engine).unwrap();

        assert_eq!(engine.cancelled, [DownloadToken::new(5)]);
        assert_eq!(state(&library, id), Some(DownloadState::Cancelled));
    }

    #[test]
    fn a_cancel_that_fails_leaves_the_engine_s_download_running() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.download_finished(id).unwrap();

        let cancelled = library.cancel_download(id, &mut engine);

        assert_eq!(cancelled, Err(AppError::Download(DownloadError::Ended(id))));
        assert_eq!(engine.cancelled, []);
    }

    #[test]
    fn a_cleared_download_is_no_longer_the_engine_s() {
        let mut library = started();
        let mut engine = FakeDownloads::default();
        let id = fetch(&mut library, &mut engine, 5, 1, "a.zip", Jar::Main);
        library.download_failed(id, "reset".to_owned()).unwrap();
        library.clear_finished_downloads().unwrap();

        let late = moved(&mut library, failed(5));

        assert_eq!(late, Ok(DownloadMoved::Unlisted));
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

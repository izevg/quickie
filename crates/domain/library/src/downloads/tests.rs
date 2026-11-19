use super::{
    CorruptDownloads, Download, DownloadError, DownloadId, DownloadState, Downloads,
    DownloadsSnapshot,
};
use crate::Timestamp;
use url::Url;

fn file(name: &str) -> Url {
    Url::parse(&format!("https://example.test/{name}")).unwrap()
}

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn start(downloads: &mut Downloads, name: &str) -> DownloadId {
    downloads.start(file(name), name.to_owned(), at(10))
}

fn state(downloads: &Downloads, id: DownloadId) -> DownloadState {
    downloads.get(id).unwrap().state.clone()
}

fn ids<'a>(downloads: impl IntoIterator<Item = &'a Download>) -> Vec<DownloadId> {
    downloads.into_iter().map(|download| download.id).collect()
}

/// One download in each state: running, finished, failed, cancelled, in that starting order.
fn one_of_each() -> (Downloads, [DownloadId; 4]) {
    let mut downloads = Downloads::new();
    let running = start(&mut downloads, "a.zip");
    let finished = start(&mut downloads, "b.zip");
    let failed = start(&mut downloads, "c.zip");
    let cancelled = start(&mut downloads, "d.zip");
    downloads.finish(finished).unwrap();
    downloads.fail(failed, "disk full".to_owned()).unwrap();
    downloads.cancel(cancelled).unwrap();
    (downloads, [running, finished, failed, cancelled])
}

mod moving {
    use super::*;

    #[test]
    fn a_new_download_is_in_progress_with_nothing_received() {
        let mut downloads = Downloads::new();

        let id = downloads.start(file("a.zip"), "a.zip".to_owned(), at(10));

        let expected = Download {
            id,
            url: file("a.zip"),
            file_name: "a.zip".to_owned(),
            state: DownloadState::InProgress {
                received: 0,
                total: None,
            },
            started_at: at(10),
        };
        assert_eq!(downloads.get(id), Some(&expected));
    }

    #[test]
    fn downloads_get_fresh_ids_in_starting_order() {
        let mut downloads = Downloads::new();

        let a = start(&mut downloads, "a.zip");
        let b = start(&mut downloads, "b.zip");

        assert_ne!(a, b);
        assert_eq!(ids(downloads.all()), vec![a, b]);
    }

    #[test]
    fn progress_records_received_and_total() {
        let mut downloads = Downloads::new();
        let id = start(&mut downloads, "a.zip");

        let result = downloads.progress(id, 512, Some(2048));

        assert_eq!(result, Ok(()));
        assert_eq!(
            state(&downloads, id),
            DownloadState::InProgress {
                received: 512,
                total: Some(2048)
            }
        );
    }

    #[test]
    fn each_ending_is_kept() {
        let (downloads, [_, finished, failed, cancelled]) = one_of_each();

        assert_eq!(state(&downloads, finished), DownloadState::Finished);
        assert_eq!(
            state(&downloads, failed),
            DownloadState::Failed("disk full".to_owned())
        );
        assert_eq!(state(&downloads, cancelled), DownloadState::Cancelled);
    }

    #[test]
    fn an_ended_download_takes_no_more_commands() {
        let (mut downloads, [_, finished, failed, cancelled]) = one_of_each();
        let before = downloads.clone();

        for id in [finished, failed, cancelled] {
            assert_eq!(
                downloads.progress(id, 1, None),
                Err(DownloadError::Ended(id))
            );
            assert_eq!(downloads.finish(id), Err(DownloadError::Ended(id)));
            assert_eq!(
                downloads.fail(id, String::new()),
                Err(DownloadError::Ended(id))
            );
            assert_eq!(downloads.cancel(id), Err(DownloadError::Ended(id)));
        }

        assert_eq!(downloads, before);
    }

    #[test]
    fn an_unknown_download_takes_no_commands() {
        let mut downloads = Downloads::new();
        let ghost = DownloadId::new(7);

        assert_eq!(
            downloads.progress(ghost, 1, None),
            Err(DownloadError::Unknown(ghost))
        );
        assert_eq!(downloads.finish(ghost), Err(DownloadError::Unknown(ghost)));
        assert_eq!(
            downloads.fail(ghost, String::new()),
            Err(DownloadError::Unknown(ghost))
        );
        assert_eq!(downloads.cancel(ghost), Err(DownloadError::Unknown(ghost)));
    }

    #[test]
    fn an_id_displays_its_number() {
        let id = DownloadId::new(3);

        assert_eq!(id.get(), 3);
        assert_eq!(id.to_string(), "download#3");
    }
}

mod clearing {
    use super::*;

    #[test]
    fn clear_finished_removes_every_ended_download() {
        let (mut downloads, [running, finished, failed, cancelled]) = one_of_each();

        let cleared = downloads.clear_finished();

        assert_eq!(ids(&cleared), vec![finished, failed, cancelled]);
        assert_eq!(ids(downloads.all()), vec![running]);
    }

    #[test]
    fn an_ended_download_can_be_removed_alone() {
        let (mut downloads, [running, finished, failed, cancelled]) = one_of_each();
        let expected = downloads.get(failed).cloned();

        let removed = downloads.remove(failed);

        assert_eq!(removed.ok(), expected);
        assert_eq!(ids(downloads.all()), vec![running, finished, cancelled]);
        assert_eq!(downloads.remove(finished).map(|d| d.id), Ok(finished));
        assert_eq!(downloads.remove(cancelled).map(|d| d.id), Ok(cancelled));
        assert_eq!(ids(downloads.all()), vec![running]);
    }

    #[test]
    fn a_running_or_unknown_download_cannot_be_removed() {
        let (mut downloads, [running, ..]) = one_of_each();
        let before = downloads.clone();
        let ghost = DownloadId::new(9);

        assert_eq!(
            downloads.remove(running),
            Err(DownloadError::Running(running))
        );
        assert_eq!(downloads.remove(ghost), Err(DownloadError::Unknown(ghost)));
        assert_eq!(downloads, before);
        assert_eq!(
            DownloadError::Running(running).to_string(),
            "download#0 is still in progress"
        );
    }

    #[test]
    fn clearing_keeps_ids_unique() {
        let (mut downloads, ids_before) = one_of_each();
        downloads.clear_finished();

        let next = start(&mut downloads, "e.zip");

        assert!(!ids_before.contains(&next));
    }
}

mod snapshot {
    use super::*;

    fn download(id: u64) -> Download {
        Download {
            id: DownloadId::new(id),
            url: file("a.zip"),
            file_name: "a.zip".to_owned(),
            state: DownloadState::Finished,
            started_at: at(1),
        }
    }

    #[test]
    fn restore_gives_back_the_same_list() {
        let (downloads, _) = one_of_each();

        let restored = Downloads::restore(downloads.snapshot());

        assert_eq!(restored, Ok(downloads));
    }

    #[test]
    fn the_snapshot_keeps_order_and_counter() {
        let (downloads, all) = one_of_each();

        let snapshot = downloads.snapshot();

        assert_eq!(ids(&snapshot.downloads), all.to_vec());
        assert_eq!(snapshot.next_id, 4);
    }

    #[test]
    fn an_id_just_below_the_counter_is_valid() {
        let snapshot = DownloadsSnapshot {
            downloads: vec![download(4)],
            next_id: 5,
        };

        let restored = Downloads::restore(snapshot.clone());

        assert_eq!(restored.map(|downloads| downloads.snapshot()), Ok(snapshot));
    }

    #[test]
    fn an_id_at_the_counter_is_corrupt() {
        let snapshot = DownloadsSnapshot {
            downloads: vec![download(5)],
            next_id: 5,
        };

        let result = Downloads::restore(snapshot);

        assert_eq!(
            result,
            Err(CorruptDownloads::IdNotAllocated(DownloadId::new(5)))
        );
    }

    #[test]
    fn an_id_twice_is_corrupt() {
        let snapshot = DownloadsSnapshot {
            downloads: vec![download(1), download(2), download(1)],
            next_id: 5,
        };

        let result = Downloads::restore(snapshot);

        assert_eq!(
            result,
            Err(CorruptDownloads::DuplicateDownload(DownloadId::new(1)))
        );
    }
}

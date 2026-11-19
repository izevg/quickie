use quickie_application::{LibraryRepo, LibrarySnapshot};
use quickie_domain_library::{
    Download, DownloadId, DownloadState, Downloads, DownloadsSnapshot, Timestamp,
};
use url::Url;

use crate::SqliteStore;

fn download(id: u64, name: &str, state: DownloadState) -> Download {
    Download {
        id: DownloadId::new(id),
        url: Url::parse(&format!("https://files.test/{name}?v=1")).unwrap(),
        file_name: name.to_owned(),
        state,
        started_at: Timestamp::from_millis(1_700_000_000_000 + id),
    }
}

/// One download in each state, a non-Latin name among them, with ids not in starting order.
fn snapshot() -> DownloadsSnapshot {
    DownloadsSnapshot {
        downloads: vec![
            download(
                4,
                "a.zip",
                DownloadState::InProgress {
                    received: 10,
                    total: Some(20),
                },
            ),
            download(
                1,
                "отчёт.pdf",
                DownloadState::InProgress {
                    received: 0,
                    total: None,
                },
            ),
            download(2, "b.dmg", DownloadState::Finished),
            download(3, "c.pkg", DownloadState::Failed("disk full".to_owned())),
            download(0, "d.txt", DownloadState::Cancelled),
        ],
        next_id: 7,
    }
}

#[test]
fn saved_downloads_load_back_equal_and_restorable() {
    let mut store = SqliteStore::in_memory().unwrap();

    store.save_downloads(&snapshot()).unwrap();
    let loaded = store.load_library().unwrap();

    assert_eq!(
        loaded,
        LibrarySnapshot {
            downloads: Some(snapshot()),
            ..LibrarySnapshot::default()
        }
    );
    assert!(Downloads::restore(loaded.downloads.unwrap()).is_ok());
}

#[test]
fn an_empty_list_keeps_its_counter() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save_downloads(&snapshot()).unwrap();
    let empty = DownloadsSnapshot {
        downloads: Vec::new(),
        next_id: 9,
    };

    store.save_downloads(&empty).unwrap();

    assert_eq!(store.load_library().unwrap().downloads, Some(empty));
}

#[test]
fn a_bad_state_is_reported_not_guessed() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save_downloads(&snapshot()).unwrap();
    store
        .db
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE downloads SET received = NULL WHERE id = 4;",
        )
        .unwrap();

    let loaded = store.load_library();

    assert!(
        loaded
            .unwrap_err()
            .0
            .contains("bad download state \"in_progress\"")
    );
}

#[test]
fn a_missing_counter_counts_from_zero() {
    let mut store = SqliteStore::in_memory().unwrap();
    store
        .save_downloads(&DownloadsSnapshot {
            downloads: Vec::new(),
            next_id: 3,
        })
        .unwrap();
    store
        .db
        .execute("DELETE FROM download_counter", [])
        .unwrap();

    let loaded = store.load_library().unwrap().downloads.unwrap();

    assert_eq!(loaded.next_id, 0);
}

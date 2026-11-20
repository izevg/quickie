use std::path::PathBuf;

use quickie_domain_browsing::{Jar, TabId};
use url::Url;

use super::{DownloadEvent, DownloadToken, ReplyToken};

#[test]
fn every_download_event_names_its_download() {
    let token = DownloadToken::new(7);
    let url = Url::parse("https://example.com/a.zip").unwrap();
    let file = PathBuf::from("/tmp/a.zip");
    let events = [
        DownloadEvent::Started {
            token,
            tab: TabId::new(1),
            jar: Jar::Main,
            url: url.clone(),
            file: file.clone(),
        },
        DownloadEvent::Progress {
            token,
            received: 1,
            total: None,
        },
        DownloadEvent::Finished { token },
        DownloadEvent::Failed {
            token,
            reason: "gone".to_owned(),
        },
        DownloadEvent::Refused {
            token,
            reason: "full".to_owned(),
        },
        DownloadEvent::Saved {
            token,
            tab: TabId::new(1),
            jar: Jar::Main,
            url,
            file,
        },
    ];

    for event in events {
        assert_eq!(event.token(), token);
    }
}

#[test]
fn tokens_minted_from_different_numbers_differ() {
    assert_ne!(DownloadToken::new(1), DownloadToken::new(2));
    assert_eq!(ReplyToken::new(3), ReplyToken::new(3));
    assert_ne!(ReplyToken::new(3), ReplyToken::new(4));
}

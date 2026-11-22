use std::path::PathBuf;

use quickie_domain_browsing::{Jar, Profile, TabId};
use url::Url;

use super::{DownloadEvent, DownloadToken, FindStep, PageContent, ReplyToken, Ticket};
use crate::EngineError;

/// Page content that remembers the finds it was asked for and can do nothing else.
#[derive(Default)]
struct Finds(Vec<(TabId, String, FindStep)>);

impl PageContent for Finds {
    fn find(&mut self, tab: TabId, query: &str, step: FindStep) -> Result<(), EngineError> {
        self.0.push((tab, query.to_owned(), step));
        Ok(())
    }

    fn read_text(&mut self, _tab: TabId, _max_chars: usize) -> Result<Ticket, EngineError> {
        Err(EngineError("no text".to_owned()))
    }

    fn link_targets(&mut self, _tab: TabId) -> Result<Ticket, EngineError> {
        Err(EngineError("no targets".to_owned()))
    }

    fn follow_link(&mut self, _tab: TabId, _: usize, _: bool) -> Result<Ticket, EngineError> {
        Err(EngineError("no targets".to_owned()))
    }
}

#[test]
fn clearing_a_find_is_a_find_of_nothing() {
    let tab = TabId::new(Profile::Main, 3);
    let mut finds = Finds::default();

    finds.clear_find(tab).unwrap();

    assert_eq!(finds.0, [(tab, String::new(), FindStep::Refine)]);
}

#[test]
fn an_engine_without_agent_links_rejects_the_request_immediately() {
    let tab = TabId::new(Profile::Main, 3);
    let finds = Finds::default();

    let request = finds.agent_links(tab, |_| panic!("unsupported requests have no completion"));

    assert!(request.is_err());
}

#[test]
fn every_download_event_names_its_download() {
    let token = DownloadToken::new(7);
    let url = Url::parse("https://example.com/a.zip").unwrap();
    let file = PathBuf::from("/tmp/a.zip");
    let events = [
        DownloadEvent::Started {
            token,
            tab: TabId::new(Profile::Main, 1),
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
            tab: TabId::new(Profile::Main, 1),
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
    assert_eq!(Ticket::new(5), Ticket::new(5));
    assert_ne!(Ticket::new(5), Ticket::new(6));
}

use std::time::Duration;

use quickie_domain_browsing::{Jar, Profile, TabId};
use quickie_store::{SqliteStore, WriteBehindStore};
use url::Url;

use super::ProfileLibrary;

#[test]
fn agent_commits_and_titles_cannot_change_browsing_history() {
    let profile = ProfileLibrary::default();
    let store = WriteBehindStore::spawn(SqliteStore::in_memory().unwrap(), Duration::ZERO).unwrap();
    profile.restore(store).unwrap();
    let public = Url::parse("https://example.com/public").unwrap();
    let agent_only = Url::parse("https://example.com/agent").unwrap();
    let browser = TabId::new(Profile::Main, 1);
    let agent = TabId::new(Profile::Agent, 1);
    assert!(
        profile
            .visit(browser, Jar::Main, &public, "Public")
            .unwrap()
    );

    assert!(
        !profile
            .visit(agent, Jar::Main, &agent_only, "Agent")
            .unwrap()
    );
    assert!(!profile.visit(agent, Jar::Main, &public, "Agent").unwrap());
    assert!(
        !profile
            .retitle(agent, Jar::Main, &public, "Agent title")
            .unwrap()
    );

    profile
        .with(|library| {
            let entries: Vec<_> = library.history().entries().collect();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].url, public);
            assert_eq!(entries[0].title, "Public");
            assert_eq!(entries[0].visit_count, 1);
        })
        .unwrap();
}

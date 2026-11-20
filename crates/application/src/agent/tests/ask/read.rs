use quickie_domain_agents::{PageScope, PageText};
use quickie_domain_browsing::{Focus, JarChoice, Profile, Session, Tab, TabId, WindowId};
use url::Url;

use crate::agent::PageReader;
use crate::{EngineError, FindStep, PageContent, Ticket};

/// Page content whose pages are the tabs it holds: each read of one gets the next ticket, and
/// any other tab has no page.
#[derive(Default)]
struct Pages {
    live: Vec<TabId>,
    /// The reads asked for, in order: which tab, and at most how many characters.
    reads: Vec<(TabId, usize)>,
    serial: u64,
}

impl Pages {
    fn of(live: &[TabId]) -> Self {
        Self {
            live: live.to_vec(),
            ..Self::default()
        }
    }
}

impl PageContent for Pages {
    fn find(&mut self, _tab: TabId, _query: &str, _step: FindStep) -> Result<(), EngineError> {
        Ok(())
    }

    fn read_text(&mut self, tab: TabId, max_chars: usize) -> Result<Ticket, EngineError> {
        if !self.live.contains(&tab) {
            return Err(EngineError(format!("no page for {tab}")));
        }
        self.reads.push((tab, max_chars));
        self.serial += 1;
        Ok(Ticket::new(self.serial))
    }

    fn link_targets(&mut self, _tab: TabId) -> Result<Ticket, EngineError> {
        Err(EngineError("no targets".to_owned()))
    }

    fn follow_link(&mut self, _tab: TabId, _: usize, _: bool) -> Result<Ticket, EngineError> {
        Err(EngineError("no targets".to_owned()))
    }
}

fn url(serial: usize) -> Url {
    Url::parse(&format!("https://example.com/{serial}")).unwrap()
}

/// A window with `count` tabs, titled "Page 1" on: the first is active.
fn window(count: usize) -> (Session, WindowId, Vec<TabId>) {
    let mut session = Session::new(Profile::Main);
    let window = session.open_window();
    let tabs = (1..=count)
        .map(|serial| {
            let tab = session
                .open(window, url(serial), JarChoice::Main, Focus::Background)
                .unwrap();
            session
                .navigated(tab, url(serial), format!("Page {serial}"))
                .unwrap();
            tab
        })
        .collect();
    (session, window, tabs)
}

fn ids(tabs: &[Tab]) -> Vec<TabId> {
    tabs.iter().map(|tab| tab.id).collect()
}

fn every_page(_: TabId) -> bool {
    true
}

#[test]
fn the_current_page_alone_is_the_active_tab() {
    let (mut session, window, tabs) = window(3);
    session.activate(tabs[1]).unwrap();

    let read = PageReader::tabs(&session, window, PageScope::Current, every_page);

    assert_eq!(ids(&read), [tabs[1]]);
    assert_eq!(read[0].title, "Page 2");
}

#[test]
fn open_tabs_are_the_active_first_then_the_rest_in_opening_order() {
    let (mut session, window, tabs) = window(3);
    session.activate(tabs[2]).unwrap();

    let read = PageReader::tabs(&session, window, PageScope::OpenTabs, every_page);

    assert_eq!(ids(&read), [tabs[2], tabs[0], tabs[1]]);
}

#[test]
fn a_tab_without_a_page_is_left_out_and_does_not_count_toward_the_most() {
    let (session, window, tabs) = window(10);
    let without = tabs[1];

    let read = PageReader::tabs(&session, window, PageScope::OpenTabs, |tab| tab != without);

    let mut want = vec![tabs[0]];
    want.extend(&tabs[2..9]);
    assert_eq!(ids(&read), want);
}

#[test]
fn an_active_tab_without_a_page_leaves_no_current_page() {
    let (session, window, tabs) = window(2);
    let active = tabs[0];

    let read = PageReader::tabs(&session, window, PageScope::Current, |tab| tab != active);

    assert_eq!(ids(&read), []);
}

#[test]
fn a_window_that_is_not_open_has_no_tabs_to_read() {
    let (session, _, _) = window(2);
    let closed = WindowId::new(Profile::Main, 99);

    let read = PageReader::tabs(&session, closed, PageScope::OpenTabs, every_page);

    assert_eq!(ids(&read), []);
}

#[test]
fn starting_asks_for_every_page_s_text_in_order_and_waits() {
    let (session, window, tabs) = window(2);
    let read = PageReader::tabs(&session, window, PageScope::OpenTabs, every_page);
    let mut pages = Pages::of(&tabs);

    let reader = PageReader::start(&mut pages, &read);

    assert_eq!(pages.reads, [(tabs[0], 200_000), (tabs[1], 200_000)]);
    assert!(!reader.is_complete());
}

#[test]
fn the_pages_are_complete_once_every_text_is_in() {
    let (session, window, tabs) = window(2);
    let read = PageReader::tabs(&session, window, PageScope::OpenTabs, every_page);
    let mut reader = PageReader::start(&mut Pages::of(&tabs), &read);

    let second = reader.text_read(Ticket::new(2), "two".to_owned());
    let between = reader.is_complete();
    let first = reader.text_read(Ticket::new(1), "one".to_owned());

    assert!(second && first);
    assert!(!between);
    assert!(reader.is_complete());
    assert_eq!(
        reader.into_pages(),
        [
            PageText::new(url(1), "Page 1", "one"),
            PageText::new(url(2), "Page 2", "two"),
        ]
    );
}

#[test]
fn a_text_for_another_request_or_read_twice_changes_nothing() {
    let (session, window, tabs) = window(1);
    let read = PageReader::tabs(&session, window, PageScope::Current, every_page);
    let mut reader = PageReader::start(&mut Pages::of(&tabs), &read);

    let other = reader.text_read(Ticket::new(9), "stale".to_owned());
    let pending = reader.is_complete();
    let first = reader.text_read(Ticket::new(1), "one".to_owned());
    let again = reader.text_read(Ticket::new(1), "again".to_owned());

    assert!(!other);
    assert!(!pending);
    assert!(first);
    assert!(!again);
    assert_eq!(
        reader.into_pages(),
        [PageText::new(url(1), "Page 1", "one")]
    );
}

#[test]
fn a_page_the_engine_cannot_read_reads_as_empty_at_once() {
    let (session, window, tabs) = window(2);
    let read = PageReader::tabs(&session, window, PageScope::OpenTabs, every_page);
    let mut pages = Pages::of(&tabs[1..]);

    let mut reader = PageReader::start(&mut pages, &read);
    let waiting = reader.is_complete();
    reader.text_read(Ticket::new(1), "two".to_owned());

    assert!(!waiting);
    assert!(reader.is_complete());
    assert_eq!(
        reader.into_pages(),
        [
            PageText::new(url(1), "Page 1", ""),
            PageText::new(url(2), "Page 2", "two"),
        ]
    );
}

#[test]
fn nothing_to_read_is_complete_at_once() {
    let reader = PageReader::start(&mut Pages::default(), &[]);

    assert!(reader.is_complete());
    assert_eq!(reader.into_pages(), []);
}

#[test]
fn a_text_still_on_its_way_reads_as_empty() {
    let (session, window, tabs) = window(1);
    let read = PageReader::tabs(&session, window, PageScope::Current, every_page);

    let reader = PageReader::start(&mut Pages::of(&tabs), &read);

    assert_eq!(reader.into_pages(), [PageText::new(url(1), "Page 1", "")]);
}

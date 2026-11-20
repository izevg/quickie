use quickie_domain_browsing::{Focus, Jar, JarChoice, JarId, Profile, Session, TabId};
use quickie_domain_settings::{
    CorruptZooms, Site, SiteError, SiteZooms, SiteZoomsSnapshot, ZoomLevel, ZoomStep,
};
use url::Url;

use super::Zoom;
use crate::{AppError, PageZoom, StoreError, ZoomRepo};

/// Keeps what was saved last, like a store would; counts saves, and fails on request.
#[derive(Debug, Default)]
struct FakeZoomRepo {
    saved: SiteZoomsSnapshot,
    saves: usize,
    load_fails: bool,
    save_fails: bool,
}

impl ZoomRepo for FakeZoomRepo {
    fn load_site_zooms(&self) -> Result<SiteZoomsSnapshot, StoreError> {
        if self.load_fails {
            return Err(StoreError("load".to_owned()));
        }
        Ok(self.saved.clone())
    }

    fn save_site_zooms(&mut self, zooms: SiteZoomsSnapshot) -> Result<(), StoreError> {
        if self.save_fails {
            return Err(StoreError("save".to_owned()));
        }
        self.saves += 1;
        self.saved = zooms;
        Ok(())
    }
}

const THROWAWAY: Jar = Jar::Throwaway(JarId::new(1));

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

fn level(percent: u16) -> ZoomLevel {
    ZoomLevel::from_percent(percent).unwrap()
}

fn fresh() -> Zoom<FakeZoomRepo> {
    Zoom::start(FakeZoomRepo::default()).unwrap()
}

#[test]
fn a_fresh_profile_shows_every_page_at_actual_size() {
    let zoom = fresh();

    assert_eq!(
        zoom.level(&url("https://example.com/"), Jar::Main),
        ZoomLevel::DEFAULT
    );
    assert_eq!(zoom.sites(), &SiteZooms::new());
    assert_eq!(zoom.corrupt_on_start(), None);
}

#[test]
fn zooming_a_page_saves_its_site_s_zoom() {
    let mut zoom = fresh();

    let zoomed = zoom.step(&url("https://example.com/a"), Jar::Main, ZoomStep::In);

    assert_eq!(zoomed, Ok(level(115)));
    assert_eq!(zoom.repo.saves, 1);
    assert_eq!(zoom.repo.saved.sites, [(site("example.com"), 115)]);
    assert_eq!(zoom.sites().snapshot(), zoom.repo.saved);
    assert_eq!(
        zoom.level(&url("https://example.com/b"), Jar::Main),
        level(115)
    );
}

#[test]
fn a_site_s_subdomains_share_its_zoom() {
    let mut zoom = fresh();

    zoom.step(&url("https://docs.example.com/"), Jar::Main, ZoomStep::Out)
        .unwrap();

    assert_eq!(
        zoom.level(&url("https://www.example.com/"), Jar::Main),
        level(85)
    );
    assert_eq!(
        zoom.level(&url("https://example.org/"), Jar::Main),
        ZoomLevel::DEFAULT
    );
}

#[test]
fn a_zoom_survives_a_restart() {
    let mut zoom = fresh();
    zoom.step(&url("https://example.com/"), Jar::Main, ZoomStep::In)
        .unwrap();
    zoom.step(&url("https://example.com/"), Jar::Main, ZoomStep::In)
        .unwrap();

    let restarted = Zoom::start(zoom.repo).unwrap();

    assert_eq!(
        restarted.level(&url("https://example.com/"), Jar::Main),
        level(125)
    );
}

#[test]
fn a_step_that_changes_nothing_saves_nothing() {
    let mut zoom = fresh();

    let reset = zoom.step(&url("https://example.com/"), Jar::Main, ZoomStep::Reset);

    assert_eq!(reset, Ok(ZoomLevel::DEFAULT));
    assert_eq!(zoom.repo.saves, 0);
}

#[test]
fn a_page_off_the_web_has_no_zoom_to_keep() {
    let mut zoom = fresh();
    let start = url("quickie://start/");

    let stepped = zoom.step(&start, Jar::Main, ZoomStep::In);

    assert_eq!(
        stepped,
        Err(AppError::NoSite(SiteError::NotWeb("quickie".to_owned())))
    );
    assert_eq!(zoom.level(&start, Jar::Main), ZoomLevel::DEFAULT);
    assert_eq!(zoom.repo.saves, 0);
}

#[test]
fn a_failed_save_changes_nothing() {
    let mut zoom = Zoom::start(FakeZoomRepo {
        save_fails: true,
        ..FakeZoomRepo::default()
    })
    .unwrap();

    let stepped = zoom.step(&url("https://example.com/"), Jar::Main, ZoomStep::In);

    assert_eq!(stepped, Err(AppError::Store(StoreError("save".to_owned()))));
    assert_eq!(
        zoom.level(&url("https://example.com/"), Jar::Main),
        ZoomLevel::DEFAULT
    );
}

#[test]
fn a_store_that_cannot_be_read_fails_the_start() {
    let started = Zoom::start(FakeZoomRepo {
        load_fails: true,
        ..FakeZoomRepo::default()
    });

    assert_eq!(
        started.unwrap_err(),
        AppError::Store(StoreError("load".to_owned()))
    );
}

#[test]
fn corrupt_saved_zooms_start_fresh_and_are_reported() {
    let zoom = Zoom::start(FakeZoomRepo {
        saved: SiteZoomsSnapshot {
            sites: vec![(site("example.com"), 110)],
        },
        ..FakeZoomRepo::default()
    })
    .unwrap();

    assert_eq!(
        zoom.corrupt_on_start(),
        Some(&CorruptZooms::NotAStep(site("example.com"), 110))
    );
    assert_eq!(
        zoom.level(&url("https://example.com/"), Jar::Main),
        ZoomLevel::DEFAULT
    );
}

#[test]
fn a_throwaway_tab_starts_at_actual_size_and_saves_nothing() {
    let mut zoom = fresh();
    let page = url("https://example.com/");
    zoom.step(&page, Jar::Main, ZoomStep::In).unwrap();

    assert_eq!(zoom.level(&page, THROWAWAY), ZoomLevel::DEFAULT);

    let zoomed = zoom.step(&page, THROWAWAY, ZoomStep::Out);

    assert_eq!(zoomed, Ok(level(85)));
    assert_eq!(zoom.level(&page, THROWAWAY), level(85));
    assert_eq!(zoom.level(&page, Jar::Main), level(115));
    assert_eq!(zoom.repo.saves, 1);
    assert_eq!(zoom.repo.saved.sites, [(site("example.com"), 115)]);
}

#[test]
fn each_throwaway_jar_zooms_on_its_own_and_forgets_with_its_jar() {
    let mut zoom = fresh();
    let page = url("https://example.com/");
    let other = Jar::Throwaway(JarId::new(2));
    zoom.step(&page, THROWAWAY, ZoomStep::In).unwrap();

    assert_eq!(zoom.level(&page, other), ZoomLevel::DEFAULT);

    zoom.forget_jar(JarId::new(1));

    assert_eq!(zoom.level(&page, THROWAWAY), ZoomLevel::DEFAULT);
}

/// Records the zoom each page was set to, in order.
#[derive(Debug, Default)]
struct FakePages {
    zoomed: Vec<(TabId, ZoomLevel)>,
}

impl PageZoom for FakePages {
    fn set_zoom(&mut self, tab: TabId, level: ZoomLevel) {
        self.zoomed.push((tab, level));
    }
}

/// Two windows: a main and a throwaway page of `example.com`, a page of `example.org`, and the
/// start page; the main jar zoomed `example.com` in, the throwaway jar out. Gives the tabs in
/// that order.
fn browsing_two_windows(zoom: &mut Zoom<FakeZoomRepo>) -> (Session, [TabId; 4]) {
    let mut session = Session::new(Profile::Main);
    let first = session.open_window();
    let second = session.open_window();
    let mut open = |window, at: &str, jar| {
        session
            .open(window, url(at), jar, Focus::Foreground)
            .unwrap()
    };
    let main = open(first, "https://example.com/a", JarChoice::Main);
    let other = open(first, "https://example.org/", JarChoice::Main);
    let throwaway = open(second, "https://docs.example.com/", JarChoice::NewThrowaway);
    let start = open(second, "quickie://start/", JarChoice::Main);
    let jar = session.tab(throwaway).unwrap().jar;
    zoom.step(&url("https://example.com/"), Jar::Main, ZoomStep::In)
        .unwrap();
    zoom.step(&url("https://example.com/"), jar, ZoomStep::Out)
        .unwrap();
    (session, [main, other, throwaway, start])
}

fn sorted(mut zoomed: Vec<(TabId, ZoomLevel)>) -> Vec<(TabId, ZoomLevel)> {
    zoomed.sort();
    zoomed
}

#[test]
fn showing_puts_every_open_page_at_its_site_s_zoom_in_its_jar() {
    let mut zoom = fresh();
    let (session, [main, other, throwaway, start]) = browsing_two_windows(&mut zoom);
    let mut pages = FakePages::default();

    zoom.show(&session, None, &mut pages);

    assert_eq!(
        sorted(pages.zoomed),
        sorted(vec![
            (main, level(115)),
            (other, ZoomLevel::DEFAULT),
            (throwaway, level(85)),
            (start, ZoomLevel::DEFAULT),
        ])
    );
}

#[test]
fn showing_only_some_tabs_leaves_the_others_alone() {
    let mut zoom = fresh();
    let (session, [main, _, throwaway, _]) = browsing_two_windows(&mut zoom);
    let mut pages = FakePages::default();

    zoom.show(&session, Some(&[throwaway, main]), &mut pages);

    assert_eq!(
        sorted(pages.zoomed),
        sorted(vec![(main, level(115)), (throwaway, level(85))])
    );
}

#[test]
fn showing_no_tabs_zooms_nothing() {
    let mut zoom = fresh();
    let (session, _) = browsing_two_windows(&mut zoom);
    let mut pages = FakePages::default();

    zoom.show(&session, Some(&[]), &mut pages);

    assert_eq!(pages.zoomed, []);
}

#[test]
fn forgetting_a_jar_keeps_the_others() {
    let mut zoom = fresh();
    let page = url("https://example.com/");
    let other = Jar::Throwaway(JarId::new(2));
    zoom.step(&page, other, ZoomStep::In).unwrap();

    zoom.forget_jar(JarId::new(1));

    assert_eq!(zoom.level(&page, other), level(115));
}

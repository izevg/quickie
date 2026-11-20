//! The suite against broken engines: each fault is a way an engine can go wrong, and the suite
//! must fail the checks it breaks, and only those. Some variations are not faults at all, and
//! the suite must pass them.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use quickie_domain_browsing::{Jar, JarId, TabId, WindowId};
use quickie_domain_settings::ZoomLevel;
use url::Url;

use super::{Memory, failed};
use crate::conformance::{
    Driven, Group, Harness, MemoryBlocker, MemoryEngine, MemoryHttp, MemoryPage, MemoryView,
    NotDriven, Outcome, PageSource, run,
};
use crate::{
    Answer, Ask, AskKind, BlockerRules, Capabilities, Dialog, DownloadEvent, DownloadToken,
    Downloads, Engine, EngineError, EngineEvent, EnginePort, FindStep, Followed, Fullscreen,
    LinkTarget, Navigation, NoExtensions, PageAsks, PageContent, PageRect, PageSurface, PageZoom,
    ReplyToken, Ticket,
};

/// How the engine under test differs from the reference.
#[derive(Clone, Copy)]
enum Fault {
    /// Every event drained goes through this first.
    Events(fn(EngineEvent<MemoryPage>) -> EngineEvent<MemoryPage>),
    /// The waker never runs.
    NoWake,
    /// Only the first waker set ever runs.
    FirstWakerOnly,
    /// The first waker set never runs; later ones do.
    FirstWakerIgnored,
    /// Creating a page wakes the shell inside the call.
    WakesInsideCalls,
    /// A closed tab's page goes on reporting: a rename, or an ask.
    Lingers { asks: bool },
    /// A download ends a turn after the one that reported its start: not a fault.
    LateEnd,
    /// A cancelled download still reports progress.
    ReportsCancelled,
    /// Saving a page as a PDF does nothing.
    SavesNoPdf,
    /// The harness cannot click pages.
    CannotClick,
    /// Every answer is taken, whether a page waits for it or not.
    TakesEveryAnswer,
    /// Clearing a find does nothing.
    KeepsFind,
    /// Reading a page's text ignores the most asked for.
    UncutText,
    /// Zooming does nothing.
    IgnoresZoom,
    /// A zoom set before its tab has a page waits for the page.
    ZoomsAhead,
    /// Every tab says a text field has focus, or every tab says none has.
    Focused(bool),
    /// Every tab says it loaded this far.
    Loading(Option<f64>),
    /// Capturing a page does nothing.
    NoPicture,
    /// The browser cannot take a page out of full screen.
    KeepsFullscreen,
    /// A page refused full screen says it went full screen all the same.
    FullscreenUnasked,
}

/// The reference engine with a fault.
struct Broken {
    engine: MemoryEngine,
    fault: Fault,
    wake: Option<Rc<dyn Fn()>>,
    /// How many wakers were set.
    wakers: usize,
    /// Tabs closed so far.
    closed: Vec<TabId>,
    /// Events held back for the next drain.
    later: Vec<EngineEvent<MemoryPage>>,
    /// Events held back until the engine turns again.
    next_turn: Vec<EngineEvent<MemoryPage>>,
    /// Zooms set for tabs without a page yet.
    ahead: Vec<(TabId, ZoomLevel)>,
}

impl Downloads for Broken {
    fn set_directory(&mut self, directory: PathBuf) {
        self.engine.set_directory(directory);
    }

    fn download(&mut self, tab: TabId, url: &Url) -> Result<(), EngineError> {
        self.engine.download(tab, url)
    }

    fn cancel(&mut self, token: DownloadToken) {
        if let Fault::ReportsCancelled = self.fault {
            self.later
                .push(EngineEvent::Download(DownloadEvent::Progress {
                    token,
                    received: 1,
                    total: None,
                }));
        }
        self.engine.cancel(token);
    }

    fn save_pdf(&mut self, tab: TabId) -> Result<(), EngineError> {
        match self.fault {
            Fault::SavesNoPdf => Ok(()),
            _ => self.engine.save_pdf(tab),
        }
    }
}

impl PageAsks for Broken {
    fn answer(&mut self, token: ReplyToken, answer: &Answer) -> bool {
        let waited = self.engine.answer(token, answer);
        waited || matches!(self.fault, Fault::TakesEveryAnswer)
    }

    fn exit_fullscreen(&mut self, tab: TabId) {
        if !matches!(self.fault, Fault::KeepsFullscreen) {
            self.engine.exit_fullscreen(tab);
        }
    }
}

impl PageZoom for Broken {
    fn set_zoom(&mut self, tab: TabId, level: ZoomLevel) {
        match self.fault {
            Fault::IgnoresZoom => {}
            Fault::ZoomsAhead => {
                self.ahead.push((tab, level));
                self.engine.set_zoom(tab, level);
            }
            _ => self.engine.set_zoom(tab, level),
        }
    }
}

impl EnginePort for Broken {
    type Page = MemoryPage;

    fn adopt(&mut self, page: MemoryPage, tab: TabId) -> Result<(), EngineError> {
        self.engine.adopt(page, tab)
    }

    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        let created = self.engine.create(tab, url, jar);
        if let (Fault::WakesInsideCalls, Some(wake)) = (self.fault, &self.wake) {
            wake();
        }
        for (_, level) in self.ahead.iter().filter(|(ahead, _)| *ahead == tab) {
            self.engine.set_zoom(tab, *level);
        }
        created
    }

    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError> {
        self.engine.show(window, tab)
    }

    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError> {
        self.engine.navigate(tab, navigation)
    }

    fn close(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.closed.push(tab);
        self.engine.close(tab)
    }

    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError> {
        self.engine.destroy_jar(jar)
    }
}

impl PageContent for Broken {
    fn find(&mut self, tab: TabId, query: &str, step: FindStep) -> Result<(), EngineError> {
        self.engine.find(tab, query, step)
    }

    fn clear_find(&mut self, tab: TabId) -> Result<(), EngineError> {
        match self.fault {
            Fault::KeepsFind => Ok(()),
            _ => self.engine.clear_find(tab),
        }
    }

    fn read_text(&mut self, tab: TabId, max_chars: usize) -> Result<Ticket, EngineError> {
        match self.fault {
            Fault::UncutText => self.engine.read_text(tab, usize::MAX),
            _ => self.engine.read_text(tab, max_chars),
        }
    }

    fn link_targets(&mut self, tab: TabId) -> Result<Ticket, EngineError> {
        self.engine.link_targets(tab)
    }

    fn follow_link(
        &mut self,
        tab: TabId,
        index: usize,
        background: bool,
    ) -> Result<Ticket, EngineError> {
        self.engine.follow_link(tab, index, background)
    }
}

impl PageSurface for Broken {
    type View = MemoryView;
    type Picture = Url;

    fn attach_host(&mut self, window: WindowId, host: &MemoryView) -> Result<(), EngineError> {
        self.engine.attach_host(window, host)
    }

    fn detach_host(&mut self, window: WindowId) {
        self.engine.detach_host(window);
    }

    fn view(&self, tab: TabId) -> Option<&MemoryView> {
        self.engine.view(tab)
    }

    fn input_focused(&self, tab: TabId) -> bool {
        match self.fault {
            Fault::Focused(focused) => focused,
            _ => self.engine.input_focused(tab),
        }
    }

    fn loading(&self, tab: TabId) -> Option<f64> {
        match self.fault {
            Fault::Loading(loading) => loading,
            _ => self.engine.loading(tab),
        }
    }

    fn capture(&mut self, tab: TabId) {
        match self.fault {
            Fault::NoPicture => {}
            _ => self.engine.capture(tab),
        }
    }

    fn picture(&self, tab: TabId) -> Option<Url> {
        self.engine.picture(tab)
    }

    fn print(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.engine.print(tab)
    }

    fn inspect(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.engine.inspect(tab)
    }
}

impl Engine for Broken {
    type Http = MemoryHttp;
    type Blocker = MemoryBlocker;
    type Extensions = NoExtensions<MemoryView>;

    const CAPS: Capabilities = <MemoryEngine as Engine>::CAPS;

    fn http(&self) -> MemoryHttp {
        self.engine.http()
    }

    fn web_extensions(&mut self) -> Option<NoExtensions<MemoryView>> {
        self.engine.web_extensions()
    }

    fn content_blocker(&mut self, directory: &Path) -> Result<MemoryBlocker, EngineError> {
        self.engine.content_blocker(directory)
    }

    fn set_waker(&mut self, wake: Box<dyn Fn()>) {
        let ignored = match self.fault {
            Fault::NoWake => true,
            Fault::FirstWakerOnly => self.wakers > 0,
            Fault::FirstWakerIgnored => self.wakers == 0,
            _ => false,
        };
        self.wakers += 1;
        if !ignored {
            let wake: Rc<dyn Fn()> = Rc::from(wake);
            let shared = Rc::clone(&wake);
            self.engine.set_waker(Box::new(move || shared()));
            self.wake = Some(wake);
        }
    }

    fn drain(&mut self, into: &mut Vec<EngineEvent<MemoryPage>>) {
        into.append(&mut self.later);
        let mut fresh = Vec::new();
        self.engine.drain(&mut fresh);
        for event in fresh {
            match (self.fault, event) {
                (Fault::Events(fault), event) => into.push(fault(event)),
                (Fault::LateEnd, event @ EngineEvent::Download(DownloadEvent::Finished { .. })) => {
                    self.next_turn.push(event);
                }
                (Fault::FullscreenUnasked, EngineEvent::Titled { tab, url, title })
                    if title == "fullscreen:denied" =>
                {
                    into.push(EngineEvent::Fullscreen {
                        tab,
                        state: Fullscreen::InWindow,
                    });
                    into.push(EngineEvent::Titled { tab, url, title });
                }
                (_, event) => into.push(event),
            }
        }
        if let Fault::Lingers { asks } = self.fault {
            into.extend(self.closed.iter().map(|&tab| lingering(tab, asks)));
        }
    }
}

/// The reference harness over a broken engine.
struct Faulty(Fault);

impl PageSource for Faulty {
    fn base(&self) -> Url {
        Memory.base()
    }
}

impl Harness for Faulty {
    type Engine = Broken;

    fn unsupported(&self, _group: Group) -> Option<&'static str> {
        None
    }

    fn engine(&mut self) -> Broken {
        Broken {
            engine: Memory.engine(),
            fault: self.0,
            wake: None,
            wakers: 0,
            closed: Vec::new(),
            later: Vec::new(),
            next_turn: Vec::new(),
            ahead: Vec::new(),
        }
    }

    fn pump(&mut self, engine: &mut Broken, done: &mut dyn FnMut(&mut Broken) -> bool) -> bool {
        (0..50).any(|_| {
            engine.later.append(&mut engine.next_turn);
            engine.engine.turn();
            done(engine)
        })
    }

    fn idle(&mut self, engine: &mut Broken) {
        Memory.idle(&mut engine.engine);
    }

    fn click(
        &mut self,
        engine: &mut Broken,
        tab: TabId,
        at: PageRect,
        command: bool,
    ) -> Driven<Result<(), String>> {
        match self.0 {
            Fault::CannotClick => Err(NotDriven("click")),
            _ => Memory.click(&mut engine.engine, tab, at, command),
        }
    }

    fn new_folder(&mut self) -> Driven<Result<PathBuf, String>> {
        Memory.new_folder()
    }

    fn downloaded(&self, engine: &Broken, file: &Path) -> Driven<Option<Vec<u8>>> {
        Memory.downloaded(&engine.engine, file)
    }

    fn block(&mut self, engine: &mut Broken, rules: &BlockerRules) -> Driven<Result<(), String>> {
        Memory.block(&mut engine.engine, rules)
    }

    fn crash(&mut self, engine: &mut Broken, tab: TabId) -> Driven<Result<(), String>> {
        Memory.crash(&mut engine.engine, tab)
    }
}

/// What a closed tab's page reports when it should not: a rename, or an ask.
fn lingering(tab: TabId, asks: bool) -> EngineEvent<MemoryPage> {
    if asks {
        EngineEvent::Asked(Ask {
            token: ReplyToken::new(0),
            tab,
            origin: Memory.base(),
            kind: AskKind::Dialog(Dialog::Alert {
                message: String::new(),
            }),
        })
    } else {
        EngineEvent::Titled {
            tab,
            url: Memory.base(),
            title: "late".to_owned(),
        }
    }
}

/// Rewrites every link target's frame with `reshape`.
fn reshape_targets(
    event: EngineEvent<MemoryPage>,
    reshape: fn(&mut PageRect),
) -> EngineEvent<MemoryPage> {
    match event {
        EngineEvent::Targets {
            tab,
            ticket,
            mut targets,
        } => {
            for LinkTarget { frame, .. } in &mut targets {
                reshape(frame);
            }
            EngineEvent::Targets {
                tab,
                ticket,
                targets,
            }
        }
        other => other,
    }
}

#[test]
fn a_waker_that_never_runs_fails_the_events_checks() {
    let report = run(&mut Faulty(Fault::NoWake));

    assert!(!report.passed());
    assert_eq!(
        failed(&report),
        [
            "the_waker_runs_when_events_wait",
            "nothing_is_delivered_inside_an_engine_call"
        ]
    );
}

#[test]
fn a_waker_set_twice_must_run_the_second() {
    let report = run(&mut Faulty(Fault::FirstWakerOnly));

    assert_eq!(
        failed(&report),
        ["nothing_is_delivered_inside_an_engine_call"]
    );
}

#[test]
fn a_first_waker_that_never_runs_fails_even_when_a_second_does() {
    let report = run(&mut Faulty(Fault::FirstWakerIgnored));

    assert_eq!(
        failed(&report),
        [
            "the_waker_runs_when_events_wait",
            "nothing_is_delivered_inside_an_engine_call"
        ]
    );
}

#[test]
fn waking_inside_a_call_fails() {
    let report = run(&mut Faulty(Fault::WakesInsideCalls));

    assert_eq!(
        failed(&report),
        ["nothing_is_delivered_inside_an_engine_call"]
    );
}

#[test]
fn a_closed_page_that_still_renames_itself_fails() {
    let report = run(&mut Faulty(Fault::Lingers { asks: false }));

    assert_eq!(
        failed(&report),
        [
            "a_closed_page_reports_nothing_more",
            "nothing_is_delivered_inside_an_engine_call",
            "closing_a_tab_declines_what_its_page_asked"
        ]
    );
}

#[test]
fn a_closed_page_that_still_asks_fails() {
    let report = run(&mut Faulty(Fault::Lingers { asks: true }));

    assert_eq!(
        failed(&report),
        [
            "a_closed_page_reports_nothing_more",
            "closing_a_tab_declines_what_its_page_asked"
        ]
    );
}

#[test]
fn a_download_may_end_in_a_later_drain_than_it_started() {
    let report = run(&mut Faulty(Fault::LateEnd));

    assert!(report.passed(), "{report}");
}

#[test]
fn a_cancelled_download_that_still_reports_fails() {
    let report = run(&mut Faulty(Fault::ReportsCancelled));

    assert_eq!(failed(&report), ["a_cancelled_download_leaves_no_file"]);
}

#[test]
fn a_pdf_that_is_never_saved_fails() {
    let report = run(&mut Faulty(Fault::SavesNoPdf));

    assert_eq!(
        failed(&report),
        ["a_page_saved_as_a_pdf_is_a_file_in_the_folder"]
    );
}

#[test]
fn a_download_that_fails_instead_of_finishing_fails() {
    let report = run(&mut Faulty(Fault::Events(|event| match event {
        EngineEvent::Download(DownloadEvent::Finished { token }) => {
            EngineEvent::Download(DownloadEvent::Failed {
                token,
                reason: "broken".to_owned(),
            })
        }
        other => other,
    })));

    assert_eq!(
        failed(&report),
        [
            "an_attachment_downloads_into_the_folder",
            "a_download_asked_for_downloads_the_same_way"
        ]
    );
}

/// The checks that click a page where a target says a control is.
const CLICKS: [&str; 5] = [
    "a_command_click_opens_in_the_background",
    "a_clicked_page_fills_its_view_in_full_screen",
    "the_browser_takes_a_page_out_of_full_screen",
    "a_page_leaves_full_screen_by_itself",
    "a_page_that_navigates_leaves_full_screen",
];

/// What targets without an area fail: the targets check, and every click where a target
/// claims to be.
const MISPLACED: [&str; 6] = [
    CLICKS[0],
    CLICKS[1],
    CLICKS[2],
    CLICKS[3],
    CLICKS[4],
    "the_targets_are_the_links_and_controls_in_order",
];

#[test]
fn targets_without_a_width_fail() {
    let report = run(&mut Faulty(Fault::Events(|event| {
        reshape_targets(event, |frame| frame.width = 0.0)
    })));

    assert_eq!(failed(&report), MISPLACED);
}

#[test]
fn targets_without_a_height_fail() {
    let report = run(&mut Faulty(Fault::Events(|event| {
        reshape_targets(event, |frame| frame.height = 0.0)
    })));

    assert_eq!(failed(&report), MISPLACED);
}

/// Targets on one line fail the targets check, and a click on any target but the first; the
/// full-screen pages have one button, so their clicks still land.
#[test]
fn targets_on_one_line_fail() {
    let report = run(&mut Faulty(Fault::Events(|event| {
        reshape_targets(event, |frame| frame.y = 0.0)
    })));

    assert_eq!(
        failed(&report),
        [
            "a_command_click_opens_in_the_background",
            "the_targets_are_the_links_and_controls_in_order"
        ]
    );
}

#[test]
fn a_find_that_cannot_be_cleared_fails() {
    let report = run(&mut Faulty(Fault::KeepsFind));

    assert_eq!(failed(&report), ["clearing_a_find_finds_nothing"]);
}

#[test]
fn text_that_is_not_the_page_s_fails() {
    let report = run(&mut Faulty(Fault::Events(|event| match event {
        EngineEvent::Text { tab, ticket, .. } => EngineEvent::Text {
            tab,
            ticket,
            text: String::new(),
        },
        other => other,
    })));

    assert_eq!(failed(&report), ["the_text_is_what_the_page_shows"]);
}

#[test]
fn text_read_whole_when_asked_for_less_fails() {
    let report = run(&mut Faulty(Fault::UncutText));

    assert_eq!(failed(&report), ["the_text_is_cut_to_the_most_asked_for"]);
}

#[test]
fn a_followed_link_left_to_the_page_fails() {
    let report = run(&mut Faulty(Fault::Events(|event| match event {
        EngineEvent::Followed { tab, ticket, .. } => EngineEvent::Followed {
            tab,
            ticket,
            followed: Followed::Done,
        },
        other => other,
    })));

    assert_eq!(failed(&report), ["following_a_link_names_where_it_goes"]);
}

#[test]
fn a_followed_button_that_names_a_link_fails() {
    let report = run(&mut Faulty(Fault::Events(|event| match event {
        EngineEvent::Followed {
            tab,
            ticket,
            followed: Followed::Done,
        } => EngineEvent::Followed {
            tab,
            ticket,
            followed: Followed::NewTab(Memory.base()),
        },
        other => other,
    })));

    assert_eq!(failed(&report), ["following_a_button_clicks_it"]);
}

#[test]
fn a_zoom_that_does_nothing_fails() {
    let report = run(&mut Faulty(Fault::IgnoresZoom));

    assert_eq!(
        failed(&report),
        ["a_zoomed_page_lays_out_larger_across_navigations"]
    );
}

#[test]
fn a_zoom_that_waits_for_its_page_fails() {
    let report = run(&mut Faulty(Fault::ZoomsAhead));

    assert_eq!(
        failed(&report),
        ["zooming_a_tab_without_a_page_changes_nothing"]
    );
}

#[test]
fn a_feature_the_harness_cannot_drive_fails_its_group_and_says_so() {
    let report = run(&mut Faulty(Fault::CannotClick));

    let clicked = report
        .verdicts()
        .iter()
        .find(|verdict| verdict.case == "a_command_click_opens_in_the_background")
        .map(|verdict| verdict.outcome.clone());
    assert_eq!(
        clicked,
        Some(Outcome::Failed(
            "the harness cannot drive `click`: drive it or declare the group unsupported"
                .to_owned()
        ))
    );
    assert_eq!(failed(&report), CLICKS);
}

#[test]
fn an_answer_nobody_waits_for_that_is_taken_fails() {
    let report = run(&mut Faulty(Fault::TakesEveryAnswer));

    assert_eq!(
        failed(&report),
        [
            "an_alert_waits_for_its_answer",
            "closing_a_tab_declines_what_its_page_asked"
        ]
    );
}

#[test]
fn a_page_whose_field_takes_the_keys_unsaid_fails() {
    let report = run(&mut Faulty(Fault::Focused(false)));

    assert_eq!(
        failed(&report),
        [
            "a_page_says_when_a_text_field_has_focus",
            "a_frame_added_while_a_field_has_focus_keeps_it_focused"
        ]
    );
}

#[test]
fn a_tab_that_always_takes_the_keys_fails() {
    let report = run(&mut Faulty(Fault::Focused(true)));

    assert_eq!(
        failed(&report),
        [
            "a_page_has_a_view_until_it_closes",
            "a_page_says_when_a_text_field_has_focus"
        ]
    );
}

#[test]
fn a_page_that_never_says_it_loads_fails() {
    let report = run(&mut Faulty(Fault::Loading(None)));

    assert_eq!(failed(&report), ["a_page_is_loading_until_it_has_loaded"]);
}

#[test]
fn a_page_that_never_stops_loading_fails() {
    let report = run(&mut Faulty(Fault::Loading(Some(0.5))));

    assert_eq!(
        failed(&report),
        [
            "a_page_has_a_view_until_it_closes",
            "a_page_is_loading_until_it_has_loaded"
        ]
    );
}

#[test]
fn a_page_loaded_past_the_end_fails() {
    let report = run(&mut Faulty(Fault::Loading(Some(1.5))));

    assert_eq!(
        failed(&report),
        [
            "a_page_has_a_view_until_it_closes",
            "a_page_is_loading_until_it_has_loaded"
        ]
    );
}

#[test]
fn a_page_never_pictured_fails() {
    let report = run(&mut Faulty(Fault::NoPicture));

    assert_eq!(failed(&report), ["a_captured_page_has_a_picture"]);
}

#[test]
fn a_page_the_browser_cannot_take_out_of_full_screen_fails() {
    let report = run(&mut Faulty(Fault::KeepsFullscreen));

    assert_eq!(
        failed(&report),
        ["the_browser_takes_a_page_out_of_full_screen"]
    );
}

#[test]
fn a_page_that_goes_full_screen_unasked_fails() {
    let report = run(&mut Faulty(Fault::FullscreenUnasked));

    assert_eq!(failed(&report), ["a_page_goes_full_screen_only_on_a_click"]);
}

#[test]
fn a_page_that_never_says_it_left_full_screen_fails() {
    let report = run(&mut Faulty(Fault::Events(|event| match event {
        EngineEvent::Fullscreen {
            tab,
            state: Fullscreen::Left,
        } => EngineEvent::Pictured { tab },
        event => event,
    })));

    assert_eq!(
        failed(&report),
        [
            "the_browser_takes_a_page_out_of_full_screen",
            "a_page_leaves_full_screen_by_itself",
            "a_page_that_navigates_leaves_full_screen"
        ]
    );
}

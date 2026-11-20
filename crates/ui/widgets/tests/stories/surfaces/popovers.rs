//! The popovers group: downloads and the site's shields.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_domain_browsing::Timestamp;
use quickie_domain_library::{Download, DownloadId, DownloadState};
use quickie_domain_shields::{FilterListId, SitePolicy};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{
    DownloadsPopover, DownloadsRequest, ShieldsPopover, ShieldsRequest, ShieldsSummary, SiteToggle,
};

use crate::fixtures::{self, NOW};

const fn downloads(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "popovers",
        widget: "downloads-popover",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

const fn shields(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "popovers",
        widget: "shields-popover",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

pub const STORIES: &[Story] = &[
    downloads("mixed", LIGHT_DARK, downloads_mixed),
    downloads("empty", LIGHT, downloads_empty),
    downloads("keys", LIGHT, downloads_keys),
    shields("protected", LIGHT_DARK, shields_protected),
    shields("off-the-web", LIGHT, shields_off_the_web),
    shields("lists-off", LIGHT, shields_lists_off),
    shields("reload", LIGHT, shields_reload),
];

fn download(id: u64, name: &str, state: DownloadState) -> Download {
    Download {
        id: DownloadId::new(id),
        url: fixtures::url(&format!("https://downloads.example.com/{name}")),
        file_name: name.to_owned(),
        state,
        started_at: Timestamp::from_millis(NOW.as_millis() - id * 60_000),
    }
}

/// One of each: running with and without a size, finished, failed, cancelled; a Russian name.
fn mixed() -> Vec<Download> {
    vec![
        download(1, "quickie-0.1.0.dmg", DownloadState::Finished),
        download(
            2,
            "Отчёт за квартал.pdf",
            DownloadState::Failed("The server stopped answering".to_owned()),
        ),
        download(3, "dataset.zip", DownloadState::Cancelled),
        download(
            4,
            "rust-1.95.0-aarch64-apple-darwin.pkg",
            DownloadState::InProgress {
                received: 48_300_000,
                total: Some(312_000_000),
            },
        ),
        download(
            5,
            "stream.log",
            DownloadState::InProgress {
                received: 1_250_000,
                total: None,
            },
        ),
    ]
}

fn downloads_show(scene: &mut Scene, list: &[Download]) -> Rc<DownloadsPopover> {
    let popover = Rc::new(DownloadsPopover::new(scene.mtm()));
    popover.show(scene.host(), list, Trigger::Keyboard);
    scene.keep(Rc::clone(&popover));
    popover
}

fn downloads_mixed(scene: &mut Scene) {
    let popover = downloads_show(scene, &mixed());
    scene.check_eq(popover.details().len(), 5, "every download is listed");
}

fn downloads_empty(scene: &mut Scene) {
    let popover = downloads_show(scene, &[]);
    scene.check_eq(popover.selected(), None, "nothing to select");
}

/// `↓` moves down the list and stops at the end; `⏎` asks what the row's state allows.
fn downloads_keys(scene: &mut Scene) {
    let list = mixed();
    let popover = downloads_show(scene, &list);
    let asked = Rc::new(RefCell::new(None));
    let shown = Rc::clone(&popover);
    let answer = Rc::clone(&asked);
    scene.route(move |stroke| {
        match stroke.key {
            Key::Down => shown.move_selection(1),
            Key::Up => shown.move_selection(-1),
            Key::Return => *answer.borrow_mut() = shown.enter(stroke.modifiers.command),
            _ => return false,
        }
        true
    });
    let first = popover.selected();
    scene.press(Key::Return);
    let first_asked = *asked.borrow();
    for _ in 0..10 {
        scene.press(Key::Down);
    }
    let bottom = popover.selected();
    scene.check(first != bottom, "↓ moves the selection");
    scene.check(
        bottom.is_some_and(|id| list.iter().any(|download| download.id == id)),
        "the selection stays on a listed download",
    );
    scene.check(
        matches!(
            first_asked,
            Some(DownloadsRequest::Open(_) | DownloadsRequest::Retry(_)) | None
        ),
        format!("⏎ on the newest download asks {first_asked:?}"),
    );
}

fn summary(site: Option<&str>, lists: bool) -> ShieldsSummary {
    let fetched = |hours: u64| Some(Timestamp::from_millis(NOW.as_millis() - hours * 3_600_000));
    ShieldsSummary {
        site: site.map(str::to_owned),
        policy: site.map(|_| SitePolicy::DEFAULT),
        blocked: site.map(|_| 27),
        lists: if lists {
            vec![
                (FilterListId::EasyList, fetched(3)),
                (FilterListId::EasyPrivacy, fetched(30)),
                (FilterListId::UblockFilters, None),
            ]
        } else {
            Vec::new()
        },
        now: NOW,
        reload_offered: false,
    }
}

fn shields_show(scene: &mut Scene, summary: ShieldsSummary) -> Rc<ShieldsPopover> {
    let popover = Rc::new(ShieldsPopover::new(scene.mtm()));
    popover.show(scene.host(), summary, Trigger::Keyboard);
    scene.keep(Rc::clone(&popover));
    popover
}

/// A click on a switch asks the browser to set it, and the switch follows.
fn shields_protected(scene: &mut Scene) {
    let popover = shields_show(scene, summary(Some("nytimes.com"), true));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    popover.on_request(move |request| log.borrow_mut().push(request));
    scene.check(
        popover.is_on(SiteToggle::JavaScript),
        "JavaScript starts on",
    );
    let Some(switch) = scene.view_labelled("JavaScript") else {
        scene.check(false, "the JavaScript switch is labelled");
        return;
    };
    scene.click(&switch);
    scene.check(
        !popover.is_on(SiteToggle::JavaScript),
        "the switch follows the click",
    );
    scene.check_eq(
        asked.borrow().clone(),
        vec![ShieldsRequest::Set(SiteToggle::JavaScript, false)],
        "the click asks to set it",
    );
    scene.click(&switch);
    scene.check(
        popover.is_on(SiteToggle::JavaScript),
        "a second click turns it back",
    );
}

fn shields_off_the_web(scene: &mut Scene) {
    shields_show(scene, summary(None, true));
}

fn shields_lists_off(scene: &mut Scene) {
    shields_show(scene, summary(Some("lenta.ru"), false));
}

fn shields_reload(scene: &mut Scene) {
    let popover = shields_show(
        scene,
        ShieldsSummary {
            reload_offered: true,
            policy: Some(SitePolicy {
                shields: false,
                ..SitePolicy::DEFAULT
            }),
            ..summary(Some("example.com"), true)
        },
    );
    scene.check(popover.reload_offered(), "⏎ reloads");
}

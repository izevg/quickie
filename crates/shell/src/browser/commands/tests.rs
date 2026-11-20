use quickie_domain_browsing::{Direction, PinnedSlot};
use quickie_domain_commands::{Catalog, CommandId, KeyStroke, Modifiers};
use quickie_domain_settings::ZoomStep;

use super::{Action, holding};

fn of(id: &str) -> Option<Action> {
    Action::of(&CommandId::try_from(id).unwrap())
}

fn slot(n: u8) -> PinnedSlot {
    PinnedSlot::try_from(n).unwrap()
}

#[test]
fn the_commands_this_shell_does_have_actions() {
    let cases = [
        ("quickie.palette.openCommands", Action::OpenCommands),
        ("quickie.palette.goto", Action::GoTo),
        ("quickie.address.open", Action::OpenAddress),
        ("quickie.tab.new", Action::NewTab),
        ("quickie.tab.newThrowaway", Action::NewThrowawayTab),
        ("quickie.tab.close", Action::CloseTab),
        ("quickie.tab.reopenClosed", Action::ReopenClosed),
        ("quickie.tab.nextRecent", Action::Cycle(Direction::Forward)),
        (
            "quickie.tab.previousRecent",
            Action::Cycle(Direction::Backward),
        ),
        ("quickie.tab.openInThrowawayJar", Action::OpenInThrowawayJar),
        ("quickie.tab.reload", Action::Reload),
        ("quickie.tab.hardReload", Action::HardReload),
        ("quickie.navigation.back", Action::Back),
        ("quickie.navigation.forward", Action::Forward),
        ("quickie.page.copyUrl", Action::CopyUrl),
        ("quickie.page.copyAsMarkdownLink", Action::CopyMarkdownLink),
        ("quickie.page.copyTitle", Action::CopyTitle),
        ("quickie.page.bookmark", Action::Bookmark),
        ("quickie.page.find", Action::Find),
        ("quickie.page.findNext", Action::FindNext),
        ("quickie.page.findPrevious", Action::FindPrevious),
        ("quickie.page.zoomIn", Action::Zoom(ZoomStep::In)),
        ("quickie.page.zoomOut", Action::Zoom(ZoomStep::Out)),
        ("quickie.page.zoomReset", Action::Zoom(ZoomStep::Reset)),
        ("quickie.page.print", Action::Print),
        ("quickie.page.savePdf", Action::SavePdf),
        ("quickie.shields.toggleSite", Action::ToggleShields),
        ("quickie.shields.enableLists", Action::EnableLists),
        ("quickie.shields.refreshLists", Action::RefreshLists),
        ("quickie.tab.overview", Action::Overview),
        ("quickie.hints.toggle", Action::Hints { background: false }),
        (
            "quickie.hints.openInBackground",
            Action::Hints { background: true },
        ),
        ("quickie.downloads.show", Action::ShowDownloads),
        ("quickie.shields.open", Action::ShowShields),
        ("quickie.window.new", Action::NewWindow),
        ("quickie.window.close", Action::CloseWindow),
        ("quickie.app.setDefaultBrowser", Action::SetDefaultBrowser),
        ("quickie.app.revealProfile", Action::RevealProfile),
        ("quickie.agent.ask", Action::AskAgent),
        ("quickie.agent.stop", Action::StopAgent),
        ("quickie.agent.showAnswer", Action::ShowAgentAnswer),
        ("quickie.agent.settings", Action::ShowAgentSettings),
        ("quickie.app.settings", Action::ShowSettings),
        ("quickie.debug.keys", Action::DebugKeys),
    ];
    for (id, action) in cases {
        assert_eq!(of(id), Some(action), "{id}");
    }
}

#[test]
fn every_developer_tools_command_points_to_web_inspector() {
    let catalog = Catalog::builtin();
    let devtools: Vec<_> = catalog
        .iter()
        .filter(|command| command.id.as_str().starts_with("quickie.devtools."))
        .collect();

    assert_eq!(devtools.len(), 9);
    for command in devtools {
        assert_eq!(
            Action::of(&command.id),
            Some(Action::DevTools),
            "{}",
            command.id
        );
    }
}

#[test]
fn only_leaving_the_page_passes_a_sheet() {
    for id in [
        "quickie.tab.close",
        "quickie.tab.new",
        "quickie.tab.newThrowaway",
        "quickie.tab.reopenClosed",
        "quickie.tab.nextRecent",
        "quickie.tab.goToSlot3",
        "quickie.window.new",
        "quickie.window.close",
    ] {
        assert!(of(id).is_some_and(Action::passes_sheet), "{id}");
    }
    for id in [
        "quickie.palette.openCommands",
        "quickie.address.open",
        "quickie.page.find",
        "quickie.tab.reload",
        "quickie.navigation.back",
        "quickie.tab.overview",
        "quickie.hints.toggle",
        "quickie.tab.pinToSlot3",
        "quickie.page.print",
        "quickie.page.zoomIn",
    ] {
        assert!(of(id).is_some_and(|action| !action.passes_sheet()), "{id}");
    }
}

#[test]
fn every_slot_goes_and_pins() {
    for n in 1..=9 {
        assert_eq!(
            of(&format!("quickie.tab.goToSlot{n}")),
            Some(Action::GoToSlot(slot(n)))
        );
        assert_eq!(
            of(&format!("quickie.tab.pinToSlot{n}")),
            Some(Action::PinToSlot(slot(n)))
        );
    }
    assert_eq!(of("quickie.tab.goToSlot0"), None);
    assert_eq!(of("quickie.tab.goToSlot10"), None);
}

#[test]
fn commands_not_built_yet_have_no_action() {
    assert_eq!(of("quickie.reader.toggle"), None);
}

#[test]
fn every_action_names_a_command_of_the_catalog() {
    let catalog = Catalog::builtin();
    let built = catalog
        .iter()
        .filter(|command| Action::of(&command.id).is_some())
        .count();

    // 44 named commands, nine slots to go to and nine to pin, and the nine developer tools.
    assert_eq!(built, 44 + 9 + 9 + 9);
}

#[test]
fn a_cycle_is_held_by_its_modifiers_but_shift() {
    let backward = KeyStroke::try_from("ctrl+shift+tab").unwrap();
    let bare = KeyStroke::try_from("tab").unwrap();

    assert_eq!(holding(Some(backward)), Modifiers::CONTROL);
    assert_eq!(holding(Some(bare)), Modifiers::NONE);
    assert_eq!(holding(None), Modifiers::NONE);
}

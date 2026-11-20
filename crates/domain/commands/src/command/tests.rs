use quickie_domain_kernel::CommandId;

use super::builtin::COMMANDS;
use super::{Catalog, Category};

fn id(text: &str) -> CommandId {
    CommandId::try_from(text).unwrap()
}

#[test]
fn every_builtin_id_parses() {
    let catalog = Catalog::builtin();

    assert_eq!(catalog.iter().count(), COMMANDS.len());
}

#[test]
fn the_catalog_keeps_palette_order() {
    let catalog = Catalog::builtin();

    let ids: Vec<&str> = catalog.iter().map(|command| command.id.as_str()).collect();

    let expected: Vec<&str> = COMMANDS.iter().map(|&(id, _, _)| id).collect();
    assert_eq!(ids, expected);
}

#[test]
fn a_command_is_found_by_its_id() {
    let catalog = Catalog::builtin();

    let command = catalog.get(&id("quickie.tab.reopenClosed")).unwrap();

    assert_eq!(command.title, "Reopen Closed Tab");
    assert_eq!(command.category, Category::Tabs);
}

#[test]
fn an_unknown_id_is_not_in_the_catalog() {
    let catalog = Catalog::builtin();

    let unknown = id("quickie.tab.explode");

    assert!(catalog.get(&unknown).is_none());
    assert!(!catalog.contains(&unknown));
}

#[test]
fn a_known_id_is_in_the_catalog() {
    let catalog = Catalog::builtin();

    let known = catalog.contains(&id("quickie.palette.openCommands"));

    assert!(known);
}

#[test]
fn the_palette_only_commands_from_the_docs_are_built_in() {
    let catalog = Catalog::builtin();
    let ids = [
        "quickie.page.copyUrl",
        "quickie.page.copyAsMarkdownLink",
        "quickie.tab.reopenClosed",
        "quickie.reader.toggle",
        "quickie.tab.openInThrowawayJar",
        "quickie.devtools.elements",
        "quickie.devtools.console",
        "quickie.devtools.network",
        "quickie.devtools.sources",
        "quickie.devtools.performance",
        "quickie.devtools.application",
        "quickie.devtools.memory",
        "quickie.page.find",
        "quickie.hints.toggle",
        "quickie.downloads.show",
        "quickie.shields.open",
    ];

    for text in ids {
        assert!(catalog.contains(&id(text)), "{text}");
    }
}

#[test]
fn every_slot_can_be_gone_to_and_pinned() {
    let catalog = Catalog::builtin();

    for n in 1..=9 {
        let pin = catalog
            .get(&id(&format!("quickie.tab.pinToSlot{n}")))
            .unwrap();
        assert_eq!(pin.title, format!("Pin Tab to Slot {n}"));
        assert!(catalog.contains(&id(&format!("quickie.tab.goToSlot{n}"))));
    }
}

#[test]
fn bookmarking_and_throwaway_tabs_are_built_in() {
    let catalog = Catalog::builtin();

    let bookmark = catalog.get(&id("quickie.page.bookmark")).unwrap();
    let throwaway = catalog.get(&id("quickie.tab.newThrowaway")).unwrap();

    assert_eq!(
        (bookmark.title, bookmark.category),
        ("Bookmark Page", Category::Page)
    );
    assert_eq!(
        (throwaway.title, throwaway.category),
        ("Open New Throwaway Tab", Category::Tabs)
    );
}

#[test]
fn the_shields_commands_are_built_in() {
    let catalog = Catalog::builtin();

    let titles: Vec<(&str, Category)> = [
        "quickie.shields.toggleSite",
        "quickie.shields.enableLists",
        "quickie.shields.refreshLists",
    ]
    .into_iter()
    .map(|text| {
        let command = catalog.get(&id(text)).unwrap();
        (command.title, command.category)
    })
    .collect();

    assert_eq!(
        titles,
        [
            ("Toggle Shields for This Site", Category::Shields),
            ("Enable Filter Lists", Category::Shields),
            ("Refresh Filter Lists", Category::Shields),
        ]
    );
}

#[test]
fn the_app_commands_are_built_in() {
    let catalog = Catalog::builtin();

    let titles: Vec<(&str, Category)> =
        ["quickie.app.setDefaultBrowser", "quickie.app.revealProfile"]
            .into_iter()
            .map(|text| {
                let command = catalog.get(&id(text)).unwrap();
                (command.title, command.category)
            })
            .collect();

    assert_eq!(
        titles,
        [
            ("Set as Default Browser", Category::App),
            ("Show Profile in Finder", Category::App),
        ]
    );
}

#[test]
fn every_title_starts_with_a_capitalized_word() {
    let catalog = Catalog::builtin();

    for command in catalog.iter() {
        let first = command.title.chars().next().unwrap();
        assert!(first.is_ascii_uppercase(), "{}", command.title);
    }
}

#[test]
fn every_category_has_its_label() {
    let labels = [
        (Category::Palette, "Palette"),
        (Category::Tabs, "Tabs"),
        (Category::Navigation, "Navigation"),
        (Category::Page, "Page"),
        (Category::Downloads, "Downloads"),
        (Category::Shields, "Shields"),
        (Category::Developer, "Developer"),
        (Category::App, "App"),
    ];

    for (category, label) in labels {
        assert_eq!(category.label(), label);
    }
}

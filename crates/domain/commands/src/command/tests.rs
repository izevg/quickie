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
    ];

    for text in ids {
        assert!(catalog.contains(&id(text)), "{text}");
    }
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
        (Category::Developer, "Developer"),
    ];

    for (category, label) in labels {
        assert_eq!(category.label(), label);
    }
}

//! The SF Symbol before each palette row: one per command, else one per command category, else
//! one per kind of row.

use quickie_domain_palette::Kind;
use quickie_ui_shared::{IconTint, RowIcon};

/// Commands by id, with their symbol.
const COMMANDS: &[(&str, &str)] = &[
    ("quickie.palette.goto", "arrow.right.circle"),
    ("quickie.palette.openCommands", "command"),
    ("quickie.tab.new", "plus.square"),
    ("quickie.tab.newThrowaway", "plus.square.dashed"),
    ("quickie.tab.close", "xmark.square"),
    ("quickie.tab.reopenClosed", "arrow.uturn.backward.square"),
    ("quickie.tab.reload", "arrow.clockwise"),
    ("quickie.tab.hardReload", "arrow.clockwise.circle"),
    ("quickie.tab.overview", "square.grid.2x2"),
    ("quickie.tab.nextRecent", "arrow.right.square"),
    ("quickie.tab.previousRecent", "arrow.left.square"),
    ("quickie.tab.explode", "rectangle.split.3x1"),
    ("quickie.tab.openInThrowawayJar", "eye.slash"),
    ("quickie.window.new", "macwindow.badge.plus"),
    ("quickie.window.close", "macwindow"),
    ("quickie.address.open", "link"),
    ("quickie.navigation.back", "chevron.backward"),
    ("quickie.navigation.forward", "chevron.forward"),
    ("quickie.page.bookmark", "bookmark"),
    ("quickie.page.copyAsMarkdownLink", "link"),
    ("quickie.page.copyTitle", "doc.on.doc"),
    ("quickie.page.copyUrl", "doc.on.clipboard"),
    ("quickie.page.find", "magnifyingglass"),
    ("quickie.page.findNext", "chevron.down"),
    ("quickie.page.findPrevious", "chevron.up"),
    ("quickie.page.print", "printer"),
    ("quickie.page.savePdf", "doc.richtext"),
    ("quickie.page.zoomIn", "plus.magnifyingglass"),
    ("quickie.page.zoomOut", "minus.magnifyingglass"),
    ("quickie.page.zoomReset", "1.magnifyingglass"),
    ("quickie.reader.toggle", "doc.plaintext"),
    ("quickie.hints.toggle", "keyboard"),
    ("quickie.hints.openInBackground", "keyboard"),
    ("quickie.downloads.show", "arrow.down.circle"),
    ("quickie.shields.open", "shield"),
    ("quickie.shields.toggleSite", "shield.lefthalf.filled"),
    (
        "quickie.shields.refreshLists",
        "arrow.triangle.2.circlepath",
    ),
    ("quickie.shields.enableLists", "list.bullet.rectangle"),
    ("quickie.agent.stop", "stop.circle"),
    ("quickie.agent.settings", "gearshape"),
    ("quickie.extensions.settings", "gearshape"),
    ("quickie.devtools.console", "terminal"),
    ("quickie.devtools.network", "network"),
    ("quickie.devtools.inspectElement", "scope"),
    (
        "quickie.devtools.performance",
        "gauge.with.dots.needle.33percent",
    ),
    ("quickie.devtools.memory", "memorychip"),
    ("quickie.devtools.sources", "curlybraces"),
    (
        "quickie.devtools.elements",
        "chevron.left.forwardslash.chevron.right",
    ),
    ("quickie.devtools.application", "externaldrive"),
    ("quickie.app.settings", "gearshape"),
    ("quickie.app.setDefaultBrowser", "globe"),
    ("quickie.app.revealProfile", "folder"),
    ("quickie.debug.keys", "keyboard"),
];

/// Command categories, by the label the palette shows, with their symbol.
const CATEGORIES: &[(&str, &str)] = &[
    ("Palette", "command"),
    ("Tabs", "square.on.square"),
    ("Navigation", "arrow.left.arrow.right"),
    ("Page", "doc"),
    ("Downloads", "arrow.down.circle"),
    ("Shields", "shield"),
    ("Agent", "sparkles"),
    ("Extensions", "puzzlepiece.extension"),
    ("Extension", "puzzlepiece.extension"),
    ("Developer", "hammer"),
    ("App", "app"),
];

/// The symbol of command `id` in `category`: its own, its category's, or the command key.
#[must_use]
pub fn command(id: &str, category: &str) -> RowIcon {
    let tint = if category == "Agent" {
        IconTint::Agent
    } else {
        IconTint::Secondary
    };
    let name = slot(id)
        .or_else(|| lookup(COMMANDS, id).map(str::to_owned))
        .or_else(|| lookup(CATEGORIES, category).map(str::to_owned))
        .unwrap_or_else(|| "command".to_owned());
    RowIcon::Symbol { name, tint }
}

/// The symbol of a row that is not a command.
#[must_use]
pub fn kind(kind: Kind) -> RowIcon {
    let (name, tint) = match kind {
        Kind::OpenTab => ("globe", IconTint::Secondary),
        Kind::Bookmark => ("bookmark", IconTint::Secondary),
        Kind::History => ("clock.arrow.circlepath", IconTint::Secondary),
        Kind::Command => ("command", IconTint::Secondary),
        Kind::Heading => ("number", IconTint::Secondary),
        Kind::Tag => ("tag", IconTint::Secondary),
        Kind::SearchShortcut => ("magnifyingglass", IconTint::Accent),
        Kind::Skill => ("sparkles", IconTint::Agent),
        Kind::Question => ("questionmark.bubble", IconTint::Agent),
    };
    RowIcon::Symbol {
        name: name.to_owned(),
        tint,
    }
}

/// `quickie.tab.goToSlot3` shows `3.square`.
fn slot(id: &str) -> Option<String> {
    let digit = id.strip_prefix("quickie.tab.goToSlot")?;
    (digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{digit}.square"))
}

fn lookup<'a>(table: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|&(_, symbol)| symbol)
}

#[cfg(test)]
mod tests {
    use quickie_domain_palette::Kind;
    use quickie_ui_shared::{IconTint, RowIcon};

    use super::{command, kind};

    fn named(name: &str, tint: IconTint) -> RowIcon {
        RowIcon::Symbol {
            name: name.to_owned(),
            tint,
        }
    }

    #[test]
    fn a_command_has_its_own_symbol() {
        assert_eq!(
            command("quickie.tab.reload", "Tabs"),
            named("arrow.clockwise", IconTint::Secondary)
        );
    }

    #[test]
    fn a_slot_command_shows_its_number() {
        assert_eq!(
            command("quickie.tab.goToSlot3", "Tabs"),
            named("3.square", IconTint::Secondary)
        );
        assert_eq!(
            command("quickie.tab.goToSlot12", "Tabs"),
            named("square.on.square", IconTint::Secondary)
        );
    }

    #[test]
    fn an_unknown_command_falls_back_to_its_category_then_the_command_key() {
        assert_eq!(
            command("quickie.devtools.new", "Developer"),
            named("hammer", IconTint::Secondary)
        );
        assert_eq!(
            command("x.y", "Elsewhere"),
            named("command", IconTint::Secondary)
        );
    }

    #[test]
    fn the_agents_commands_are_orange() {
        assert_eq!(
            command("quickie.agent.ask", "Agent"),
            named("sparkles", IconTint::Agent)
        );
        assert_eq!(kind(Kind::Skill), named("sparkles", IconTint::Agent));
    }

    #[test]
    fn history_shows_a_clock() {
        assert_eq!(
            kind(Kind::History),
            named("clock.arrow.circlepath", IconTint::Secondary)
        );
    }
}

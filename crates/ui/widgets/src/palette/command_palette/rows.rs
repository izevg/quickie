//! The palette's answers as list rows, grouped into sections.
//!
//! A command is one line: its symbol, its title, its category as a tag, its first chord as
//! keycaps. A page is two lines: its favicon (a clock for history without one), its title and
//! how long ago it was visited, its address as people read it, and on the right the slot its tab
//! is pinned to or how often it was visited. Only the title is ever highlighted.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use quickie_application::{PaletteItem, Target};
use quickie_domain_browsing::Timestamp;
use quickie_domain_palette::{Kind, Mode};
use quickie_ui_shared::format::{ago, display_url, visits};
use quickie_ui_shared::{ListRow, RowIcon};
use url::Url;

use super::symbols;

/// The picture a page's site shows, if one is kept: its favicon.
pub type Favicon = dyn Fn(&Url) -> Option<Rc<[u8]>>;

/// The row of `item`, with `now` for its age and `favicon` for its picture. Equal targets make
/// equal keys, so a row that stays in the answer stays in the table.
#[must_use]
pub fn row(item: &PaletteItem, now: Timestamp, favicon: &Favicon) -> ListRow {
    let mut hasher = DefaultHasher::new();
    item.target.hash(&mut hasher);
    let base = ListRow {
        key: hasher.finish(),
        title: item.title.clone(),
        title_matches: item.title_matches.clone(),
        keycaps: item.glyphs.first().cloned(),
        ..ListRow::default()
    };
    match (&item.target, item.kind) {
        (Target::InstallExtension(_), _) => ListRow {
            detail: display_url(&item.detail),
            icon: Some(RowIcon::symbol("puzzlepiece.extension")),
            ..base
        },
        (Target::Command(id), _) => ListRow {
            icon: Some(symbols::command(id.as_str(), &item.detail)),
            tag: Some(item.detail.clone()),
            ..base
        },
        (Target::Extension(_), _) => ListRow {
            icon: Some(symbols::command("", &item.detail)),
            tag: Some(item.detail.clone()),
            ..base
        },
        (_, Kind::OpenTab | Kind::Bookmark | Kind::History) => page(item, now, favicon, base),
        (_, Kind::Tag) => ListRow {
            icon: Some(symbols::kind(Kind::Tag)),
            trailing: Some(item.detail.clone()),
            ..base
        },
        (_, Kind::SearchShortcut) => ListRow {
            detail: display_url(&item.detail),
            icon: Some(symbols::kind(item.kind)),
            ..base
        },
        (_, kind) => ListRow {
            detail: item.detail.clone(),
            icon: Some(symbols::kind(kind)),
            ..base
        },
    }
}

/// A page: two lines, its favicon, its age, and its slot or its visits.
fn page(item: &PaletteItem, now: Timestamp, favicon: &Favicon, base: ListRow) -> ListRow {
    let facts = item.facts;
    let picture = Url::parse(&item.detail)
        .ok()
        .and_then(|url| favicon(&url))
        .map(RowIcon::Picture);
    ListRow {
        detail: display_url(&item.detail),
        icon: picture.or_else(|| Some(symbols::kind(item.kind))),
        note: facts.last_visit.map(|at| ago(now.millis_since(at))),
        badge: facts.slot.map(|slot| format!("⌘{}", slot.get())),
        trailing: (facts.slot.is_none() && facts.visits > 0).then(|| visits(facts.visits)),
        ..base
    }
}

/// A section of the answer: its header and the positions of its items in the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: &'static str,
    pub counter: String,
    pub members: Vec<usize>,
}

/// How the palette's answer to a query in `mode` is grouped: going to, by what each page is
/// (an install offer first, then open tabs, bookmarks, history); every other mode in one
/// section. `capped` says the answer was cut at the palette's limit: the last section's count
/// then reads as a floor (`50+ matches`). Empty sections are left out.
#[must_use]
pub fn groups(items: &[PaletteItem], mode: Mode, query: &str, capped: bool) -> Vec<Group> {
    let mut groups = match mode {
        Mode::GoTo => {
            let of = |wanted: fn(&PaletteItem) -> bool| -> Vec<usize> {
                (0..items.len()).filter(|&i| wanted(&items[i])).collect()
            };
            vec![
                group(
                    "",
                    "",
                    of(|item| matches!(item.target, Target::InstallExtension(_))),
                ),
                group("Open tabs", "tab", of(|item| item.kind == Kind::OpenTab)),
                group("Bookmarks", "page", of(|item| item.kind == Kind::Bookmark)),
                group("History", "page", of(|item| item.kind == Kind::History)),
            ]
        }
        Mode::Commands => vec![group(
            "Commands",
            if query.is_empty() { "command" } else { "match" },
            (0..items.len()).collect(),
        )],
        Mode::Tags => vec![group("Tags", "tag", (0..items.len()).collect())],
        Mode::SearchShortcut => vec![group(
            "Search engines",
            "engine",
            (0..items.len()).collect(),
        )],
        Mode::Agent => vec![group("Agent", "", (0..items.len()).collect())],
        Mode::Headings => vec![group("Headings", "heading", (0..items.len()).collect())],
    };
    groups.retain(|group| !group.members.is_empty());
    if capped
        && let Some(last) = groups.last_mut()
        && !last.counter.is_empty()
    {
        last.counter = last.counter.replacen(' ', "+ ", 1);
    }
    groups
}

/// A group whose counter counts `members` as `noun`s; no counter for no noun.
fn group(title: &'static str, noun: &str, members: Vec<usize>) -> Group {
    let counter = match (noun, members.len()) {
        ("", _) => String::new(),
        (noun, 1) => format!("1 {noun}"),
        ("match", n) => format!("{n} matches"),
        (noun, n) => format!("{n} {noun}s"),
    };
    Group {
        title,
        counter,
        members,
    }
}

/// What the palette says when the query `query` in `mode` finds nothing; `None` for an empty
/// query, which shows no message.
#[must_use]
pub fn nothing_found(mode: Mode, query: &str) -> Option<String> {
    if mode == Mode::Headings {
        return Some("This page's headings are not listed yet".to_owned());
    }
    if query.is_empty() {
        return None;
    }
    let what = match mode {
        Mode::GoTo => "No tabs, bookmarks, or pages match",
        Mode::Commands => "No commands match",
        Mode::Tags => "No tags match",
        Mode::SearchShortcut => "No search engines match",
        Mode::Agent | Mode::Headings => "Nothing matches",
    };
    Some(format!("{what} “{query}”"))
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use quickie_application::{PageFacts, PaletteItem, Target};
    use quickie_domain_browsing::{PinnedSlot, Profile, TabId, Timestamp};
    use quickie_domain_palette::{Kind, Mode};
    use quickie_ui_shared::{IconTint, RowIcon};
    use url::Url;

    use super::{Group, groups, nothing_found, row};

    const NOW: Timestamp = Timestamp::from_millis(10 * 60_000);

    fn no_favicon(_: &Url) -> Option<Rc<[u8]>> {
        None
    }

    fn item(url: &str, kind: Kind, title: &str) -> PaletteItem {
        PaletteItem {
            target: Target::Url(Url::parse(url).unwrap()),
            kind,
            title: title.to_owned(),
            detail: url.to_owned(),
            glyphs: Vec::new(),
            title_matches: vec![0, 1],
            detail_matches: vec![8],
            facts: PageFacts::default(),
        }
    }

    fn command(id: &str, title: &str, category: &str, glyphs: &[&str]) -> PaletteItem {
        PaletteItem {
            target: Target::Command(id.try_into().unwrap()),
            kind: Kind::Command,
            title: title.to_owned(),
            detail: category.to_owned(),
            glyphs: glyphs.iter().map(|&glyph| glyph.to_owned()).collect(),
            title_matches: vec![0],
            detail_matches: vec![0],
            facts: PageFacts::default(),
        }
    }

    #[test]
    fn a_command_is_one_line_with_its_category_as_a_tag() {
        let row = row(
            &command("quickie.tab.reload", "Reload", "Tabs", &["⌘R", "F5"]),
            NOW,
            &no_favicon,
        );

        assert_eq!(row.title, "Reload");
        assert_eq!(row.title_matches, [0]);
        assert_eq!(row.detail, "");
        assert_eq!(row.tag.as_deref(), Some("Tabs"));
        assert_eq!(row.keycaps.as_deref(), Some("⌘R"));
        assert!(
            matches!(row.icon, Some(RowIcon::Symbol { ref name, .. }) if name == "arrow.clockwise")
        );
    }

    #[test]
    fn a_history_page_reads_its_address_age_and_visits() {
        let mut page = item("https://ru.wikipedia.org/wiki/%D0%90", Kind::History, "А");
        page.facts = PageFacts {
            visits: 6,
            last_visit: Some(Timestamp::from_millis(5 * 60_000)),
            slot: None,
        };

        let row = row(&page, NOW, &no_favicon);

        assert_eq!(row.detail, "ru.wikipedia.org/wiki/А");
        assert_eq!(row.note.as_deref(), Some("5 min ago"));
        assert_eq!(row.trailing.as_deref(), Some("Visited 6 times"));
        assert_eq!(row.badge, None);
        assert_eq!(row.tag, None);
        assert_eq!(
            row.icon,
            Some(RowIcon::Symbol {
                name: "clock.arrow.circlepath".to_owned(),
                tint: IconTint::Secondary
            })
        );
    }

    #[test]
    fn a_pinned_tab_shows_its_slot_instead_of_its_visits() {
        let mut tab = item("https://news.ycombinator.com/", Kind::OpenTab, "HN");
        tab.target = Target::Tab(TabId::new(Profile::Main, 4));
        tab.facts = PageFacts {
            visits: 3,
            last_visit: None,
            slot: Some(PinnedSlot::try_from(3).unwrap()),
        };

        let row = row(&tab, NOW, &no_favicon);

        assert_eq!(row.badge.as_deref(), Some("⌘3"));
        assert_eq!(row.trailing, None);
        assert_eq!(row.note, None);
        assert_eq!(row.detail, "news.ycombinator.com");
    }

    #[test]
    fn a_kept_favicon_replaces_the_glyph() {
        let bytes: Rc<[u8]> = Rc::from(&b"png"[..]);
        let kept = Rc::clone(&bytes);
        let favicon =
            move |url: &Url| (url.host_str() == Some("a.example")).then(|| Rc::clone(&kept));

        let row = row(
            &item("https://a.example/x", Kind::History, "A"),
            NOW,
            &favicon,
        );

        assert_eq!(row.icon, Some(RowIcon::Picture(bytes)));
    }

    #[test]
    fn the_target_is_the_key() {
        let first = row(
            &item("https://a.example/", Kind::History, "Alpha"),
            NOW,
            &no_favicon,
        );
        let renamed = row(
            &item("https://a.example/", Kind::History, "Alpha, renamed"),
            NOW,
            &no_favicon,
        );
        let other = row(
            &item("https://b.example/", Kind::History, "Alpha"),
            NOW,
            &no_favicon,
        );

        assert_eq!(first.key, renamed.key);
        assert_ne!(first.key, other.key);
    }

    #[test]
    fn going_to_groups_tabs_then_bookmarks_then_history() {
        let items = [
            item("https://h.example/", Kind::History, "H"),
            item("https://t.example/", Kind::OpenTab, "T"),
            item("https://b.example/", Kind::Bookmark, "B"),
            item("https://u.example/", Kind::OpenTab, "U"),
        ];

        let groups = groups(&items, Mode::GoTo, "", false);

        assert_eq!(
            groups,
            [
                Group {
                    title: "Open tabs",
                    counter: "2 tabs".to_owned(),
                    members: vec![1, 3]
                },
                Group {
                    title: "Bookmarks",
                    counter: "1 page".to_owned(),
                    members: vec![2]
                },
                Group {
                    title: "History",
                    counter: "1 page".to_owned(),
                    members: vec![0]
                },
            ]
        );
    }

    #[test]
    fn commands_count_what_matched() {
        let items = [command("quickie.tab.reload", "Reload", "Tabs", &[])];

        assert_eq!(
            groups(&items, Mode::Commands, "rel", false)[0].counter,
            "1 match"
        );
        assert_eq!(
            groups(&items, Mode::Commands, "", false)[0].counter,
            "1 command"
        );
        let two = [items[0].clone(), items[0].clone()];
        assert_eq!(
            groups(&two, Mode::Commands, "re", false)[0].counter,
            "2 matches"
        );
    }

    #[test]
    fn a_capped_answer_counts_a_floor() {
        let items = [
            command("quickie.tab.reload", "Reload", "Tabs", &[]),
            command("quickie.tab.close", "Close", "Tabs", &[]),
        ];

        assert_eq!(
            groups(&items, Mode::Commands, "", true)[0].counter,
            "2+ commands"
        );
    }

    #[test]
    fn nothing_found_names_the_query() {
        assert_eq!(
            nothing_found(Mode::Commands, "qqqqzz").as_deref(),
            Some("No commands match “qqqqzz”")
        );
        assert_eq!(nothing_found(Mode::GoTo, ""), None);
        assert!(nothing_found(Mode::Headings, "").is_some());
    }
}

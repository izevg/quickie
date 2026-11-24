//! The Linux palette offers only actions its shell can run.

use quickie_application::{
    PaletteIndex, PaletteItem, Suggestion, SuggestionKind, Target as Choice,
};
use quickie_domain_browsing::{Destination, TabId};
use quickie_domain_commands::Catalog;
use quickie_domain_palette::{Candidate, Kind, Mode as QueryMode, Query, Ranker};

const LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Address,
    Commands,
    GoTo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Address,
    Back,
    Forward,
    OpenCommands,
    Reload,
    HardReload,
    Inspector,
    Find,
    FindNext,
    FindPrevious,
    LinkHints,
    BackgroundLinkHints,
    CopyUrl,
    Bookmark,
    Updates,
    Settings,
    GoTo,
    NewTab,
    NewThrowaway,
    CloseTab,
    ReopenTab,
    Overview,
    NextTab,
    PreviousTab,
    Agent,
    AgentSettings,
    Shields,
    Downloads,
}

impl Command {
    const ALL: [Self; 28] = [
        Self::Address,
        Self::Back,
        Self::Forward,
        Self::OpenCommands,
        Self::Reload,
        Self::HardReload,
        Self::Inspector,
        Self::Find,
        Self::FindNext,
        Self::FindPrevious,
        Self::LinkHints,
        Self::BackgroundLinkHints,
        Self::CopyUrl,
        Self::Bookmark,
        Self::Updates,
        Self::Settings,
        Self::GoTo,
        Self::NewTab,
        Self::NewThrowaway,
        Self::CloseTab,
        Self::ReopenTab,
        Self::Overview,
        Self::NextTab,
        Self::PreviousTab,
        Self::Agent,
        Self::AgentSettings,
        Self::Shields,
        Self::Downloads,
    ];

    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Address => "quickie.address.open",
            Self::Back => "quickie.navigation.back",
            Self::Forward => "quickie.navigation.forward",
            Self::OpenCommands => "quickie.palette.openCommands",
            Self::Reload => "quickie.tab.reload",
            Self::HardReload => "quickie.tab.hardReload",
            Self::Inspector => "quickie.devtools.toggle",
            Self::Find => "quickie.page.find",
            Self::FindNext => "quickie.page.findNext",
            Self::FindPrevious => "quickie.page.findPrevious",
            Self::LinkHints => "quickie.hints.toggle",
            Self::BackgroundLinkHints => "quickie.hints.openInBackground",
            Self::CopyUrl => "quickie.page.copyUrl",
            Self::Bookmark => "quickie.page.bookmark",
            Self::Settings | Self::Updates => "quickie.app.settings",
            Self::GoTo => "quickie.palette.goto",
            Self::NewTab => "quickie.tab.new",
            Self::NewThrowaway => "quickie.tab.newThrowaway",
            Self::CloseTab => "quickie.tab.close",
            Self::ReopenTab => "quickie.tab.reopenClosed",
            Self::Overview => "quickie.tab.overview",
            Self::NextTab => "quickie.tab.nextRecent",
            Self::PreviousTab => "quickie.tab.previousRecent",
            Self::Agent => "quickie.agent.ask",
            Self::AgentSettings => "quickie.agent.settings",
            Self::Shields => "quickie.shields.open",
            Self::Downloads => "quickie.downloads.show",
        }
    }

    pub(super) fn of(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|&command| command != Self::Updates && command.id() == id)
    }

    const fn icon(self) -> &'static str {
        match self {
            Self::Address | Self::Find | Self::FindNext | Self::FindPrevious => {
                "edit-find-symbolic"
            }
            Self::Back => "go-previous-symbolic",
            Self::Forward => "go-next-symbolic",
            Self::Reload | Self::HardReload => "view-refresh-symbolic",
            Self::Inspector => "utilities-terminal-symbolic",
            Self::CopyUrl => "edit-copy-symbolic",
            Self::Bookmark => "bookmark-new-symbolic",
            Self::LinkHints | Self::BackgroundLinkHints => "insert-link-symbolic",
            Self::Updates => "system-software-update-symbolic",
            Self::GoTo | Self::OpenCommands | Self::NextTab | Self::PreviousTab => {
                "go-jump-symbolic"
            }
            Self::NewTab | Self::NewThrowaway => "tab-new-symbolic",
            Self::CloseTab => "window-close-symbolic",
            Self::ReopenTab => "edit-undo-symbolic",
            Self::Overview => "view-grid-symbolic",
            Self::Agent => "system-search-symbolic",
            Self::Settings | Self::AgentSettings => "preferences-system-symbolic",
            Self::Shields => "security-high-symbolic",
            Self::Downloads => "folder-download-symbolic",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Command(Command),
    Navigate(String),
    Visit(String),
    Tab(TabId),
    Ask(String),
}

impl Target {
    fn kind(&self) -> Kind {
        match self {
            Self::Command(_) => Kind::Command,
            Self::Tab(_) => Kind::OpenTab,
            Self::Ask(_) => Kind::Question,
            Self::Navigate(_) | Self::Visit(_) => Kind::History,
        }
    }
}

pub(super) struct Item {
    pub target: Target,
    pub title: String,
    pub detail: String,
    pub shortcut: String,
    pub icon: &'static str,
    pub emphasis: Vec<u32>,
}

pub(super) struct Model {
    catalog: Catalog,
    ranker: Ranker,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            catalog: Catalog::builtin(),
            ranker: Ranker::default(),
        }
    }
}

impl Model {
    pub fn items(
        &mut self,
        mode: Mode,
        input: &str,
        current: &str,
        index: Option<&mut PaletteIndex>,
    ) -> Vec<Item> {
        if mode != Mode::Address && input.starts_with('?') {
            let query = Query::parse(input);
            let question = query.text();
            return (query.mode() == QueryMode::Agent && !question.trim().is_empty())
                .then(|| Item {
                    target: Target::Ask(question.into()),
                    title: "Ask about this page".into(),
                    detail: question.into(),
                    shortcut: String::new(),
                    icon: "system-search-symbolic",
                    emphasis: Vec::new(),
                })
                .into_iter()
                .collect();
        }
        match mode {
            Mode::Address => {
                if input == current && !current.is_empty() {
                    return self.command_item(Command::CopyUrl).into_iter().collect();
                }
                index.map_or_else(Vec::new, |index| address_rows(index, input))
            }
            Mode::GoTo => index.map_or_else(Vec::new, |index| {
                index
                    .pages(input, LIMIT)
                    .into_iter()
                    .filter_map(palette_row)
                    .collect()
            }),
            Mode::Commands => {
                let command = input.trim_start().strip_prefix('>').unwrap_or(input);
                let query = Query::parse(&format!(">{command}"));
                let items = Command::ALL
                    .into_iter()
                    .filter_map(|command| self.command_item(command))
                    .collect();
                self.rank(&query, items)
            }
        }
    }

    fn command_item(&self, command: Command) -> Option<Item> {
        let entry = self
            .catalog
            .iter()
            .find(|entry| entry.id.as_str() == command.id())?;
        let title = if command == Command::Updates {
            "Software Updates"
        } else {
            entry.title
        };
        Some(Item {
            target: Target::Command(command),
            title: title.into(),
            detail: entry.category.label().into(),
            shortcut: String::new(),
            icon: command.icon(),
            emphasis: Vec::new(),
        })
    }

    fn rank(&mut self, query: &Query, items: Vec<Item>) -> Vec<Item> {
        let hits = self.ranker.rank(
            query,
            items.iter().enumerate().map(|(slot, item)| Candidate {
                key: slot,
                kind: item.target.kind(),
                title: item.title.as_str().into(),
                detail: item.detail.as_str().into(),
                frecency: 0,
            }),
            LIMIT,
        );
        let mut items = items.into_iter().map(Some).collect::<Vec<_>>();
        hits.into_iter()
            .filter_map(|hit| {
                let mut item = items[hit.key].take()?;
                item.emphasis = hit.title_matches;
                Some(item)
            })
            .collect()
    }
}

fn row(
    choice: Choice,
    kind: Kind,
    title: String,
    detail: String,
    emphasis: Vec<u32>,
) -> Option<Item> {
    let target = match choice {
        Choice::Tab(tab) => Target::Tab(tab),
        Choice::Url(url) if matches!(kind, Kind::Bookmark | Kind::History) => {
            Target::Visit(url.into())
        }
        Choice::Url(url) | Choice::Search(url) => Target::Navigate(url.into()),
        _ => return None,
    };
    let icon = match kind {
        Kind::Bookmark => "bookmark-new-symbolic",
        Kind::History => "document-open-recent-symbolic",
        Kind::SearchShortcut => "edit-find-symbolic",
        _ => "web-browser-symbolic",
    };
    Some(Item {
        target,
        title,
        detail,
        shortcut: String::new(),
        icon,
        emphasis,
    })
}

fn palette_row(item: PaletteItem) -> Option<Item> {
    row(
        item.target,
        item.kind,
        item.title,
        item.detail,
        item.title_matches,
    )
}

fn address_row(item: Suggestion) -> Option<Item> {
    let kind = match item.kind {
        SuggestionKind::OpenTab => Kind::OpenTab,
        SuggestionKind::Bookmark => Kind::Bookmark,
        SuggestionKind::History => Kind::History,
        SuggestionKind::Search => Kind::SearchShortcut,
        SuggestionKind::GoTo => Kind::Command,
    };
    row(item.target, kind, item.title, item.url, item.title_matches)
}

fn address_rows(index: &mut PaletteIndex, input: &str) -> Vec<Item> {
    let mut rows: Vec<_> = index
        .suggest(input, false)
        .rows
        .into_iter()
        .filter_map(address_row)
        .collect();
    // A typed address is an explicit navigation in the current tab, including a
    // throwaway tab. Only choosing a tab suggestion can switch to another jar.
    if let Some(first) = rows.first_mut()
        && matches!(first.target, Target::Tab(_))
        && let Some(Destination::Address(url)) = Destination::of(input)
        && first.detail == url.as_str()
    {
        first.title = format!("Go to {}", input.trim());
        first.target = Target::Navigate(url.into());
        first.emphasis.clear();
    }
    rows
}

pub(super) fn completion<'a>(items: &'a [Item], typed: &str) -> Option<&'a str> {
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    items.iter().find_map(|item| {
        let address = match &item.target {
            Target::Visit(address) => address.as_str(),
            Target::Tab(_) => item.detail.as_str(),
            _ => return None,
        };
        let without_scheme = address.split_once("://").map_or(address, |(_, rest)| rest);
        [address, without_scheme]
            .into_iter()
            .any(|part| {
                part.strip_prefix(typed)
                    .is_some_and(|suffix| !suffix.is_empty())
            })
            .then_some(address)
    })
}

#[cfg(test)]
mod tests {
    use quickie_application::{Palette, Sources};
    use quickie_domain_browsing::{Focus, Jar, JarChoice, Profile, Session, Tab, TabId, Timestamp};
    use quickie_domain_commands::{Catalog, Keymap, Platform};
    use quickie_domain_library::{Bookmarks, History, SearchShortcuts};
    use url::Url;

    use super::{Command, Item, Mode, Model, Target, completion};
    use crate::gtk::browsing::SystemClock;

    struct Visit {
        title: String,
        address: String,
    }

    fn items(mode: Mode, query: &str, current: &str, visits: &[Visit], tabs: &[Tab]) -> Vec<Item> {
        let mut session = Session::new(Profile::Main);
        let window = session.open_window();
        let live: Vec<_> = tabs
            .iter()
            .map(|tab| {
                let id = session
                    .open(window, tab.url.clone(), JarChoice::Main, Focus::Foreground)
                    .unwrap();
                session
                    .navigated(id, tab.url.clone(), tab.title.clone())
                    .unwrap();
                (id, tab.id)
            })
            .collect();
        let mut history = History::new();
        for visit in visits {
            history.record_visit(
                &Url::parse(&visit.address).unwrap(),
                &visit.title,
                Timestamp::from_millis(1),
            );
        }
        let mut index = Palette::new(Catalog::builtin(), SystemClock).open(&Sources {
            session: &session,
            history: &history,
            bookmarks: &Bookmarks::new(),
            shortcuts: &SearchShortcuts::defaults(),
            keymap: &Keymap::defaults(Platform::Linux),
            extensions: &[],
        });
        let mut items = Model::default().items(mode, query, current, Some(&mut index));
        for item in &mut items {
            if let Target::Tab(tab) = &mut item.target {
                *tab = live.iter().find(|(id, _)| id == tab).unwrap().1;
            }
        }
        items
    }

    #[test]
    fn command_search_returns_an_executable_reload_with_match_positions() {
        let items = items(Mode::Commands, "reload tab", "", &[], &[]);

        assert_eq!(items[0].target, Target::Command(Command::Reload));
        assert_ne!(items[0].emphasis, [] as [u32; 0]);
    }

    #[test]
    fn unsupported_command_search_has_no_action() {
        let items = items(Mode::Commands, ">unavailable-shortcut", "", &[], &[]);

        assert!(items.is_empty());
    }

    #[test]
    fn address_punctuation_is_plain_text_and_uses_shared_destination_rules() {
        let items = items(Mode::Address, "? GTK", "", &[], &[]);

        assert!(
            matches!(&items[0].target, Target::Navigate(url) if url.starts_with("https://duckduckgo.com/"))
        );
    }

    #[test]
    fn history_matches_in_the_original_script() {
        let history = [
            Visit {
                title: "Суверенитет".into(),
                address: "https://example.com/russian".into(),
            },
            Visit {
                title: "Navigation".into(),
                address: "https://example.com/".into(),
            },
        ];

        let items = items(Mode::GoTo, "сувер", "", &history, &[]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Суверенитет");
    }

    #[test]
    fn a_history_miss_offers_no_navigation_target() {
        let history = [Visit {
            title: "Visited page".into(),
            address: "https://example.org/history".into(),
        }];

        let items = items(Mode::GoTo, "zzzxxxx", "", &history, &[]);

        assert_eq!(items.len(), 0);
    }

    #[test]
    fn go_to_searches_a_leading_punctuation_character_literally() {
        let history = [Visit {
            title: "@alice".into(),
            address: "https://example.org/alice".into(),
        }];
        let items = items(Mode::GoTo, "@alice", "", &history, &[]);

        assert_eq!(
            items[0].target,
            Target::Visit("https://example.org/alice".into())
        );
    }

    #[test]
    fn address_suggestions_offer_the_typed_search_and_ranked_page() {
        let history = [Visit {
            title: "Суверенитет".into(),
            address: "https://example.org/history".into(),
        }];
        let items = items(Mode::Address, "сувер", "", &history, &[]);

        assert!(
            matches!(&items[0].target, Target::Navigate(url) if url.starts_with("https://duckduckgo.com/"))
        );
        assert!(
            matches!(&items[1].target, Target::Visit(url) if url == "https://example.org/history")
        );
    }

    #[test]
    fn an_already_visited_address_is_not_offered_twice() {
        let url = "https://example.org/";
        let history = [Visit {
            title: "Example".into(),
            address: url.into(),
        }];
        let items = items(Mode::Address, url, "", &history, &[]);

        assert_eq!(items[0].target, Target::Visit(url.into()));
        assert_eq!(
            items
                .iter()
                .filter(|item| item.target == Target::Visit(url.into()))
                .count(),
            1
        );
    }

    #[test]
    fn completion_accepts_an_actual_visited_url_prefix() {
        let complete = "https://example.org/history";
        let history = [Visit {
            title: "Visited Page".into(),
            address: complete.into(),
        }];
        let items = items(Mode::Address, "example.org/his", "", &history, &[]);

        assert_eq!(completion(&items, "example.org/his"), Some(complete));
        assert_eq!(completion(&items, "Visited Page"), None);
        assert_eq!(completion(&items, complete), None);
    }

    #[test]
    fn go_to_returns_a_live_cyrillic_tab_by_id_when_the_url_is_also_in_history() {
        let id = TabId::new(Profile::Main, 42);
        let address = "https://example.org/review";
        let tab = Tab {
            id,
            url: Url::parse(address).unwrap(),
            title: "Обзор браузера".into(),
            jar: Jar::Main,
        };
        let history = [Visit {
            title: tab.title.clone(),
            address: address.into(),
        }];

        let items = items(Mode::GoTo, "обзор", "", &history, &[tab]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, Target::Tab(id));
    }

    #[test]
    fn an_address_search_can_select_an_open_tab_instead_of_loading_it() {
        let id = TabId::new(Profile::Main, 42);
        let tab = Tab {
            id,
            url: Url::parse("https://example.org/independent-page").unwrap(),
            title: "Independent page".into(),
            jar: Jar::Main,
        };

        let items = items(Mode::Address, "independent", "", &[], &[tab]);

        assert!(matches!(items[0].target, Target::Navigate(_)));
        assert_eq!(items[1].target, Target::Tab(id));
    }

    #[test]
    fn an_exact_typed_address_opens_in_the_current_tab() {
        let address = "https://example.org/independent-page";
        let tab = Tab {
            id: TabId::new(Profile::Main, 42),
            url: Url::parse(address).unwrap(),
            title: "Independent page".into(),
            jar: Jar::Main,
        };
        let items = items(Mode::Address, address, "", &[], &[tab]);

        assert_eq!(items[0].target, Target::Navigate(address.into()));
    }

    #[test]
    fn an_explicit_question_exposes_one_typed_agent_action() {
        let question = "Where does this page say the train stops?";

        let items = items(Mode::Commands, &format!("? {question}"), "", &[], &[]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, Target::Ask(question.into()));
    }
}

//! The Linux palette offers only actions its shell can run.

use quickie_domain_browsing::{Destination, Tab, TabId};
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
    const ALL: [Self; 27] = [
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

pub(super) struct Visit {
    pub title: String,
    pub address: String,
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
        history: &[Visit],
        tabs: &[Tab],
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
                if input.trim().is_empty() {
                    return Vec::new();
                }
                let direct = destination(input);
                let direct_url = direct.as_ref().and_then(|item| match &item.target {
                    Target::Navigate(address) => Some(address.as_str()),
                    _ => None,
                });
                let matching = self.go_to(input, history, tabs);
                let matching = matching
                    .into_iter()
                    .filter(|item| {
                        !matches!(item.target, Target::Visit(_))
                            || Some(item.detail.as_str()) != direct_url
                    })
                    .take(LIMIT - 1)
                    .collect::<Vec<_>>();
                direct.into_iter().chain(matching).collect()
            }
            Mode::GoTo => self.go_to(input, history, tabs),
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

    fn go_to(&mut self, input: &str, history: &[Visit], tabs: &[Tab]) -> Vec<Item> {
        let live = tabs.iter().map(|tab| Item {
            target: Target::Tab(tab.id),
            title: if tab.title.trim().is_empty() {
                if tab.url.as_str() == "about:blank" {
                    "New Tab".into()
                } else {
                    tab.url.as_str().into()
                }
            } else {
                tab.title.clone()
            },
            detail: tab.url.as_str().into(),
            shortcut: String::new(),
            icon: "web-browser-symbolic",
            emphasis: Vec::new(),
        });
        let visited = history
            .iter()
            .filter(|visit| !tabs.iter().any(|tab| tab.url.as_str() == visit.address))
            .map(visit_item);
        self.rank(&Query::go_to(input), live.chain(visited).collect())
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

fn visit_item(visit: &Visit) -> Item {
    Item {
        target: Target::Visit(visit.address.clone()),
        title: visit.title.clone(),
        detail: visit.address.clone(),
        shortcut: String::new(),
        icon: "document-open-recent-symbolic",
        emphasis: Vec::new(),
    }
}

fn destination(input: &str) -> Option<Item> {
    let destination = Destination::of(input)?;
    let (title, detail, icon) = match &destination {
        Destination::Address(url) => ("Open Page", url.as_str(), "web-browser-symbolic"),
        Destination::Search(_) => ("Search with DuckDuckGo", input.trim(), "edit-find-symbolic"),
    };
    Some(Item {
        target: Target::Navigate(destination.url().as_str().into()),
        title: title.into(),
        detail: detail.into(),
        shortcut: String::new(),
        icon,
        emphasis: Vec::new(),
    })
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
    use quickie_domain_browsing::{Jar, Profile, Tab, TabId};
    use url::Url;

    use super::{Command, Mode, Model, Target, Visit, completion};

    #[test]
    fn command_search_returns_an_executable_reload_with_match_positions() {
        let items = Model::default().items(Mode::Commands, "reload tab", "", &[], &[]);

        assert_eq!(items[0].target, Target::Command(Command::Reload));
        assert_ne!(items[0].emphasis, [] as [u32; 0]);
    }

    #[test]
    fn unsupported_command_search_has_no_action() {
        let items = Model::default().items(Mode::Commands, ">unavailable-shortcut", "", &[], &[]);

        assert!(items.is_empty());
    }

    #[test]
    fn address_punctuation_is_plain_text_and_uses_shared_destination_rules() {
        let items = Model::default().items(Mode::Address, "? GTK", "", &[], &[]);

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

        let items = Model::default().items(Mode::GoTo, "сувер", "", &history, &[]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Суверенитет");
    }

    #[test]
    fn a_history_miss_offers_no_navigation_target() {
        let history = [Visit {
            title: "Visited page".into(),
            address: "https://example.org/history".into(),
        }];

        let items = Model::default().items(Mode::GoTo, "zzzxxxx", "", &history, &[]);

        assert_eq!(items.len(), 0);
    }

    #[test]
    fn address_suggestions_offer_the_typed_search_and_ranked_page() {
        let history = [Visit {
            title: "Суверенитет".into(),
            address: "https://example.org/history".into(),
        }];
        let items = Model::default().items(Mode::Address, "сувер", "", &history, &[]);

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
        let items = Model::default().items(Mode::Address, url, "", &history, &[]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, Target::Navigate(url.into()));
    }

    #[test]
    fn completion_accepts_an_actual_visited_url_prefix() {
        let complete = "https://example.org/history";
        let history = [Visit {
            title: "Visited Page".into(),
            address: complete.into(),
        }];
        let items = Model::default().items(Mode::Address, "example.org/his", "", &history, &[]);

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

        let items = Model::default().items(Mode::GoTo, "обзор", "", &history, &[tab]);

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

        let items = Model::default().items(Mode::Address, "independent", "", &[], &[tab]);

        assert!(matches!(items[0].target, Target::Navigate(_)));
        assert_eq!(items[1].target, Target::Tab(id));
    }

    #[test]
    fn an_explicit_question_exposes_one_typed_agent_action() {
        let question = "Where does this page say the train stops?";

        let items = Model::default().items(Mode::Commands, &format!("? {question}"), "", &[], &[]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, Target::Ask(question.into()));
    }
}

//! What the user typed into the palette, read as a mode and the text that mode searches for.

use crate::Kind;

/// What the palette searches, chosen by the first character of the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// No prefix: open tabs, bookmarks, and history.
    GoTo,
    /// `>`: commands.
    Commands,
    /// `@`: headings of the current page.
    Headings,
    /// `#`: bookmark tags.
    Tags,
    /// `?`: a question for the agent. Never fuzzy matched; the text goes to the agent as typed.
    Agent,
    /// `!`: a keyword search such as `!gh quickie`.
    SearchShortcut,
}

impl Mode {
    /// The mode a prefix character selects, or `None` when the character is ordinary text.
    #[must_use]
    pub const fn from_prefix(prefix: char) -> Option<Self> {
        match prefix {
            '>' => Some(Self::Commands),
            '@' => Some(Self::Headings),
            '#' => Some(Self::Tags),
            '?' => Some(Self::Agent),
            '!' => Some(Self::SearchShortcut),
            _ => None,
        }
    }

    /// The character that selects this mode; `GoTo` has none.
    #[must_use]
    pub const fn prefix(self) -> Option<char> {
        match self {
            Self::GoTo => None,
            Self::Commands => Some('>'),
            Self::Headings => Some('@'),
            Self::Tags => Some('#'),
            Self::Agent => Some('?'),
            Self::SearchShortcut => Some('!'),
        }
    }

    /// Whether a candidate of `kind` can answer a query in this mode. The agent mode ranks only
    /// its skills; the question itself is the text as typed, never a ranked candidate.
    #[must_use]
    pub const fn admits(self, kind: Kind) -> bool {
        match self {
            Self::GoTo => matches!(kind, Kind::OpenTab | Kind::Bookmark | Kind::History),
            Self::Commands => matches!(kind, Kind::Command),
            Self::Headings => matches!(kind, Kind::Heading),
            Self::Tags => matches!(kind, Kind::Tag),
            Self::SearchShortcut => matches!(kind, Kind::SearchShortcut),
            Self::Agent => matches!(kind, Kind::Skill),
        }
    }
}

/// A parsed palette input: the mode its prefix selects and the text after the prefix, with the
/// whitespace between prefix and text dropped.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Query {
    mode: Mode,
    text: String,
}

/// A `!` query split into the shortcut's keyword and the terms it searches for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeywordSearch<'a> {
    /// Which search shortcut to use, such as `gh`.
    pub keyword: &'a str,
    /// What to search for with it, such as `quickie`; may be empty.
    pub terms: &'a str,
}

impl Query {
    /// Reads the palette input. Only the very first character can be a prefix; any input is a
    /// valid query, so parsing cannot fail.
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let mut chars = input.chars();
        let (mode, rest) = match chars.next().and_then(Mode::from_prefix) {
            Some(mode) => (mode, chars.as_str()),
            None => (Mode::GoTo, input),
        };
        Self {
            mode,
            text: rest.trim_start().to_owned(),
        }
    }

    /// Reads `input` as go-to text whatever its first character, as the address overlay does:
    /// it has no modes, so `#anchor` or `?q` are text like any other.
    #[must_use]
    pub fn go_to(input: &str) -> Self {
        Self {
            mode: Mode::GoTo,
            text: input.trim_start().to_owned(),
        }
    }

    /// The mode the prefix selected.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// The text after the prefix. In the agent mode this is the question exactly as typed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The keyword and the search terms of a `!` query; `None` in every other mode.
    #[must_use]
    pub fn keyword_search(&self) -> Option<KeywordSearch<'_>> {
        (self.mode == Mode::SearchShortcut).then(|| {
            let (keyword, terms) = self
                .text
                .split_once(char::is_whitespace)
                .unwrap_or((&self.text, ""));
            KeywordSearch {
                keyword,
                terms: terms.trim_start(),
            }
        })
    }

    /// The text candidates are matched against: the keyword alone for a `!` query, since the
    /// terms belong to the search engine, not to the palette.
    pub(crate) fn needle(&self) -> &str {
        self.keyword_search()
            .map_or(self.text(), |search| search.keyword)
    }
}

#[cfg(test)]
mod tests {
    use super::{KeywordSearch, Mode, Query};
    use crate::Kind;

    const MODES: [Mode; 6] = [
        Mode::GoTo,
        Mode::Commands,
        Mode::Headings,
        Mode::Tags,
        Mode::Agent,
        Mode::SearchShortcut,
    ];

    #[test]
    fn each_prefix_selects_its_mode() {
        let parsed: Vec<Mode> = [">", "@", "#", "?", "!", ""]
            .iter()
            .map(|input| Query::parse(input).mode())
            .collect();

        assert_eq!(
            parsed,
            [
                Mode::Commands,
                Mode::Headings,
                Mode::Tags,
                Mode::Agent,
                Mode::SearchShortcut,
                Mode::GoTo,
            ]
        );
    }

    #[test]
    fn every_prefixed_mode_is_selected_by_its_own_prefix() {
        for mode in MODES.into_iter().filter(|&mode| mode != Mode::GoTo) {
            let selected = mode.prefix().and_then(Mode::from_prefix);

            assert_eq!(selected, Some(mode), "{mode:?}");
        }
    }

    #[test]
    fn go_to_has_no_prefix() {
        assert_eq!(Mode::GoTo.prefix(), None);
    }

    #[test]
    fn input_without_a_prefix_goes_to_tabs_and_history() {
        let query = Query::parse("pr review");

        assert_eq!((query.mode(), query.text()), (Mode::GoTo, "pr review"));
    }

    #[test]
    fn whitespace_after_the_prefix_is_dropped() {
        let query = Query::parse(">  reload ");

        assert_eq!((query.mode(), query.text()), (Mode::Commands, "reload "));
    }

    #[test]
    fn leading_whitespace_of_a_go_to_query_is_dropped() {
        assert_eq!(Query::parse("  docs").text(), "docs");
    }

    #[test]
    fn a_prefix_counts_only_as_the_first_character() {
        let query = Query::parse(" >reload");

        assert_eq!((query.mode(), query.text()), (Mode::GoTo, ">reload"));
    }

    #[test]
    fn go_to_text_has_no_prefix_at_all() {
        let query = Query::go_to("  #top?x ");

        assert_eq!((query.mode(), query.text()), (Mode::GoTo, "#top?x "));
        assert_eq!(query.needle(), "#top?x ");
    }

    #[test]
    fn an_agent_question_keeps_its_own_spelling() {
        let query = Query::parse("? What's  this page ABOUT?  ");

        assert_eq!(
            (query.mode(), query.text()),
            (Mode::Agent, "What's  this page ABOUT?  ")
        );
    }

    #[test]
    fn a_keyword_search_splits_into_keyword_and_terms() {
        let query = Query::parse("!gh   quickie browser");

        assert_eq!(
            query.keyword_search(),
            Some(KeywordSearch {
                keyword: "gh",
                terms: "quickie browser"
            })
        );
    }

    #[test]
    fn a_bare_keyword_has_no_terms() {
        let query = Query::parse("! gh");

        assert_eq!(
            query.keyword_search(),
            Some(KeywordSearch {
                keyword: "gh",
                terms: ""
            })
        );
    }

    #[test]
    fn only_a_bang_query_is_a_keyword_search() {
        assert_eq!(Query::parse("gh quickie").keyword_search(), None);
    }

    #[test]
    fn a_keyword_search_matches_by_its_keyword_only() {
        assert_eq!(Query::parse("!gh quickie").needle(), "gh");
    }

    #[test]
    fn other_modes_match_by_their_whole_text() {
        assert_eq!(Query::parse(">reload page").needle(), "reload page");
    }

    #[test]
    fn go_to_admits_tabs_bookmarks_and_history_only() {
        let admitted: Vec<Kind> = Kind::ALL
            .into_iter()
            .filter(|&kind| Mode::GoTo.admits(kind))
            .collect();

        assert_eq!(admitted, [Kind::OpenTab, Kind::Bookmark, Kind::History]);
    }

    #[test]
    fn every_other_list_mode_admits_one_kind() {
        let modes = [
            (Mode::Commands, Kind::Command),
            (Mode::Headings, Kind::Heading),
            (Mode::Tags, Kind::Tag),
            (Mode::SearchShortcut, Kind::SearchShortcut),
            (Mode::Agent, Kind::Skill),
        ];

        for (mode, only) in modes {
            let admitted: Vec<Kind> = Kind::ALL
                .into_iter()
                .filter(|&kind| mode.admits(kind))
                .collect();

            assert_eq!(admitted, [only], "{mode:?}");
        }
    }

    #[test]
    fn no_mode_admits_the_question_as_typed() {
        for mode in MODES {
            assert!(!mode.admits(Kind::Question), "{mode:?}");
        }
    }
}

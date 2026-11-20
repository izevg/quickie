//! Skills: named recipes the agent follows, shaped like A2A's `AgentSkill` (id, name,
//! description, tags, examples) so they can be published later, plus what the agent needs to
//! run one: the instructions, the form of the answer, and which pages it reads.

use std::fmt;

/// Names a skill for good, in kebab-case: `summarize`, `extract-table`. It is what follows `?`
/// in the palette.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkillId(String);

impl SkillId {
    /// The id as typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SkillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A text that is not a skill id: kebab-case is lowercase ASCII letters and digits in words
/// joined by single hyphens, starting with a letter.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a kebab-case skill id")]
pub struct SkillIdError(pub String);

impl TryFrom<&str> for SkillId {
    type Error = SkillIdError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let words = text.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
        if words && text.starts_with(|c: char| c.is_ascii_lowercase()) {
            Ok(Self(text.to_owned()))
        } else {
            Err(SkillIdError(text.to_owned()))
        }
    }
}

/// The form an answer takes, which the model is asked for and the answer panel renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputForm {
    /// Short paragraphs.
    Prose,
    /// A bullet list.
    Bullets,
    /// Quotes from the page, each with what it supports.
    Citations,
    /// One Markdown table.
    Table,
}

impl OutputForm {
    /// How the model is told to shape its answer.
    #[must_use]
    pub const fn instruction(self) -> &'static str {
        match self {
            Self::Prose => "Answer in short paragraphs of plain prose.",
            Self::Bullets => "Answer as a short Markdown bullet list.",
            Self::Citations => {
                "Answer as a list of quotes from the page, each written as [[cite: exact words]] \
                 and followed by one line on what it supports."
            }
            Self::Table => {
                "Answer with one Markdown table with a header row; if the page has nothing to \
                 tabulate, say so in one line instead."
            }
        }
    }
}

/// Which pages a skill reads. Ask mode reads only what the user is looking at: the current
/// page, or, for a skill that compares, the open tabs the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageScope {
    /// The current page only.
    Current,
    /// Several open tabs, at least two.
    OpenTabs,
}

impl PageScope {
    /// The fewest pages the scope makes sense with.
    #[must_use]
    pub const fn min_pages(self) -> usize {
        match self {
            Self::Current => 1,
            Self::OpenTabs => 2,
        }
    }

    /// The most pages read; any beyond are left out, as each would get too small a share of
    /// the token budget to be worth reading.
    #[must_use]
    pub const fn max_pages(self) -> usize {
        match self {
            Self::Current => 1,
            Self::OpenTabs => 8,
        }
    }
}

/// A named recipe for the agent: what to do with the page, and how to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    id: SkillId,
    name: String,
    description: String,
    tags: Vec<String>,
    examples: Vec<String>,
    instructions: String,
    output: OutputForm,
    scope: PageScope,
}

impl Skill {
    /// The skills Quickie ships, in the order the palette lists them.
    #[must_use]
    pub fn builtin() -> Vec<Self> {
        vec![
            builtin(
                "summarize",
                "Summarize",
                "Sums up the page: its main point first, then the key facts.",
                &["summary", "reading"],
                &["Summarize this page", "TL;DR"],
                "Summarize the page: its main point first, then the key facts, in at most seven \
                 bullets.",
                OutputForm::Bullets,
                PageScope::Current,
            ),
            builtin(
                "citations",
                "Synthesize Page Citations",
                "Picks the passages that carry the page's claims, quoted exactly so each can be \
                 found on the page.",
                &["citations", "quotes", "sources"],
                &["What are the key quotes?", "Cite the evidence"],
                "Find the passages that carry the page's main claims and their evidence.",
                OutputForm::Citations,
                PageScope::Current,
            ),
            builtin(
                "extract-table",
                "Extract Table",
                "Turns the listings, prices, specifications, or schedules on the page into one \
                 table.",
                &["table", "data", "extract"],
                &["Make a table of the prices", "Extract the specs"],
                "Extract the structured data of the page, such as listings, prices, \
                 specifications, or schedules, into one table. Keep every value as the page \
                 writes it.",
                OutputForm::Table,
                PageScope::Current,
            ),
            builtin(
                "explain-code",
                "Explain Code",
                "Explains the code on the page: what it does, how, and what to watch out for.",
                &["code", "programming"],
                &["Explain this code", "What does this function do?"],
                "Explain the code on the page: what it does, how it works step by step, and its \
                 pitfalls. Quote the lines you talk about.",
                OutputForm::Prose,
                PageScope::Current,
            ),
            builtin(
                "compare-tabs",
                "Compare Tabs",
                "Compares the chosen tabs: what each covers, where they agree, where they differ.",
                &["compare", "tabs"],
                &["Compare these products", "How do these articles differ?"],
                "Compare the pages: what each covers, where they agree, and where they differ, \
                 with a column per page. Then name the most important difference in one line.",
                OutputForm::Table,
                PageScope::OpenTabs,
            ),
        ]
    }

    /// The built-in skill with this id, if Quickie ships one.
    #[must_use]
    pub fn builtin_named(id: &SkillId) -> Option<Self> {
        Self::builtin().into_iter().find(|skill| skill.id == *id)
    }

    /// The id, unique among skills.
    #[must_use]
    pub fn id(&self) -> &SkillId {
        &self.id
    }

    /// The name the palette shows.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// One sentence on what the skill does.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Words the palette also matches the skill by.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Requests the skill answers, as the user might type them.
    #[must_use]
    pub fn examples(&self) -> &[String] {
        &self.examples
    }

    /// What the model is asked to do, in place of a question.
    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// The form of the answer.
    #[must_use]
    pub fn output(&self) -> OutputForm {
        self.output
    }

    /// Which pages the skill reads.
    #[must_use]
    pub fn scope(&self) -> PageScope {
        self.scope
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one argument per field of the A2A skill shape, each named at the call site"
)]
fn builtin(
    id: &str,
    name: &str,
    description: &str,
    tags: &[&str],
    examples: &[&str],
    instructions: &str,
    output: OutputForm,
    scope: PageScope,
) -> Skill {
    let owned = |texts: &[&str]| texts.iter().map(|&text| text.to_owned()).collect();
    Skill {
        id: SkillId(id.to_owned()),
        name: name.to_owned(),
        description: description.to_owned(),
        tags: owned(tags),
        examples: owned(examples),
        instructions: instructions.to_owned(),
        output,
        scope,
    }
}

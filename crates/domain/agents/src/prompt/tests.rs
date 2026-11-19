//! Questions, skills, and requests here; cleaning, budgets, and assembly in [`assembly`].

mod assembly;

use url::Url;

use super::{
    OutputForm, PageScope, PageText, Question, QuestionError, Request, Role, Skill, SkillId,
    SkillIdError,
};

fn page(text: &str) -> PageText {
    PageText::new(
        Url::parse("https://example.com/article").unwrap(),
        "An article",
        text,
    )
}

fn question(text: &str) -> Request {
    Request::Question(Question::try_from(text).unwrap())
}

fn skill(id: &str) -> Skill {
    Skill::builtin_named(&SkillId::try_from(id).unwrap()).unwrap()
}

// ---- question ----------------------------------------------------------------------------------

#[test]
fn a_question_keeps_its_text_without_surrounding_whitespace() {
    let asked = Question::try_from("  What is this about?\n").unwrap();

    assert_eq!(asked.as_str(), "What is this about?");
}

#[test]
fn an_empty_question_is_refused() {
    let error = Question::try_from(" \t").unwrap_err();

    assert_eq!(error, QuestionError::Empty);
}

#[test]
fn a_question_of_the_longest_length_is_accepted() {
    let text = "?".repeat(Question::MAX_CHARS);

    assert!(Question::try_from(text.as_str()).is_ok());
}

#[test]
fn a_question_one_character_too_long_is_refused() {
    let text = "?".repeat(Question::MAX_CHARS + 1);

    let error = Question::try_from(text.as_str()).unwrap_err();

    assert_eq!(error, QuestionError::TooLong);
}

#[test]
fn question_length_counts_characters_not_bytes() {
    let text = "я".repeat(Question::MAX_CHARS);

    assert!(Question::try_from(text.as_str()).is_ok());
}

// ---- skills ------------------------------------------------------------------------------------

#[test]
fn the_builtin_skills_ship_in_palette_order() {
    let ids: Vec<String> = Skill::builtin()
        .iter()
        .map(|skill| skill.id().to_string())
        .collect();

    assert_eq!(
        ids,
        [
            "summarize",
            "citations",
            "extract-table",
            "explain-code",
            "compare-tabs"
        ]
    );
}

#[test]
fn every_builtin_skill_id_is_a_valid_skill_id() {
    let skills = Skill::builtin();

    for skill in &skills {
        assert_eq!(
            SkillId::try_from(skill.id().as_str()).as_ref(),
            Ok(skill.id())
        );
    }
}

#[test]
fn every_builtin_skill_has_the_a2a_fields_filled_in() {
    let skills = Skill::builtin();

    for skill in &skills {
        assert_ne!(skill.name(), "");
        assert_ne!(skill.description(), "");
        assert_ne!(skill.tags().len(), 0);
        assert_ne!(skill.examples().len(), 0);
        assert_ne!(skill.instructions(), "");
    }
}

#[test]
fn the_citations_skill_is_synthesize_page_citations() {
    let citations = skill("citations");

    assert_eq!(citations.name(), "Synthesize Page Citations");
    assert_eq!(
        citations.description(),
        "Picks the passages that carry the page's claims, quoted exactly so each can be found \
         on the page."
    );
    assert_eq!(
        citations.instructions(),
        "Find the passages that carry the page's main claims and their evidence."
    );
    assert_eq!(citations.output(), OutputForm::Citations);
    assert_eq!(citations.scope(), PageScope::Current);
    assert_eq!(citations.tags(), ["citations", "quotes", "sources"]);
    assert_eq!(
        citations.examples(),
        ["What are the key quotes?", "Cite the evidence"]
    );
}

#[test]
fn each_builtin_skill_answers_in_its_own_form() {
    let forms: Vec<OutputForm> = Skill::builtin().iter().map(Skill::output).collect();

    assert_eq!(
        forms,
        [
            OutputForm::Bullets,
            OutputForm::Citations,
            OutputForm::Table,
            OutputForm::Prose,
            OutputForm::Table
        ]
    );
}

#[test]
fn only_compare_tabs_reads_several_tabs() {
    let scopes: Vec<PageScope> = Skill::builtin().iter().map(Skill::scope).collect();

    assert_eq!(
        scopes,
        [
            PageScope::Current,
            PageScope::Current,
            PageScope::Current,
            PageScope::Current,
            PageScope::OpenTabs
        ]
    );
}

#[test]
fn an_unknown_skill_is_not_builtin() {
    let id = SkillId::try_from("translate").unwrap();

    assert_eq!(Skill::builtin_named(&id), None);
}

#[test]
fn a_kebab_case_skill_id_with_digits_is_valid() {
    let id = SkillId::try_from("extract-table2").unwrap();

    assert_eq!(id.as_str(), "extract-table2");
}

#[test]
fn skill_ids_that_are_not_kebab_case_are_refused() {
    let refused = [
        "",
        "Summarize",
        "2fast",
        "extract--table",
        "extract-",
        "-x",
        "a_b",
        "a b",
    ];

    for text in refused {
        assert_eq!(
            SkillId::try_from(text),
            Err(SkillIdError(text.to_owned())),
            "{text:?}"
        );
    }
}

#[test]
fn page_scopes_bound_how_many_pages_are_read() {
    assert_eq!(PageScope::Current.min_pages(), 1);
    assert_eq!(PageScope::Current.max_pages(), 1);
    assert_eq!(PageScope::OpenTabs.min_pages(), 2);
    assert_eq!(PageScope::OpenTabs.max_pages(), 8);
}

#[test]
fn each_output_form_has_its_own_instruction() {
    let instructions: Vec<&str> = [
        OutputForm::Prose,
        OutputForm::Bullets,
        OutputForm::Citations,
        OutputForm::Table,
    ]
    .into_iter()
    .map(OutputForm::instruction)
    .collect();

    assert!(instructions[0].contains("prose"));
    assert!(instructions[1].contains("bullet list"));
    assert!(instructions[2].contains("[[cite: exact words]]"));
    assert!(instructions[3].contains("Markdown table"));
}

// ---- requests ----------------------------------------------------------------------------------

#[test]
fn a_question_is_answered_in_prose_from_the_current_page() {
    let request = question("Who wrote this?");

    assert_eq!(request.instructions(), "Who wrote this?");
    assert_eq!(request.output(), OutputForm::Prose);
    assert_eq!(request.scope(), PageScope::Current);
}

#[test]
fn a_skill_request_follows_the_skill() {
    let compare = skill("compare-tabs");

    let request = Request::Skill(compare.clone());

    assert_eq!(request.instructions(), compare.instructions());
    assert_eq!(request.output(), OutputForm::Table);
    assert_eq!(request.scope(), PageScope::OpenTabs);
}

#[test]
fn roles_are_named_as_the_api_names_them() {
    assert_eq!(Role::System.as_str(), "system");
    assert_eq!(Role::User.as_str(), "user");
}

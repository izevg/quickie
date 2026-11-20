use quickie_domain_kernel::Timestamp;
use url::Url;

use super::citation::Sources;
use super::scan::{Piece, Scanner};
use super::{AgentSession, AnswerPart, Citation, Mode, Speed};
use crate::{FenceMarker, PageText, Prompt, Question, Request, TokenBudget};

const PAGE: &str = "The  quick brown fox\njumps over the lazy dog. Prices rose by 4% in May.";

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

/// The page text as a prompt sends it: cleaned, whitespace collapsed.
fn pages() -> Vec<String> {
    vec!["The quick brown fox\njumps over the lazy dog. Prices rose by 4% in May.".to_owned()]
}

fn prompt_about(text: &str, budget: TokenBudget) -> Prompt {
    let page = PageText::new(Url::parse("https://a.b/").unwrap(), "", text);
    let request = Request::Question(Question::try_from("What?").unwrap());
    Prompt::assemble(&request, &[page], budget, &FenceMarker::from_random([0; 6])).unwrap()
}

fn session() -> AgentSession {
    AgentSession::new(&prompt_about(PAGE, TokenBudget::DEFAULT), at(1_000))
}

fn text(text: &str) -> Piece {
    Piece::Text(text.to_owned())
}

fn quote(text: &str) -> Piece {
    Piece::Quote(text.to_owned())
}

/// Every piece of `tokens` scanned one after another, then finished.
fn scan(tokens: &[&str]) -> Vec<Piece> {
    let mut scanner = Scanner::default();
    let mut pieces: Vec<Piece> = tokens
        .iter()
        .flat_map(|token| scanner.push(token))
        .collect();
    pieces.extend(scanner.finish());
    pieces
}

// ---- scanning for quote marks ------------------------------------------------------------------

#[test]
fn plain_text_passes_through_at_once() {
    let mut scanner = Scanner::default();

    let pieces = scanner.push("Hello world");

    assert_eq!(pieces, [text("Hello world")]);
}

#[test]
fn a_whole_mark_in_one_token_is_a_quote_between_texts() {
    let pieces = scan(&["As it says, [[cite: the quick fox]], indeed."]);

    assert_eq!(
        pieces,
        [
            text("As it says, "),
            quote("the quick fox"),
            text(", indeed.")
        ]
    );
}

#[test]
fn a_mark_split_at_every_character_is_still_found() {
    let answer = "a [[cite: fox]] b";
    let tokens: Vec<String> = answer.chars().map(String::from).collect();
    let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();

    let pieces = scan(&tokens);

    let quotes: Vec<&Piece> = pieces
        .iter()
        .filter(|p| matches!(p, Piece::Quote(_)))
        .collect();
    assert_eq!(quotes, [&quote("fox")]);
    let texts: String = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Text(t) => Some(t.as_str()),
            Piece::Quote(_) => None,
        })
        .collect();
    assert_eq!(texts, "a  b");
}

#[test]
fn the_start_of_a_mark_is_held_until_the_next_token() {
    let mut scanner = Scanner::default();

    let first = scanner.push("see [[ci");
    let second = scanner.push("te: fox]]");

    assert_eq!(first, [text("see ")]);
    assert_eq!(second, [quote("fox")]);
}

#[test]
fn a_lone_bracket_at_the_end_is_held_then_released() {
    let mut scanner = Scanner::default();

    let first = scanner.push("array[");
    let second = scanner.push("0]");

    assert_eq!(first, [text("array")]);
    assert_eq!(second, [text("[0]")]);
}

#[test]
fn a_held_start_that_never_becomes_a_mark_is_text_at_the_end() {
    let pieces = scan(&["see [[cit"]);

    assert_eq!(pieces, [text("see "), text("[[cit")]);
}

#[test]
fn an_unclosed_mark_is_text_at_the_end() {
    let pieces = scan(&["see [[cite: the fox"]);

    assert_eq!(pieces, [text("see "), text("[[cite: the fox")]);
}

#[test]
fn an_empty_mark_vanishes() {
    let pieces = scan(&["a[[cite:  ]]b"]);

    assert_eq!(pieces, [text("a"), text("b")]);
}

#[test]
fn two_marks_in_a_row_are_two_quotes() {
    let pieces = scan(&["[[cite: one]][[cite: two]]"]);

    assert_eq!(pieces, [quote("one"), quote("two")]);
}

#[test]
fn a_mark_is_closed_by_its_first_closing_brackets() {
    let pieces = scan(&["[[cite: a]] b]]"]);

    assert_eq!(pieces, [quote("a"), text(" b]]")]);
}

#[test]
fn a_quote_of_the_longest_length_is_still_waited_for() {
    let mut scanner = Scanner::default();
    let long = "x".repeat(2_000);

    let pieces = scanner.push(&format!("[[cite:{long}"));
    let closed = scanner.push("]]");

    assert_eq!(pieces, []);
    assert_eq!(closed, [quote(&long)]);
}

#[test]
fn a_quote_one_byte_too_long_is_given_up_as_text() {
    let mut scanner = Scanner::default();
    let long = "x".repeat(2_001);

    let pieces = scanner.push(&format!("[[cite:{long}"));
    let after = scanner.push("]] more");

    assert_eq!(pieces, [text(&format!("[[cite:{long}"))]);
    assert_eq!(after, [text("]] more")]);
}

#[test]
fn nothing_held_finishes_with_nothing() {
    let scanner = Scanner::default();

    assert_eq!(scanner.finish(), None);
}

#[test]
fn a_mark_with_multibyte_text_is_split_safely() {
    let pieces = scan(&["«[[cite: ", "Цена выросла", "]]» ✓"]);

    assert_eq!(pieces, [text("«"), quote("Цена выросла"), text("» ✓")]);
}

// ---- citations ---------------------------------------------------------------------------------

fn sources(texts: &[&str]) -> Sources {
    let texts: Vec<String> = texts.iter().map(|&text| text.to_owned()).collect();
    Sources::of(&texts)
}

#[test]
fn a_quote_the_page_contains_is_a_citation_anchored_by_its_first_unique_words() {
    let sources = Sources::of(&pages());

    let citation = sources.cite("quick brown fox jumps");

    assert_eq!(
        citation,
        Some(Citation {
            quote: "quick brown fox jumps".to_owned(),
            anchor: "quick brown fox".to_owned(),
        })
    );
}

#[test]
fn a_quote_matches_across_case_and_whitespace() {
    let sources = Sources::of(&pages());

    let citation = sources.cite("THE QUICK\n brown   fox");

    assert_eq!(
        citation.map(|citation| citation.anchor),
        Some("THE QUICK brown".to_owned())
    );
}

#[test]
fn quotation_marks_around_a_quote_are_dropped() {
    let sources = Sources::of(&pages());

    let citation = sources.cite("“over the lazy dog.”");

    assert_eq!(
        citation.map(|citation| citation.quote),
        Some("over the lazy dog.".to_owned())
    );
}

#[test]
fn a_single_letter_is_no_citation() {
    let citation = sources(&["abc"]).cite("a");

    assert_eq!(citation, None);
}

#[test]
fn two_long_words_are_no_citation() {
    let found = sources(&["supercalifragilistic expialidocious word"]);

    assert_eq!(found.cite("supercalifragilistic expialidocious"), None);
    assert!(
        found
            .cite("supercalifragilistic expialidocious word")
            .is_some()
    );
}

#[test]
fn three_words_need_fifteen_characters() {
    let found = sources(&["aaaa bbbb ccccc d"]);

    assert_eq!(found.cite("aaaa bbbb cccc"), None);
    assert!(found.cite("aaaa bbbb ccccc").is_some());
}

#[test]
fn an_anchor_grows_until_it_occurs_only_once() {
    let found = sources(&["the cat sat here. The cat sat there and slept."]);

    let citation = found.cite("the cat sat there and slept").unwrap();

    assert_eq!(citation.anchor, "the cat sat there");
}

#[test]
fn a_quote_with_no_unique_start_is_its_own_anchor() {
    let found = sources(&["one two three four. one two three four."]);

    let citation = found.cite("one two three four").unwrap();

    assert_eq!(citation.anchor, "one two three four");
}

#[test]
fn a_quote_found_only_past_what_was_sent_is_no_citation() {
    let mut session = AgentSession::new(&prompt_about(PAGE, TokenBudget::new(10)), at(1_000));

    let parts = session.received("[[cite: Prices rose by 4% in May]]", at(1_100));

    assert_eq!(
        parts,
        [AnswerPart::Text("“Prices rose by 4% in May”".to_owned())]
    );
}

#[test]
fn a_quote_the_page_lacks_is_no_citation() {
    let sources = Sources::of(&pages());

    let citation = sources.cite("the slow red fox");

    assert_eq!(citation, None);
}

#[test]
fn a_quote_of_only_quotation_marks_is_no_citation() {
    let sources = Sources::of(&pages());

    let citation = sources.cite("“”");

    assert_eq!(citation, None);
}

#[test]
fn a_quote_from_a_second_page_is_a_citation() {
    let mut both = pages();
    both.push("Another tab entirely".to_owned());
    let sources = Sources::of(&both);

    let citation = sources.cite("another tab entirely");

    assert!(citation.is_some());
}

// ---- the session -------------------------------------------------------------------------------

#[test]
fn a_session_works_in_ask_mode() {
    assert_eq!(session().mode(), Mode::Ask);
}

#[test]
fn tokens_become_text_parts_and_the_answer_grows() {
    let mut session = session();

    let first = session.received("The page ", at(1_100));
    let second = session.received("is short.", at(1_150));

    assert_eq!(first, [AnswerPart::Text("The page ".to_owned())]);
    assert_eq!(second, [AnswerPart::Text("is short.".to_owned())]);
    assert_eq!(session.answer(), "The page is short.");
}

#[test]
fn a_located_quote_becomes_a_citation_part() {
    let mut session = session();

    let parts = session.received("It says [[cite: Prices rose by 4%]].", at(1_100));

    let citation = Citation {
        quote: "Prices rose by 4%".to_owned(),
        anchor: "Prices rose by".to_owned(),
    };
    assert_eq!(
        parts,
        [
            AnswerPart::Text("It says ".to_owned()),
            AnswerPart::Citation(citation.clone()),
            AnswerPart::Text(".".to_owned()),
        ]
    );
    assert_eq!(session.citations(), [citation]);
    assert_eq!(session.answer(), "It says “Prices rose by 4%”.");
}

#[test]
fn a_quote_the_pages_lack_stays_quoted_text() {
    let mut session = session();

    let parts = session.received("[[cite: Prices fell]]", at(1_100));

    assert_eq!(parts, [AnswerPart::Text("“Prices fell”".to_owned())]);
    assert_eq!(session.citations(), []);
    assert_eq!(session.answer(), "“Prices fell”");
}

#[test]
fn an_empty_token_counts_for_nothing() {
    let mut session = session();

    let parts = session.received("", at(1_100));
    let finished = session.finish(at(2_000), None);

    assert_eq!(parts, []);
    assert_eq!(finished.speed.ttft_ms, None);
}

#[test]
fn time_to_first_token_runs_from_the_question_to_the_first_token() {
    let mut session = session();
    session.received("a", at(1_400));
    session.received("b", at(1_900));

    let finished = session.finish(at(2_000), None);

    assert_eq!(finished.speed.ttft_ms, Some(400));
}

#[test]
fn speed_counts_chunks_from_the_first_token_to_the_end() {
    let mut session = session();
    for millis in [1_400, 1_450, 1_500, 1_550, 1_600] {
        session.received("t", at(millis));
    }

    let finished = session.finish(at(1_650), None);

    assert_eq!(
        finished.speed,
        Speed {
            ttft_ms: Some(400),
            tokens_per_s: Some(20.0),
        }
    );
}

#[test]
fn the_servers_own_token_count_wins_over_chunks() {
    let mut session = session();
    session.received("several tokens at once", at(1_000));

    let finished = session.finish(at(1_500), Some(12));

    assert_eq!(finished.speed.tokens_per_s, Some(24.0));
}

#[test]
fn an_answer_that_ends_with_its_first_token_has_no_rate() {
    let mut session = session();
    session.received("all", at(1_300));

    let finished = session.finish(at(1_300), None);

    assert_eq!(
        finished.speed,
        Speed {
            ttft_ms: Some(300),
            tokens_per_s: None,
        }
    );
}

#[test]
fn an_answer_one_millisecond_long_has_a_rate() {
    let mut session = session();
    session.received("x", at(1_300));

    let finished = session.finish(at(1_301), None);

    assert_eq!(finished.speed.tokens_per_s, Some(1_000.0));
}

#[test]
fn finishing_releases_what_was_held_back() {
    let mut session = session();
    session.received("see [[cite: the fox", at(1_100));

    let finished = session.finish(at(1_200), None);

    assert_eq!(
        finished.parts,
        [AnswerPart::Text("[[cite: the fox".to_owned())]
    );
    assert_eq!(finished.answer, "see [[cite: the fox");
}

#[test]
fn finishing_hands_over_the_answer_and_its_citations() {
    let mut session = session();
    session.received("[[cite: over the lazy dog]] ok", at(1_100));

    let finished = session.finish(at(1_200), None);

    assert_eq!(finished.parts, []);
    assert_eq!(finished.answer, "“over the lazy dog” ok");
    assert_eq!(
        finished.citations,
        [Citation {
            quote: "over the lazy dog".to_owned(),
            anchor: "over the lazy".to_owned(),
        }]
    );
}

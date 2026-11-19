use quickie_domain_agents::{AnswerPart, Citation, KeyRef, Speed};

use super::fakes::{FakeAgentRepo, FakeLlm, FakeSecrets};
use super::{hosted, page, question};
use crate::agent::{Agent, AnswerEvent, AskError, Asking};
use crate::fakes::FakeClock;
use crate::{ChatEvent, HttpError, LlmError, SecretError, StopSignal};

const PAGE: &str = "Prices rose by 4% in May.";

fn delta(text: &str) -> ChatEvent {
    ChatEvent::Delta(text.to_owned())
}

fn token(text: &str) -> AnswerEvent {
    AnswerEvent::Token(text.to_owned())
}

/// Asking about [`PAGE`] at the hosted endpoint, with or without a key.
fn asking(with_key: bool) -> Asking {
    let endpoint = if with_key {
        hosted().with_key()
    } else {
        hosted()
    };
    let repo = FakeAgentRepo {
        saved: Some(endpoint.snapshot()),
        ..FakeAgentRepo::default()
    };
    let agent = Agent::start(repo, FakeSecrets::default()).unwrap();
    agent
        .ask(&question("What happened?"), vec![page(PAGE)])
        .unwrap()
}

/// Runs `asking` against `llm` and collects every event.
fn run(
    asking: Asking,
    llm: &FakeLlm,
    secrets: &FakeSecrets,
    stop: &StopSignal,
) -> Vec<AnswerEvent> {
    let mut events = Vec::new();
    asking.run(llm, secrets, &llm.clock, stop, &mut |event| {
        events.push(event);
    });
    events
}

fn answer(llm: &FakeLlm) -> Vec<AnswerEvent> {
    run(
        asking(false),
        llm,
        &FakeSecrets::default(),
        &StopSignal::new(),
    )
}

/// A fake endpoint asked at 1000 ms that plays `script`.
fn llm(script: Vec<(u64, ChatEvent)>) -> FakeLlm {
    let clock = FakeClock::default();
    clock.0.set(1_000);
    let mut llm = FakeLlm::new(&clock);
    llm.script = script;
    llm
}

#[test]
fn tokens_stream_out_then_the_answer_is_done_with_its_speed() {
    let llm = llm(vec![(1_300, delta("Prices ")), (1_400, delta("rose."))]);

    let events = answer(&llm);

    assert_eq!(
        events,
        [
            token("Prices "),
            token("rose."),
            AnswerEvent::Done(Speed {
                ttft_ms: Some(300),
                tokens_per_s: Some(20.0),
            }),
        ]
    );
}

#[test]
fn a_quote_the_page_contains_streams_out_as_a_citation() {
    let llm = llm(vec![(1_100, delta("[[cite: rose by 4%]]"))]);

    let events = answer(&llm);

    assert_eq!(
        events[0],
        AnswerEvent::Citation(Citation {
            quote: "rose by 4%".to_owned(),
            anchor: "rose by 4%".to_owned(),
        })
    );
}

#[test]
fn the_servers_token_count_sets_the_speed() {
    let llm = llm(vec![
        (1_100, delta("Everything at once.")),
        (
            1_600,
            ChatEvent::Usage {
                completion_tokens: 10,
            },
        ),
    ]);

    let events = answer(&llm);

    assert_eq!(
        events.last(),
        Some(&AnswerEvent::Done(Speed {
            ttft_ms: Some(100),
            tokens_per_s: Some(20.0),
        }))
    );
}

#[test]
fn a_stopped_answer_releases_held_text_then_says_stopped() {
    let llm = llm(vec![(1_100, delta("See [[cite: unfinished"))]);
    let stop = StopSignal::new();
    stop.stop();

    let events = run(asking(false), &llm, &FakeSecrets::default(), &stop);

    assert_eq!(
        events,
        [
            token("See "),
            token("[[cite: unfinished"),
            AnswerEvent::Stopped
        ]
    );
}

#[test]
fn an_endpoint_that_fails_mid_answer_keeps_what_came() {
    let mut llm = llm(vec![(1_100, delta("Partial"))]);
    llm.ended = Err(LlmError::Api("overloaded".to_owned()));

    let events = answer(&llm);

    assert_eq!(
        events,
        [
            token("Partial"),
            AnswerEvent::Error(AskError::Llm(LlmError::Api("overloaded".to_owned())))
        ]
    );
}

#[test]
fn the_endpoint_gets_the_prompt_and_model_without_a_key() {
    let llm = llm(Vec::new());
    let asking = asking(false);
    let prompt = asking.prompt().clone();

    run(asking, &llm, &FakeSecrets::default(), &StopSignal::new());

    let asked = llm.asked.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].model, "small");
    assert_eq!(asked[0].key, None);
    assert_eq!(asked[0].prompt, prompt);
}

#[test]
fn an_endpoint_without_a_key_never_reads_the_keychain() {
    let llm = llm(Vec::new());
    let secrets = FakeSecrets::default();

    run(asking(false), &llm, &secrets, &StopSignal::new());

    assert_eq!(secrets.reads.get(), 0);
}

#[test]
fn the_key_comes_from_the_keychain_for_the_endpoint() {
    let llm = llm(Vec::new());
    let secrets = FakeSecrets::holding(&KeyRef::for_origin(hosted().base()), "sk-live");

    run(asking(true), &llm, &secrets, &StopSignal::new());

    assert_eq!(llm.asked.borrow()[0].key, Some("sk-live".to_owned()));
}

#[test]
fn a_key_missing_from_the_keychain_fails_before_asking() {
    let llm = llm(Vec::new());

    let events = run(
        asking(true),
        &llm,
        &FakeSecrets::default(),
        &StopSignal::new(),
    );

    assert_eq!(
        events,
        [AnswerEvent::Error(AskError::KeyMissing(
            KeyRef::for_origin(hosted().base())
        ))]
    );
    assert_eq!(llm.asked.borrow().len(), 0);
}

#[test]
fn a_keychain_that_cannot_be_read_fails_before_asking() {
    let llm = llm(Vec::new());
    let secrets = FakeSecrets {
        read_fails: true,
        ..FakeSecrets::default()
    };

    let events = run(asking(true), &llm, &secrets, &StopSignal::new());

    assert_eq!(
        events,
        [AnswerEvent::Error(AskError::Secret(SecretError(
            "read".to_owned()
        )))]
    );
    assert_eq!(llm.asked.borrow().len(), 0);
}

#[test]
fn an_answer_without_tokens_is_done_without_a_speed() {
    let llm = llm(Vec::new());

    let events = answer(&llm);

    assert_eq!(
        events,
        [AnswerEvent::Done(Speed {
            ttft_ms: None,
            tokens_per_s: None,
        })]
    );
}

#[test]
fn answer_parts_become_events() {
    let citation = Citation {
        quote: "q".to_owned(),
        anchor: "q".to_owned(),
    };

    assert_eq!(
        AnswerEvent::from(AnswerPart::Text("t".to_owned())),
        token("t")
    );
    assert_eq!(
        AnswerEvent::from(AnswerPart::Citation(citation.clone())),
        AnswerEvent::Citation(citation)
    );
}

#[test]
fn a_stop_signal_is_shared_by_its_clones() {
    let signal = StopSignal::new();
    let worker = signal.clone();

    let before = worker.is_stopped();
    signal.stop();

    assert!(!before);
    assert!(worker.is_stopped());
}

#[test]
fn ask_errors_explain_themselves() {
    let missing = AskError::KeyMissing(KeyRef::for_origin(hosted().base()));
    let failed = AskError::Llm(LlmError::Http(HttpError("refused".to_owned())));

    assert_eq!(
        missing.to_string(),
        "the Keychain has no key for https://api.example.com; set it again in settings"
    );
    assert_eq!(failed.to_string(), "http: refused");
}

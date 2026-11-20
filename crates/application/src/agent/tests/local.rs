use quickie_domain_agents::{KeyRef, LocalServer};

use super::fakes::{FakeLlm, FakeSecrets};
use super::{base, hosted, model};
use crate::agent::{AskError, LocalFound, detect_local, list_models};
use crate::fakes::FakeClock;
use crate::{HttpError, LlmError};

fn llm() -> FakeLlm {
    FakeLlm::new(&FakeClock::default())
}

#[test]
fn both_local_servers_are_found_when_both_answer() {
    let mut llm = llm();
    llm.servers = vec![
        (LocalServer::LmStudio, vec![model("qwen2.5-7b-instruct")]),
        (
            LocalServer::Ollama,
            vec![model("llama3.1:8b"), model("phi3")],
        ),
    ];

    let found = detect_local(&llm);

    assert_eq!(
        found,
        [
            LocalFound {
                server: LocalServer::Ollama,
                base: base("http://127.0.0.1:11434/v1"),
                models: vec![model("llama3.1:8b"), model("phi3")],
            },
            LocalFound {
                server: LocalServer::LmStudio,
                base: base("http://127.0.0.1:1234/v1"),
                models: vec![model("qwen2.5-7b-instruct")],
            },
        ]
    );
}

#[test]
fn a_server_that_does_not_answer_is_left_out() {
    let mut llm = llm();
    llm.servers = vec![(LocalServer::LmStudio, Vec::new())];

    let found = detect_local(&llm);

    assert_eq!(
        found.iter().map(|found| found.server).collect::<Vec<_>>(),
        [LocalServer::LmStudio]
    );
}

#[test]
fn nothing_is_found_when_nothing_answers() {
    let found = detect_local(&llm());

    assert_eq!(found, []);
}

#[test]
fn models_are_listed_with_the_endpoints_key() {
    let mut llm = llm();
    llm.models = Ok(vec![model("small"), model("large")]);
    let endpoint = hosted().with_key();
    let secrets = FakeSecrets::holding(endpoint.key().unwrap(), "sk-live");

    let models = list_models(&llm, &secrets, &endpoint);

    assert_eq!(models, Ok(vec![model("small"), model("large")]));
    assert_eq!(
        llm.models_asked.borrow().as_slice(),
        [(hosted().base().clone(), Some("sk-live".to_owned()))]
    );
}

#[test]
fn models_of_an_endpoint_without_a_key_are_listed_without_one() {
    let llm = llm();

    let models = list_models(&llm, &FakeSecrets::default(), &hosted());

    assert_eq!(models, Ok(Vec::new()));
    assert_eq!(
        llm.models_asked.borrow().as_slice(),
        [(hosted().base().clone(), None)]
    );
}

#[test]
fn listing_models_without_the_key_in_the_keychain_fails() {
    let llm = llm();
    let endpoint = hosted().with_key();

    let error = list_models(&llm, &FakeSecrets::default(), &endpoint).unwrap_err();

    assert_eq!(
        error,
        AskError::KeyMissing(KeyRef::for_origin(endpoint.base()))
    );
    assert_eq!(llm.models_asked.borrow().len(), 0);
}

#[test]
fn an_endpoint_that_fails_to_list_models_is_reported() {
    let mut llm = llm();
    llm.models = Err(LlmError::Http(HttpError("refused".to_owned())));

    let error = list_models(&llm, &FakeSecrets::default(), &hosted()).unwrap_err();

    assert_eq!(
        error,
        AskError::Llm(LlmError::Http(HttpError("refused".to_owned())))
    );
}

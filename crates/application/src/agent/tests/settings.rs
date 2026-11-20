use quickie_domain_agents::{
    CorruptEndpoint, EndpointSnapshot, FenceMarker, KeyRef, ModelIdError, Prompt, PromptError,
    TokenBudget,
};

use super::fakes::{FakeAgentRepo, FakeEntropy, FakeSecrets};
use super::{base, hosted, key, model, page, question};
use crate::agent::Agent;
use crate::{AppError, SecretError, StoreError};

type Subject = Agent<FakeAgentRepo, FakeSecrets, FakeEntropy>;

fn fresh() -> Subject {
    Agent::start(
        FakeAgentRepo::default(),
        FakeSecrets::default(),
        FakeEntropy::default(),
    )
    .unwrap()
}

/// An agent whose hosted endpoint has a key in the Keychain.
fn keyed() -> Subject {
    let endpoint = hosted().with_key();
    let reference = endpoint.key().unwrap().clone();
    let repo = FakeAgentRepo {
        saved: Some(endpoint.snapshot()),
        ..FakeAgentRepo::default()
    };
    Agent::start(
        repo,
        FakeSecrets::holding(&reference, "sk-old"),
        FakeEntropy::default(),
    )
    .unwrap()
}

fn reference() -> KeyRef {
    KeyRef::for_origin(hosted().base())
}

// ---- start -------------------------------------------------------------------------------------

#[test]
fn a_fresh_profile_has_no_endpoint() {
    let agent = fresh();

    assert_eq!(agent.endpoint(), None);
    assert_eq!(agent.corrupt_on_start(), None);
}

#[test]
fn the_saved_endpoint_comes_back() {
    let agent = keyed();

    assert_eq!(agent.endpoint(), Some(&hosted().with_key()));
    assert_eq!(agent.corrupt_on_start(), None);
}

#[test]
fn a_corrupt_saved_endpoint_is_dropped_and_reported() {
    let repo = FakeAgentRepo {
        saved: Some(EndpointSnapshot {
            model: " ".to_owned(),
            ..hosted().snapshot()
        }),
        ..FakeAgentRepo::default()
    };

    let agent = Agent::start(repo, FakeSecrets::default(), FakeEntropy::default()).unwrap();

    assert_eq!(agent.endpoint(), None);
    assert_eq!(
        agent.corrupt_on_start(),
        Some(&CorruptEndpoint::Model(ModelIdError::Empty))
    );
}

#[test]
fn a_store_that_cannot_be_read_fails_the_start() {
    let repo = FakeAgentRepo {
        load_fails: true,
        ..FakeAgentRepo::default()
    };

    let error = Agent::start(repo, FakeSecrets::default(), FakeEntropy::default()).unwrap_err();

    assert_eq!(error, AppError::Store(StoreError("load".to_owned())));
}

// ---- endpoint ----------------------------------------------------------------------------------

#[test]
fn setting_up_an_endpoint_saves_it() {
    let mut agent = fresh();

    agent
        .set_endpoint(base("http://127.0.0.1:11434/v1"), model("llama3.1:8b"))
        .unwrap();

    let endpoint = agent.endpoint().unwrap();
    assert_eq!(endpoint.base().as_str(), "http://127.0.0.1:11434/v1/");
    assert_eq!(endpoint.model().as_str(), "llama3.1:8b");
    assert_eq!(agent.repo.saved, Some(endpoint.snapshot()));
}

#[test]
fn a_failed_save_leaves_the_endpoint_as_it_was() {
    let mut agent = keyed();
    agent.repo.save_fails = true;

    let result = agent.set_endpoint(base("https://other.example.com/v1"), model("big"));

    assert_eq!(result, Err(AppError::Store(StoreError("save".to_owned()))));
    assert_eq!(agent.endpoint(), Some(&hosted().with_key()));
}

#[test]
fn another_model_on_the_same_origin_keeps_the_key() {
    let mut agent = keyed();

    agent
        .set_endpoint(base("https://api.example.com/v1"), model("large"))
        .unwrap();

    assert_eq!(agent.endpoint().unwrap().key(), Some(&reference()));
    assert_eq!(agent.endpoint().unwrap().model().as_str(), "large");
}

#[test]
fn another_origin_drops_the_key_but_leaves_it_in_the_keychain() {
    let mut agent = keyed();

    agent
        .set_endpoint(base("https://other.example.com/v1"), model("small"))
        .unwrap();

    assert_eq!(agent.endpoint().unwrap().key(), None);
    assert_eq!(agent.secrets.get(&reference()), Some("sk-old".to_owned()));
}

// ---- key ---------------------------------------------------------------------------------------

#[test]
fn setting_a_key_keeps_it_in_the_keychain_and_only_its_reference_in_the_store() {
    let mut agent = fresh();
    agent
        .set_endpoint(hosted().base().clone(), model("small"))
        .unwrap();

    agent.set_key(&key("sk-new")).unwrap();

    assert_eq!(agent.secrets.get(&reference()), Some("sk-new".to_owned()));
    let saved = agent.repo.saved.clone().unwrap();
    assert_eq!(saved.key_ref, Some("https://api.example.com".to_owned()));
    assert_eq!(agent.endpoint().unwrap().key(), Some(&reference()));
}

#[test]
fn setting_a_key_replaces_the_one_there() {
    let mut agent = keyed();

    agent.set_key(&key("sk-new")).unwrap();

    assert_eq!(agent.secrets.get(&reference()), Some("sk-new".to_owned()));
}

#[test]
fn a_key_needs_an_endpoint() {
    let mut agent = fresh();

    let result = agent.set_key(&key("sk-new"));

    assert_eq!(result, Err(AppError::NoEndpoint));
    assert_eq!(agent.secrets.keys.borrow().len(), 0);
}

#[test]
fn a_keychain_that_refuses_the_key_changes_nothing() {
    let mut agent = fresh();
    agent
        .set_endpoint(hosted().base().clone(), model("small"))
        .unwrap();
    agent.secrets.write_fails = true;

    let result = agent.set_key(&key("sk-new"));

    assert_eq!(
        result,
        Err(AppError::Secret(SecretError("write".to_owned())))
    );
    assert_eq!(agent.endpoint(), Some(&hosted()));
    assert_eq!(agent.repo.saves, 1);
}

#[test]
fn a_failed_save_of_the_key_leaves_the_endpoint_without_it() {
    let mut agent = fresh();
    agent
        .set_endpoint(hosted().base().clone(), model("small"))
        .unwrap();
    agent.repo.save_fails = true;

    let result = agent.set_key(&key("sk-new"));

    assert_eq!(result, Err(AppError::Store(StoreError("save".to_owned()))));
    assert_eq!(agent.endpoint(), Some(&hosted()));
}

#[test]
fn removing_the_key_drops_it_from_settings_and_keychain() {
    let mut agent = keyed();

    agent.remove_key().unwrap();

    assert_eq!(agent.endpoint(), Some(&hosted()));
    assert_eq!(agent.repo.saved, Some(hosted().snapshot()));
    assert_eq!(agent.secrets.get(&reference()), None);
}

#[test]
fn removing_a_key_that_is_not_there_saves_nothing() {
    let mut agent = fresh();
    agent
        .set_endpoint(hosted().base().clone(), model("small"))
        .unwrap();

    agent.remove_key().unwrap();

    assert_eq!(agent.repo.saves, 1);
}

#[test]
fn removing_the_key_without_an_endpoint_does_nothing() {
    let mut agent = fresh();

    let result = agent.remove_key();

    assert_eq!(result, Ok(()));
    assert_eq!(agent.repo.saves, 0);
}

#[test]
fn a_failed_save_keeps_the_key_everywhere() {
    let mut agent = keyed();
    agent.repo.save_fails = true;

    let result = agent.remove_key();

    assert_eq!(result, Err(AppError::Store(StoreError("save".to_owned()))));
    assert_eq!(agent.endpoint(), Some(&hosted().with_key()));
    assert_eq!(agent.secrets.get(&reference()), Some("sk-old".to_owned()));
}

#[test]
fn a_keychain_that_cannot_delete_is_reported_after_the_settings_dropped_the_key() {
    let mut agent = keyed();
    agent.secrets.delete_fails = true;

    let result = agent.remove_key();

    assert_eq!(
        result,
        Err(AppError::Secret(SecretError("delete".to_owned())))
    );
    assert_eq!(agent.endpoint(), Some(&hosted()));
}

// ---- forgetting --------------------------------------------------------------------------------

#[test]
fn forgetting_the_endpoint_deletes_its_key() {
    let mut agent = keyed();

    agent.forget_endpoint().unwrap();

    assert_eq!(agent.endpoint(), None);
    assert_eq!(agent.repo.saved, None);
    assert_eq!(agent.repo.saves, 1);
    assert_eq!(agent.secrets.get(&reference()), None);
}

#[test]
fn forgetting_an_endpoint_without_a_key_leaves_the_keychain_alone() {
    let mut agent = fresh();
    agent
        .set_endpoint(hosted().base().clone(), model("small"))
        .unwrap();
    agent.secrets.delete_fails = true;

    let result = agent.forget_endpoint();

    assert_eq!(result, Ok(()));
    assert_eq!(agent.endpoint(), None);
}

#[test]
fn a_failed_save_keeps_the_endpoint_and_its_key() {
    let mut agent = keyed();
    agent.repo.save_fails = true;

    let result = agent.forget_endpoint();

    assert_eq!(result, Err(AppError::Store(StoreError("save".to_owned()))));
    assert_eq!(agent.endpoint(), Some(&hosted().with_key()));
    assert_eq!(agent.secrets.get(&reference()), Some("sk-old".to_owned()));
}

#[test]
fn a_keychain_that_cannot_delete_is_reported_after_forgetting() {
    let mut agent = keyed();
    agent.secrets.delete_fails = true;

    let result = agent.forget_endpoint();

    assert_eq!(
        result,
        Err(AppError::Secret(SecretError("delete".to_owned())))
    );
    assert_eq!(agent.endpoint(), None);
}

// ---- asking ------------------------------------------------------------------------------------

#[test]
fn asking_needs_an_endpoint() {
    let agent = fresh();

    let error = agent
        .ask(&question("Why?"), &[page("Because.")])
        .unwrap_err();

    assert_eq!(error, AppError::NoEndpoint);
}

#[test]
fn asking_needs_a_page() {
    let agent = keyed();

    let error = agent.ask(&question("Why?"), &[]).unwrap_err();

    assert_eq!(error, AppError::Prompt(PromptError::NoPage));
}

#[test]
fn asking_prepares_the_prompt_for_the_endpoint_with_a_fresh_marker() {
    let agent = keyed();
    let request = question("Why?");

    let asking = agent.ask(&request, &[page("Because.")]).unwrap();

    assert_eq!(asking.endpoint(), &hosted().with_key());
    let marker = FenceMarker::from_random([1; 6]);
    assert_eq!(
        asking.prompt(),
        &Prompt::assemble(&request, &[page("Because.")], TokenBudget::DEFAULT, &marker).unwrap()
    );
}

#[test]
fn each_request_is_fenced_with_its_own_marker() {
    let agent = keyed();
    let request = question("Why?");

    let first = agent.ask(&request, &[page("Because.")]).unwrap();
    let second = agent.ask(&request, &[page("Because.")]).unwrap();

    assert_ne!(first.prompt(), second.prompt());
}

#[test]
fn asking_reaches_neither_keychain_nor_store() {
    let agent = keyed();

    let _ = agent.ask(&question("Why?"), &[page("Because.")]);

    assert_eq!(agent.secrets.reads.get(), 0);
    assert_eq!(agent.repo.saves, 0);
}

use super::{
    ApiKey, ApiKeyError, BaseUrl, BaseUrlError, CorruptEndpoint, EndpointSnapshot, KeyRef,
    LlmEndpoint, LocalServer, ModelId, ModelIdError,
};

fn base(text: &str) -> BaseUrl {
    BaseUrl::try_from(text).unwrap()
}

fn model(text: &str) -> ModelId {
    ModelId::try_from(text).unwrap()
}

fn openai() -> LlmEndpoint {
    LlmEndpoint::new(base("https://api.openai.com/v1"), model("gpt-4o-mini"))
}

// ---- base URL ----------------------------------------------------------------------------------

#[test]
fn an_https_base_gains_a_trailing_slash() {
    let url = base("https://api.openai.com/v1");

    assert_eq!(url.as_str(), "https://api.openai.com/v1/");
    assert_eq!(url.to_string(), "https://api.openai.com/v1/");
    assert_eq!(url.as_url().as_str(), "https://api.openai.com/v1/");
}

#[test]
fn a_base_with_its_trailing_slash_keeps_one() {
    let url = base("https://api.openai.com/v1/");

    assert_eq!(url.as_str(), "https://api.openai.com/v1/");
}

#[test]
fn whitespace_around_a_base_is_ignored() {
    let url = base("  https://api.openai.com/v1 \n");

    assert_eq!(url.as_str(), "https://api.openai.com/v1/");
}

#[test]
fn chat_completions_and_models_hang_off_the_base() {
    let url = base("https://openrouter.ai/api/v1");

    assert_eq!(
        url.chat_completions().as_str(),
        "https://openrouter.ai/api/v1/chat/completions"
    );
    assert_eq!(url.models().as_str(), "https://openrouter.ai/api/v1/models");
}

#[test]
fn a_bare_host_has_its_paths_at_the_root() {
    let url = base("https://llm.example.com");

    assert_eq!(
        url.chat_completions().as_str(),
        "https://llm.example.com/chat/completions"
    );
}

#[test]
fn plain_http_is_allowed_on_ipv4_loopback() {
    let url = base("http://127.0.0.1:11434/v1");

    assert!(url.is_loopback());
}

#[test]
fn plain_http_is_allowed_anywhere_in_127_0_0_0_8() {
    let url = BaseUrl::try_from("http://127.1.2.3:8080/v1");

    assert!(url.is_ok());
}

#[test]
fn plain_http_is_allowed_on_ipv6_loopback() {
    let url = base("http://[::1]:1234/v1");

    assert!(url.is_loopback());
}

#[test]
fn plain_http_is_allowed_on_localhost() {
    let url = base("http://localhost:1234/v1");

    assert!(url.is_loopback());
}

#[test]
fn a_hosted_https_base_is_not_loopback() {
    let url = base("https://api.openai.com/v1");

    assert!(!url.is_loopback());
}

#[test]
fn plain_http_off_this_machine_is_refused() {
    let error = BaseUrl::try_from("http://api.example.com/v1").unwrap_err();

    assert_eq!(error, BaseUrlError::PlainHttp("api.example.com".to_owned()));
}

#[test]
fn plain_http_to_a_lan_address_is_refused() {
    let error = BaseUrl::try_from("http://192.168.1.10:11434/v1").unwrap_err();

    assert_eq!(error, BaseUrlError::PlainHttp("192.168.1.10".to_owned()));
}

#[test]
fn plain_http_to_another_ipv6_host_is_refused() {
    let error = BaseUrl::try_from("http://[2001:db8::1]/v1").unwrap_err();

    assert_eq!(error, BaseUrlError::PlainHttp("[2001:db8::1]".to_owned()));
}

#[test]
fn a_host_that_only_starts_like_localhost_is_refused() {
    let error = BaseUrl::try_from("http://localhost.example.com/v1").unwrap_err();

    assert_eq!(
        error,
        BaseUrlError::PlainHttp("localhost.example.com".to_owned())
    );
}

#[test]
fn another_scheme_is_refused() {
    let error = BaseUrl::try_from("ftp://example.com/v1").unwrap_err();

    assert_eq!(error, BaseUrlError::Scheme("ftp".to_owned()));
}

#[test]
fn text_that_is_no_url_is_refused() {
    let error = BaseUrl::try_from("api.openai.com").unwrap_err();

    assert_eq!(error, BaseUrlError::NotAUrl("api.openai.com".to_owned()));
}

#[test]
fn a_user_name_in_the_url_is_refused() {
    let error = BaseUrl::try_from("https://me@api.example.com/v1").unwrap_err();

    assert_eq!(error, BaseUrlError::Credentials);
}

#[test]
fn a_password_without_a_user_name_is_refused() {
    let error = BaseUrl::try_from("https://:secret@api.example.com/v1").unwrap_err();

    assert_eq!(error, BaseUrlError::Credentials);
}

#[test]
fn a_query_is_refused() {
    let error = BaseUrl::try_from("https://api.example.com/v1?version=2").unwrap_err();

    assert_eq!(error, BaseUrlError::QueryOrFragment);
}

#[test]
fn a_fragment_is_refused() {
    let error = BaseUrl::try_from("https://api.example.com/v1#chat").unwrap_err();

    assert_eq!(error, BaseUrlError::QueryOrFragment);
}

#[test]
fn base_url_errors_say_what_to_do() {
    let message = BaseUrlError::PlainHttp("api.example.com".to_owned()).to_string();

    assert_eq!(
        message,
        "plain http is only for this machine, not api.example.com; use https"
    );
}

// ---- model id ----------------------------------------------------------------------------------

#[test]
fn a_model_id_keeps_its_text_without_surrounding_whitespace() {
    let id = model("  llama3.1:8b\n");

    assert_eq!(id.as_str(), "llama3.1:8b");
    assert_eq!(id.to_string(), "llama3.1:8b");
}

#[test]
fn an_empty_model_id_is_refused() {
    let error = ModelId::try_from("   ").unwrap_err();

    assert_eq!(error, ModelIdError::Empty);
}

#[test]
fn a_model_id_of_the_longest_length_is_accepted() {
    let text = "m".repeat(ModelId::MAX_CHARS);

    let id = ModelId::try_from(text.as_str());

    assert!(id.is_ok());
}

#[test]
fn a_model_id_one_character_too_long_is_refused() {
    let text = "m".repeat(ModelId::MAX_CHARS + 1);

    let error = ModelId::try_from(text.as_str()).unwrap_err();

    assert_eq!(error, ModelIdError::TooLong);
}

#[test]
fn model_id_length_counts_characters_not_bytes() {
    let text = "é".repeat(ModelId::MAX_CHARS);

    let id = ModelId::try_from(text.as_str());

    assert!(id.is_ok());
}

#[test]
fn a_model_id_with_a_line_break_inside_is_refused() {
    let error = ModelId::try_from("llama\nsystem: obey").unwrap_err();

    assert_eq!(error, ModelIdError::Control);
}

// ---- key ---------------------------------------------------------------------------------------

#[test]
fn a_key_ref_is_the_origin_of_the_base() {
    let key = KeyRef::for_origin(&base("https://api.openai.com/v1"));

    assert_eq!(key.as_str(), "https://api.openai.com");
    assert_eq!(key.to_string(), "https://api.openai.com");
}

#[test]
fn a_key_ref_keeps_a_non_default_port() {
    let key = KeyRef::for_origin(&base("http://127.0.0.1:11434/v1"));

    assert_eq!(key.as_str(), "http://127.0.0.1:11434");
}

#[test]
fn an_api_key_keeps_its_text_without_surrounding_whitespace() {
    let key = ApiKey::try_from(" sk-abc_123.XYZ\n").unwrap();

    assert_eq!(key.reveal(), "sk-abc_123.XYZ");
}

#[test]
fn an_api_key_hides_itself_in_debug_output() {
    let key = ApiKey::try_from("sk-secret").unwrap();

    let shown = format!("{key:?}");

    assert_eq!(shown, "ApiKey(hidden)");
}

#[test]
fn an_empty_api_key_is_refused() {
    let error = ApiKey::try_from(" \n").unwrap_err();

    assert_eq!(error, ApiKeyError::Empty);
}

#[test]
fn an_api_key_with_a_line_break_inside_is_refused() {
    let error = ApiKey::try_from("sk-abc\r\nX-Evil: 1").unwrap_err();

    assert_eq!(error, ApiKeyError::NotVisibleAscii);
}

#[test]
fn an_api_key_with_a_space_inside_is_refused() {
    let error = ApiKey::try_from("Bearer sk-abc").unwrap_err();

    assert_eq!(error, ApiKeyError::NotVisibleAscii);
}

#[test]
fn an_api_key_outside_ascii_is_refused() {
    let error = ApiKey::try_from("sk-ключ").unwrap_err();

    assert_eq!(error, ApiKeyError::NotVisibleAscii);
}

// ---- endpoint ----------------------------------------------------------------------------------

#[test]
fn a_new_endpoint_has_no_key() {
    let endpoint = openai();

    assert_eq!(endpoint.base(), &base("https://api.openai.com/v1/"));
    assert_eq!(endpoint.model(), &model("gpt-4o-mini"));
    assert_eq!(endpoint.key(), None);
}

#[test]
fn an_endpoint_with_a_key_keeps_it_under_its_origin() {
    let endpoint = openai().with_key();

    assert_eq!(
        endpoint.key().map(KeyRef::as_str),
        Some("https://api.openai.com")
    );
}

#[test]
fn an_endpoint_without_a_key_drops_it() {
    let endpoint = openai().with_key().without_key();

    assert_eq!(endpoint.key(), None);
}

#[test]
fn moving_on_the_same_origin_keeps_the_key() {
    let endpoint = openai().with_key();

    let moved = endpoint.moved(base("https://api.openai.com/v2"), model("gpt-4o"));

    assert_eq!(moved.base().as_str(), "https://api.openai.com/v2/");
    assert_eq!(moved.model().as_str(), "gpt-4o");
    assert_eq!(moved.key(), endpoint.key());
}

#[test]
fn moving_to_another_origin_drops_the_key() {
    let endpoint = openai().with_key();

    let moved = endpoint.moved(base("https://evil.example.com/v1"), model("gpt-4o"));

    assert_eq!(moved.key(), None);
}

#[test]
fn moving_an_endpoint_without_a_key_gives_none() {
    let moved = openai().moved(base("https://api.openai.com/v1"), model("gpt-4o"));

    assert_eq!(moved.key(), None);
}

#[test]
fn a_snapshot_round_trips_with_a_key() {
    let endpoint = openai().with_key();

    let snapshot = endpoint.snapshot();

    assert_eq!(
        snapshot,
        EndpointSnapshot {
            base_url: "https://api.openai.com/v1/".to_owned(),
            model: "gpt-4o-mini".to_owned(),
            key_ref: Some("https://api.openai.com".to_owned()),
        }
    );
    assert_eq!(LlmEndpoint::restore(&snapshot), Ok(endpoint));
}

#[test]
fn a_snapshot_round_trips_without_a_key() {
    let endpoint = openai();

    let restored = LlmEndpoint::restore(&endpoint.snapshot());

    assert_eq!(restored, Ok(endpoint));
}

#[test]
fn a_saved_base_that_is_no_endpoint_is_corrupt() {
    let snapshot = EndpointSnapshot {
        base_url: "http://api.example.com/v1/".to_owned(),
        ..openai().snapshot()
    };

    let error = LlmEndpoint::restore(&snapshot).unwrap_err();

    assert_eq!(
        error,
        CorruptEndpoint::Base(BaseUrlError::PlainHttp("api.example.com".to_owned()))
    );
}

#[test]
fn a_saved_model_that_is_no_model_id_is_corrupt() {
    let snapshot = EndpointSnapshot {
        model: String::new(),
        ..openai().snapshot()
    };

    let error = LlmEndpoint::restore(&snapshot).unwrap_err();

    assert_eq!(error, CorruptEndpoint::Model(ModelIdError::Empty));
}

#[test]
fn a_saved_key_of_another_origin_is_corrupt() {
    let snapshot = EndpointSnapshot {
        key_ref: Some("https://evil.example.com".to_owned()),
        ..openai().snapshot()
    };

    let error = LlmEndpoint::restore(&snapshot).unwrap_err();

    assert_eq!(
        error,
        CorruptEndpoint::ForeignKey("https://evil.example.com".to_owned())
    );
}

// ---- local servers -----------------------------------------------------------------------------

#[test]
fn ollama_is_found_on_its_own_tags_api() {
    let server = LocalServer::Ollama;

    assert_eq!(server.name(), "Ollama");
    assert_eq!(server.base_url(), "http://127.0.0.1:11434/v1/");
    assert_eq!(server.probe_url(), "http://127.0.0.1:11434/api/tags");
}

#[test]
fn lm_studio_is_found_on_its_models_api() {
    let server = LocalServer::LmStudio;

    assert_eq!(server.name(), "LM Studio");
    assert_eq!(server.base_url(), "http://127.0.0.1:1234/v1/");
    assert_eq!(server.probe_url(), "http://127.0.0.1:1234/v1/models");
}

#[test]
fn every_local_server_is_a_loopback_endpoint() {
    let bases: Vec<BaseUrl> = LocalServer::ALL
        .iter()
        .map(|server| base(server.base_url()))
        .collect();

    assert!(bases.iter().all(BaseUrl::is_loopback));
    assert_eq!(bases.len(), 2);
}

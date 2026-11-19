use quickie_application::{ChatEvent, ChatRequest, LlmError};
use quickie_domain_agents::{
    ApiKey, BaseUrl, LlmEndpoint, ModelId, PageText, Prompt, Question, Request, TokenBudget,
};
use serde_json::{Value, json};
use url::Url;

use super::{Chunk, chat_body, chunk, error_message, headers, models, ollama_tags, whole};

fn model(text: &str) -> ModelId {
    ModelId::try_from(text).unwrap()
}

fn delta(text: &str) -> Chunk {
    Chunk::Events(vec![ChatEvent::Delta(text.to_owned())])
}

#[test]
fn a_chat_body_streams_the_prompt_to_the_model() {
    let endpoint = LlmEndpoint::new(
        BaseUrl::try_from("http://127.0.0.1:11434/v1").unwrap(),
        model("llama3.1:8b"),
    );
    let request = Request::Question(Question::try_from("Why?").unwrap());
    let page = PageText::new(Url::parse("https://example.com/").unwrap(), "T", "Text");
    let prompt = Prompt::assemble(&request, &[page], TokenBudget::DEFAULT).unwrap();

    let body = chat_body(&ChatRequest {
        endpoint: &endpoint,
        key: None,
        prompt: &prompt,
    });

    let value: Value = serde_json::from_slice(&body).unwrap();
    let messages = &prompt.messages();
    assert_eq!(
        value,
        json!({
            "model": "llama3.1:8b",
            "messages": [
                { "role": "system", "content": messages[0].content },
                { "role": "user", "content": messages[1].content },
            ],
            "stream": true,
            "stream_options": { "include_usage": true },
        })
    );
}

#[test]
fn headers_carry_the_key_as_a_bearer_token() {
    let key = ApiKey::try_from("sk-live").unwrap();

    let sent = headers("text/event-stream", true, Some(&key));

    assert_eq!(
        sent,
        [
            ("Accept".to_owned(), "text/event-stream".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), "Bearer sk-live".to_owned()),
        ]
    );
}

#[test]
fn headers_without_a_key_have_no_authorization() {
    let sent = headers("application/json", false, None);

    assert_eq!(sent, [("Accept".to_owned(), "application/json".to_owned())]);
}

// ---- chat chunks -------------------------------------------------------------------------------

#[test]
fn done_ends_the_answer() {
    assert_eq!(chunk("[DONE]"), Ok(Chunk::Done));
    assert_eq!(chunk(" [DONE] "), Ok(Chunk::Done));
}

#[test]
fn a_delta_is_the_next_token() {
    let read = chunk(r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Hi"}}]}"#);

    assert_eq!(read, Ok(delta("Hi")));
}

#[test]
fn a_delta_without_content_is_nothing() {
    let read = chunk(r#"{"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#);

    assert_eq!(read, Ok(Chunk::Events(Vec::new())));
}

#[test]
fn an_empty_delta_is_nothing() {
    let read = chunk(r#"{"choices":[{"delta":{"content":""}}]}"#);

    assert_eq!(read, Ok(Chunk::Events(Vec::new())));
}

#[test]
fn a_null_content_is_nothing() {
    let read = chunk(r#"{"choices":[{"delta":{"content":null}}]}"#);

    assert_eq!(read, Ok(Chunk::Events(Vec::new())));
}

#[test]
fn usage_is_the_servers_token_count() {
    let read = chunk(r#"{"choices":[],"usage":{"prompt_tokens":90,"completion_tokens":42}}"#);

    assert_eq!(
        read,
        Ok(Chunk::Events(vec![ChatEvent::Usage {
            completion_tokens: 42
        }]))
    );
}

#[test]
fn a_last_token_can_come_with_usage() {
    let read = chunk(r#"{"choices":[{"delta":{"content":"."}}],"usage":{"completion_tokens":3}}"#);

    assert_eq!(
        read,
        Ok(Chunk::Events(vec![
            ChatEvent::Delta(".".to_owned()),
            ChatEvent::Usage {
                completion_tokens: 3
            }
        ]))
    );
}

#[test]
fn an_error_object_in_the_stream_fails_the_answer() {
    let read = chunk(r#"{"error":{"message":"model overloaded","type":"server_error"}}"#);

    assert_eq!(read, Err(LlmError::Api("model overloaded".to_owned())));
}

#[test]
fn an_error_string_in_the_stream_fails_the_answer() {
    let read = chunk(r#"{"error":"out of memory"}"#);

    assert_eq!(read, Err(LlmError::Api("out of memory".to_owned())));
}

#[test]
fn an_error_of_another_shape_is_quoted() {
    let read = chunk(r#"{"error":{"code":500}}"#);

    assert_eq!(read, Err(LlmError::Api(r#"{"code":500}"#.to_owned())));
}

#[test]
fn data_that_is_not_json_is_malformed() {
    let read = chunk("not json");

    assert_eq!(read, Err(LlmError::Malformed("not json".to_owned())));
}

// ---- whole answers -----------------------------------------------------------------------------

#[test]
fn a_whole_answer_is_one_token() {
    let read = whole(br#"{"choices":[{"message":{"role":"assistant","content":"All at once"}}]}"#);

    assert_eq!(read, Ok(vec![ChatEvent::Delta("All at once".to_owned())]));
}

#[test]
fn a_whole_error_fails() {
    let read = whole(br#"{"error":{"message":"bad model"}}"#);

    assert_eq!(read, Err(LlmError::Api("bad model".to_owned())));
}

#[test]
fn a_whole_object_without_an_answer_is_malformed() {
    let read = whole(br#"{"choices":[]}"#);

    assert_eq!(
        read,
        Err(LlmError::Malformed(r#"{"choices":[]}"#.to_owned()))
    );
}

#[test]
fn an_empty_body_is_malformed() {
    let read = whole(b"  ");

    assert_eq!(read, Err(LlmError::Malformed("an empty answer".to_owned())));
}

// ---- errors ------------------------------------------------------------------------------------

#[test]
fn an_openai_error_body_gives_its_message() {
    let body = br#"{"error":{"message":"Incorrect API key provided","code":"invalid_api_key"}}"#;

    assert_eq!(error_message(body), "Incorrect API key provided");
}

#[test]
fn an_ollama_error_body_gives_its_message() {
    assert_eq!(
        error_message(br#"{"error":"model \"x\" not found"}"#),
        r#"model "x" not found"#
    );
}

#[test]
fn a_bare_message_body_gives_its_message() {
    assert_eq!(
        error_message(br#"{"message":"Unauthorized"}"#),
        "Unauthorized"
    );
}

#[test]
fn a_json_body_without_a_message_is_quoted() {
    assert_eq!(error_message(br#"{"detail":1}"#), r#"{"detail":1}"#);
}

#[test]
fn a_text_body_is_quoted_trimmed_and_cut() {
    let body = format!("  {}  ", "x".repeat(400));

    assert_eq!(error_message(body.as_bytes()), "x".repeat(300));
}

// ---- model lists -------------------------------------------------------------------------------

#[test]
fn an_openai_model_list_gives_its_ids() {
    let body = br#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"o3"}]}"#;

    assert_eq!(models(body), Ok(vec![model("gpt-4o"), model("o3")]));
}

#[test]
fn entries_that_are_no_model_are_left_out() {
    let body = br#"{"data":[{"id":""},{"name":"x"},{"id":7},{"id":"ok"}]}"#;

    assert_eq!(models(body), Ok(vec![model("ok")]));
}

#[test]
fn a_model_list_without_data_is_malformed() {
    assert_eq!(
        models(br#"{"models":[]}"#),
        Err(LlmError::Malformed(r#"{"models":[]}"#.to_owned()))
    );
}

#[test]
fn a_model_list_that_is_not_json_is_malformed() {
    assert_eq!(
        models(b"<html>"),
        Err(LlmError::Malformed("<html>".to_owned()))
    );
}

#[test]
fn ollama_tags_give_the_model_names() {
    let body = br#"{"models":[{"name":"llama3.1:8b","size":4661224676},{"name":"phi3:latest"}]}"#;

    assert_eq!(
        ollama_tags(body),
        Ok(vec![model("llama3.1:8b"), model("phi3:latest")])
    );
}

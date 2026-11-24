//! The client against a scripted [`HttpPort`]: what it sends, and how it reads every kind of
//! answer.

mod detection;
mod tools;

use std::cell::{Cell, RefCell};

use quickie_application::{
    ChatEvent, ChatRequest, Ended, Flow, HttpError, HttpEvent, HttpMethod, HttpPort, HttpRequest,
    LlmError, LlmPort, StopSignal,
};
use quickie_domain_agents::{
    ApiKey, BaseUrl, FenceMarker, LlmEndpoint, LocalServer, ModelId, PageText, Prompt, Question,
    Request, TokenBudget,
};
use url::Url;

use super::{MAX_ANSWER_BYTES, MAX_KEPT_BYTES, OpenAiClient, keep};

/// Answers every request with `status` and `body`, cut into `pieces`, then `ended`, unless the
/// receiver stops it, which ends it as stopped; records the requests and the pieces delivered.
struct ScriptedHttp {
    status: Option<u16>,
    pieces: Vec<Vec<u8>>,
    ended: Result<Ended, HttpError>,
    sent: RefCell<Vec<HttpRequest>>,
    delivered: Cell<usize>,
}

impl ScriptedHttp {
    fn new(status: u16, pieces: &[&str]) -> Self {
        Self {
            status: Some(status),
            pieces: pieces
                .iter()
                .map(|piece| piece.as_bytes().to_vec())
                .collect(),
            ended: Ok(Ended::Complete),
            sent: RefCell::default(),
            delivered: Cell::new(0),
        }
    }
}

impl HttpPort for ScriptedHttp {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        Err(HttpError(format!("unexpected GET {url}")))
    }

    fn stream(
        &self,
        request: &HttpRequest,
        _stop: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        self.sent.borrow_mut().push(request.clone());
        if let Some(status) = self.status
            && on_event(HttpEvent::Status(status)) == Flow::Stop
        {
            return Ok(Ended::Stopped);
        }
        for piece in &self.pieces {
            self.delivered.set(self.delivered.get() + 1);
            if on_event(HttpEvent::Body(piece)) == Flow::Stop {
                return Ok(Ended::Stopped);
            }
        }
        self.ended.clone()
    }
}

fn endpoint() -> LlmEndpoint {
    LlmEndpoint::new(
        BaseUrl::try_from("https://api.example.com/v1").unwrap(),
        ModelId::try_from("small").unwrap(),
    )
}

fn prompt() -> Prompt {
    let request = Request::Question(Question::try_from("Why?").unwrap());
    let page = PageText::new(Url::parse("https://example.com/").unwrap(), "T", "Text");
    Prompt::assemble(
        &request,
        &[page],
        TokenBudget::DEFAULT,
        &FenceMarker::from_random([0; 6]),
    )
    .unwrap()
}

/// Asks through `http` without a key; returns how it ended and every event.
fn chat(http: ScriptedHttp) -> (Result<Ended, LlmError>, Vec<ChatEvent>, ScriptedHttp) {
    let client = OpenAiClient::new(http);
    let endpoint = endpoint();
    let prompt = prompt();
    let mut events = Vec::new();
    let ended = client.chat(
        &ChatRequest {
            endpoint: &endpoint,
            key: None,
            prompt: &prompt,
        },
        &StopSignal::new(),
        &mut |event| events.push(event),
    );
    (ended, events, client.http)
}

fn delta(text: &str) -> ChatEvent {
    ChatEvent::Delta(text.to_owned())
}

fn sse(content: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}\n\n")
}

// ---- the request -------------------------------------------------------------------------------

#[test]
fn a_chat_posts_to_chat_completions_as_a_stream() {
    let (_, _, http) = chat(ScriptedHttp::new(200, &["data: [DONE]\n\n"]));

    let sent = http.sent.borrow();
    assert_eq!(
        sent[0].url.as_str(),
        "https://api.example.com/v1/chat/completions"
    );
    assert!(matches!(&sent[0].method, HttpMethod::Post(body) if !body.is_empty()));
    assert_eq!(
        sent[0].headers,
        [
            ("Accept".to_owned(), "text/event-stream".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ]
    );
}

#[test]
fn a_chat_with_a_key_sends_it_as_a_bearer_token() {
    let client = OpenAiClient::new(ScriptedHttp::new(200, &["data: [DONE]\n\n"]));
    let endpoint = endpoint();
    let prompt = prompt();
    let key = ApiKey::try_from("sk-live").unwrap();

    let _ = client.chat(
        &ChatRequest {
            endpoint: &endpoint,
            key: Some(&key),
            prompt: &prompt,
        },
        &StopSignal::new(),
        &mut |_| {},
    );

    let sent = client.http.sent.borrow();
    assert!(
        sent[0]
            .headers
            .contains(&("Authorization".to_owned(), "Bearer sk-live".to_owned()))
    );
}

// ---- streamed answers --------------------------------------------------------------------------

#[test]
fn tokens_stream_out_until_done() {
    let body = format!("{}{}data: [DONE]\n\n", sse("Good"), sse(" day"));

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[&body]));

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, [delta("Good"), delta(" day")]);
}

#[test]
fn tokens_cut_across_pieces_stream_out_whole() {
    let body = format!("{}{}", sse("При"), sse("вет"));
    let (head, tail) = body.as_bytes().split_at(40);
    let mut http = ScriptedHttp::new(200, &[]);
    http.pieces = vec![head.to_vec(), tail.to_vec()];

    let (ended, events, _) = chat(http);

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, [delta("При"), delta("вет")]);
}

#[test]
fn anything_after_done_is_ignored() {
    let body = format!("data: [DONE]\n\n{}data: oops\n\n", sse("late"));

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[&body]));

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, []);
}

#[test]
fn a_stream_that_ends_without_done_is_complete() {
    let (ended, events, _) = chat(ScriptedHttp::new(200, &[&sse("Hi")]));

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, [delta("Hi")]);
}

#[test]
fn a_last_event_without_its_blank_line_still_counts() {
    let last = sse("end");
    let body = last.trim_end();

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[body]));

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, [delta("end")]);
}

#[test]
fn a_last_event_that_is_malformed_fails_the_answer() {
    let (ended, _, _) = chat(ScriptedHttp::new(200, &[&sse("ok"), "data: {"]));

    assert_eq!(ended, Err(LlmError::Malformed("{".to_owned())));
}

#[test]
fn the_usage_chunk_reports_the_token_count() {
    let body = "data: {\"choices\":[],\"usage\":{\"completion_tokens\":9}}\n\ndata: [DONE]\n\n";

    let (_, events, _) = chat(ScriptedHttp::new(200, &[body]));

    assert_eq!(
        events,
        [ChatEvent::Usage {
            completion_tokens: 9
        }]
    );
}

#[test]
fn an_error_in_the_stream_fails_the_answer_after_the_tokens_before_it() {
    let body = format!(
        "{}data: {{\"error\":{{\"message\":\"overloaded\"}}}}\n\n{}",
        sse("Part"),
        sse("never")
    );

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[&body]));

    assert_eq!(ended, Err(LlmError::Api("overloaded".to_owned())));
    assert_eq!(events, [delta("Part")]);
}

#[test]
fn an_error_event_fails_the_answer() {
    let body = "event: error\ndata: {\"error\":\"model unloaded\"}\n\n";

    let (ended, _, _) = chat(ScriptedHttp::new(200, &[body]));

    assert_eq!(ended, Err(LlmError::Api("model unloaded".to_owned())));
}

#[test]
fn an_event_that_is_not_json_fails_the_answer() {
    let (ended, _, _) = chat(ScriptedHttp::new(200, &["data: <html>\n\n"]));

    assert_eq!(ended, Err(LlmError::Malformed("<html>".to_owned())));
}

#[test]
fn a_whole_answer_instead_of_a_stream_is_one_token() {
    let body = r#"{"choices":[{"message":{"content":"Whole"}}]}"#;

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[body]));

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, [delta("Whole")]);
}

#[test]
fn an_error_object_instead_of_a_stream_fails() {
    let (ended, _, _) = chat(ScriptedHttp::new(200, &[r#"{"error":"no such model"}"#]));

    assert_eq!(ended, Err(LlmError::Api("no such model".to_owned())));
}

// ---- refused, stopped, broken ------------------------------------------------------------------

#[test]
fn a_refused_request_gives_the_status_and_the_endpoints_message() {
    let body = r#"{"error":{"message":"Incorrect API key provided"}}"#;

    let (ended, events, _) = chat(ScriptedHttp::new(401, &[body]));

    assert_eq!(
        ended,
        Err(LlmError::Status {
            status: 401,
            message: "Incorrect API key provided".to_owned(),
        })
    );
    assert_eq!(events, []);
}

#[test]
fn a_refused_request_is_never_read_as_a_stream() {
    let (ended, events, _) = chat(ScriptedHttp::new(500, &[&sse("not a token")]));

    assert!(matches!(ended, Err(LlmError::Status { status: 500, .. })));
    assert_eq!(events, []);
}

#[test]
fn statuses_at_both_ends_of_2xx_succeed() {
    for status in [200, 299] {
        let (ended, _, _) = chat(ScriptedHttp::new(status, &["data: [DONE]\n\n"]));

        assert_eq!(ended, Ok(Ended::Complete), "status {status}");
    }
}

#[test]
fn statuses_just_outside_2xx_fail() {
    for status in [199, 300] {
        let (ended, _, _) = chat(ScriptedHttp::new(status, &["data: [DONE]\n\n"]));

        assert!(
            matches!(ended, Err(LlmError::Status { .. })),
            "status {status}"
        );
    }
}

#[test]
fn a_stopped_stream_keeps_its_tokens() {
    let mut http = ScriptedHttp::new(200, &[&sse("so far")]);
    http.ended = Ok(Ended::Stopped);

    let (ended, events, _) = chat(http);

    assert_eq!(ended, Ok(Ended::Stopped));
    assert_eq!(events, [delta("so far")]);
}

#[test]
fn a_stopped_stream_that_had_failed_reports_the_failure() {
    let mut http = ScriptedHttp::new(200, &["data: nope\n\n"]);
    http.ended = Ok(Ended::Stopped);

    let (ended, _, _) = chat(http);

    assert_eq!(ended, Err(LlmError::Malformed("nope".to_owned())));
}

#[test]
fn a_stopped_stream_does_not_read_what_it_held_back() {
    let mut http = ScriptedHttp::new(200, &["data: {\"error\":\"late\"}"]);
    http.ended = Ok(Ended::Stopped);

    let (ended, _, _) = chat(http);

    assert_eq!(ended, Ok(Ended::Stopped));
}

#[test]
fn a_connection_failure_is_an_http_error() {
    let mut http = ScriptedHttp::new(200, &[]);
    http.ended = Err(HttpError("refused".to_owned()));

    let (ended, _, _) = chat(http);

    assert_eq!(ended, Err(LlmError::Http(HttpError("refused".to_owned()))));
}

#[test]
fn an_answer_without_a_status_is_malformed() {
    let mut http = ScriptedHttp::new(200, &[&sse("x")]);
    http.status = None;

    let (ended, events, _) = chat(http);

    assert_eq!(
        ended,
        Err(LlmError::Malformed("an answer without a status".to_owned()))
    );
    assert_eq!(events, []);
}

// ---- models ------------------------------------------------------------------------------------

#[test]
fn models_are_listed_from_the_models_path_with_the_key() {
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[r#"{"data":[{"id":"small"}]}"#]));
    let key = ApiKey::try_from("sk-live").unwrap();

    let models = client.models(endpoint().base(), Some(&key));

    assert_eq!(models, Ok(vec![ModelId::try_from("small").unwrap()]));
    let sent = client.http.sent.borrow();
    assert_eq!(sent[0].method, HttpMethod::Get);
    assert_eq!(sent[0].url.as_str(), "https://api.example.com/v1/models");
    assert_eq!(
        sent[0].headers,
        [
            ("Accept".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), "Bearer sk-live".to_owned()),
        ]
    );
}

#[test]
fn a_model_list_in_pieces_is_read_whole() {
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[r#"{"data":[{"id":"#, r#""a"}]}"#]));

    let models = client.models(endpoint().base(), None);

    assert_eq!(models, Ok(vec![ModelId::try_from("a").unwrap()]));
}

#[test]
fn a_refused_model_list_gives_the_status() {
    let client = OpenAiClient::new(ScriptedHttp::new(403, &[r#"{"message":"Forbidden"}"#]));

    let models = client.models(endpoint().base(), None);

    assert_eq!(
        models,
        Err(LlmError::Status {
            status: 403,
            message: "Forbidden".to_owned(),
        })
    );
}

#[test]
fn a_model_list_without_a_status_is_malformed() {
    let mut http = ScriptedHttp::new(200, &[]);
    http.status = None;
    let client = OpenAiClient::new(http);

    let models = client.models(endpoint().base(), None);

    assert_eq!(
        models,
        Err(LlmError::Malformed("an answer without a status".to_owned()))
    );
}

#[test]
fn a_model_list_that_never_came_is_an_http_error() {
    let mut http = ScriptedHttp::new(200, &[]);
    http.ended = Err(HttpError("refused".to_owned()));
    let client = OpenAiClient::new(http);

    let models = client.models(endpoint().base(), None);

    assert_eq!(models, Err(LlmError::Http(HttpError("refused".to_owned()))));
}

#[test]
fn ollama_is_probed_on_its_tags_without_a_key() {
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[r#"{"models":[{"name":"phi3"}]}"#]));

    let models = client.probe(LocalServer::Ollama);

    assert_eq!(models, Ok(vec![ModelId::try_from("phi3").unwrap()]));
    let sent = client.http.sent.borrow();
    assert_eq!(sent[0].url.as_str(), "http://127.0.0.1:11434/api/tags");
    assert_eq!(
        sent[0].headers,
        [("Accept".to_owned(), "application/json".to_owned())]
    );
}

#[test]
fn lm_studio_is_probed_on_its_models() {
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[r#"{"data":[{"id":"qwen"}]}"#]));

    let models = client.probe(LocalServer::LmStudio);

    assert_eq!(models, Ok(vec![ModelId::try_from("qwen").unwrap()]));
    assert_eq!(
        client.http.sent.borrow()[0].url.as_str(),
        "http://127.0.0.1:1234/v1/models"
    );
}

// ---- keeping bodies ----------------------------------------------------------------------------

#[test]
fn a_body_is_kept_up_to_its_limit() {
    let mut kept = vec![0; MAX_KEPT_BYTES - 4];

    let first = keep(&mut kept, b"abcd");
    let second = keep(&mut kept, b"ef");

    assert!(first);
    assert!(!second);
    assert_eq!(kept.len(), MAX_KEPT_BYTES);
    assert_eq!(&kept[MAX_KEPT_BYTES - 4..], b"abcd");
}

#[test]
fn a_body_that_overfills_by_one_byte_does_not_fit() {
    let mut kept = vec![0; MAX_KEPT_BYTES - 3];

    let fits = keep(&mut kept, b"abcd");

    assert!(!fits);
    assert_eq!(&kept[MAX_KEPT_BYTES - 3..], b"abc");
}

// ---- bounds ------------------------------------------------------------------------------------

#[test]
fn the_stream_is_stopped_once_done_arrives() {
    let http = ScriptedHttp::new(200, &["data: [DONE]\n\n", &sse("late"), &sse("later")]);

    let (ended, events, http) = chat(http);

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events, []);
    assert_eq!(http.delivered.get(), 1);
}

#[test]
fn the_stream_is_stopped_once_it_fails() {
    let http = ScriptedHttp::new(200, &["data: nope\n\n", &sse("never")]);

    let (ended, _, http) = chat(http);

    assert_eq!(ended, Err(LlmError::Malformed("nope".to_owned())));
    assert_eq!(http.delivered.get(), 1);
}

#[test]
fn a_line_without_end_is_too_large_and_stops_the_stream() {
    let endless = format!("data: {}", "x".repeat(1 << 20));
    let http = ScriptedHttp::new(200, &[&endless, &sse("never")]);

    let (ended, events, http) = chat(http);

    assert!(matches!(ended, Err(LlmError::TooLarge(_))));
    assert_eq!(events, []);
    assert_eq!(http.delivered.get(), 1);
}

#[test]
fn a_last_event_too_large_fails_at_the_end() {
    let line = format!("data: {}\n", "x".repeat(1023));
    let full = line.repeat(1024);

    let (ended, _, _) = chat(ScriptedHttp::new(200, &[&full, "data: x"]));

    assert!(matches!(ended, Err(LlmError::TooLarge(_))));
}

#[test]
fn an_answer_of_the_most_text_is_read_whole() {
    let token = "x".repeat(1024);
    let body = sse(&token).repeat(MAX_ANSWER_BYTES / 1024);

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[&body]));

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(events.len(), MAX_ANSWER_BYTES / 1024);
}

#[test]
fn an_answer_past_the_most_text_is_too_large_and_stops_the_stream() {
    let token = "x".repeat(1024);
    let body = sse(&token).repeat(MAX_ANSWER_BYTES / 1024);
    let http = ScriptedHttp::new(200, &[&body, &sse("x"), &sse("never")]);

    let (ended, events, http) = chat(http);

    assert_eq!(
        ended,
        Err(LlmError::TooLarge(format!(
            "an answer over {MAX_ANSWER_BYTES} bytes"
        )))
    );
    assert_eq!(events.len(), MAX_ANSWER_BYTES / 1024);
    assert_eq!(http.delivered.get(), 2);
}

#[test]
fn a_whole_answer_past_the_most_text_is_too_large() {
    let body = format!(
        r#"{{"choices":[{{"message":{{"content":"{}"}}}}]}}"#,
        "x".repeat(MAX_ANSWER_BYTES + 1)
    );

    let (ended, events, _) = chat(ScriptedHttp::new(200, &[&body]));

    assert!(matches!(ended, Err(LlmError::TooLarge(_))));
    assert_eq!(events, []);
}

#[test]
fn a_refusal_is_read_until_its_message_fills_what_is_kept() {
    let big = "x".repeat(MAX_KEPT_BYTES);
    let http = ScriptedHttp::new(500, &[&big, "more", "never"]);

    let (ended, _, http) = chat(http);

    assert!(matches!(ended, Err(LlmError::Status { status: 500, .. })));
    assert_eq!(http.delivered.get(), 2);
}

#[test]
fn a_model_list_too_large_to_keep_fails_and_stops_the_stream() {
    let big = "x".repeat(MAX_KEPT_BYTES);
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[&big, "y", "never"]));

    let models = client.models(endpoint().base(), None);

    assert_eq!(
        models,
        Err(LlmError::TooLarge(format!(
            "a list of models over {MAX_KEPT_BYTES} bytes"
        )))
    );
    assert_eq!(client.http.delivered.get(), 2);
}

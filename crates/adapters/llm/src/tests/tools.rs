use quickie_application::{
    AgentPage, AgentTool, AgentToolCall, ToolChatMessage, ToolChatReply, ToolChatRequest,
};
use serde_json::{Value, json};

use super::*;

fn completion(body: &str, automation: bool) -> Result<ToolChatReply, LlmError> {
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[body]));
    client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation,
        },
        &StopSignal::new(),
    )
}

#[test]
fn tool_completion_returns_a_finished_text_answer() {
    // Arrange
    let body = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Ready."}}]}"#;
    // Act
    let reply = completion(body, false);
    // Assert
    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
}

#[test]
fn tool_completion_returns_a_typed_source_read() {
    // Arrange
    let body = r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_page","arguments":"{\"page\":\"source\"}"}}]}}]}"#;
    // Act
    let reply = completion(body, false);
    // Assert
    assert_eq!(
        reply,
        Ok(ToolChatReply::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::ReadPage(AgentPage::Source)
        }))
    );
}

#[test]
fn nonstreaming_ollama_tool_calls_include_an_index() {
    let mut body = call_body("read_page", r#"{"page":"source"}"#);
    body["choices"][0]["message"]["tool_calls"][0]["index"] = json!(0);

    let reply = completion(&body.to_string(), false);

    assert_eq!(
        reply,
        Ok(ToolChatReply::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::ReadPage(AgentPage::Source),
        }))
    );
}

#[test]
fn compatible_gateways_can_omit_redundant_tags_and_send_object_arguments() {
    let body = r#"{"choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":"call_1","provider_specific_fields":{"opaque":true},"function":{"name":"read_page","arguments":{"page":"source"},"provider_field":"ignored"}}]}}]}"#;

    assert_eq!(
        completion(body, false),
        Ok(ToolChatReply::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::ReadPage(AgentPage::Source),
        }))
    );
}

#[test]
fn duplicate_object_arguments_are_rejected() {
    let body = r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_page","arguments":{"page":"source","page":"workspace"}}}]}}]}"#;

    assert!(matches!(
        completion(body, true),
        Err(LlmError::Malformed(_))
    ));
}

#[test]
fn tool_completion_serializes_call_and_result_in_order() {
    // Arrange
    let client = OpenAiClient::new(ScriptedHttp::new(
        200,
        &[
            r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Done"}}]}"#,
        ],
    ));
    let messages = [
        ToolChatMessage::System("Rules".to_owned()),
        ToolChatMessage::User("Read".to_owned()),
        ToolChatMessage::Assistant("Checking".to_owned()),
        ToolChatMessage::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::ReadPage(AgentPage::Source),
        }),
        ToolChatMessage::Result {
            id: "call_1".to_owned(),
            content: "Page text".to_owned(),
        },
    ];
    let key = ApiKey::try_from("sk-live").unwrap();
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: Some(&key),
            messages: &messages,
            automation: false,
        },
        &StopSignal::new(),
    );
    // Assert
    assert_eq!(reply, Ok(ToolChatReply::Answer("Done".to_owned())));
    let sent = client.http.sent.borrow();
    let HttpMethod::Post(body) = &sent[0].method else {
        panic!("expected POST")
    };
    let body: Value = serde_json::from_slice(body).unwrap();
    assert_eq!(
        body["messages"],
        json!([
            {"role":"system","content":"Rules"},
            {"role":"user","content":"Read"},
            {"role":"assistant","content":"Checking"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_page","arguments":"{\"page\":\"source\"}"}}]},
            {"role":"tool","tool_call_id":"call_1","content":"Page text"}
        ])
    );
    assert_eq!(body["stream"], false);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["tools"][0]["function"]["parameters"]["properties"]["page"]["enum"],
        json!(["source"])
    );
    assert_eq!(
        sent[0].url.as_str(),
        "https://api.example.com/v1/chat/completions"
    );
    assert_eq!(
        sent[0].headers[2],
        ("Authorization".to_owned(), "Bearer sk-live".to_owned())
    );
}

#[test]
fn a_stopped_transport_never_exposes_a_complete_tool_call() {
    // Arrange
    let mut http = ScriptedHttp::new(
        200,
        &[
            r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"x","type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]}"#,
        ],
    );
    http.ended = Ok(Ended::Stopped);
    let client = OpenAiClient::new(http);
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: true,
        },
        &StopSignal::new(),
    );
    // Assert
    assert!(matches!(reply, Err(LlmError::Api(_))));
}

#[test]
fn a_pre_cancelled_completion_sends_nothing() {
    // Arrange
    let client = OpenAiClient::new(ScriptedHttp::new(200, &[]));
    let stop = StopSignal::new();
    stop.stop();
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: false,
        },
        &stop,
    );
    // Assert
    assert!(matches!(reply, Err(LlmError::Api(_))));
    assert!(client.http.sent.borrow().is_empty());
}

fn call_body(name: &str, arguments: &str) -> Value {
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":arguments}}]}}]})
}

macro_rules! rejects_call {
    ($test:ident, $name:literal, $arguments:literal, $automation:literal) => {
        #[test]
        fn $test() {
            // Arrange
            let body = call_body($name, $arguments).to_string();
            // Act
            let reply = completion(&body, $automation);
            // Assert
            assert!(matches!(reply, Err(LlmError::Malformed(_))), "{reply:?}");
        }
    };
}

rejects_call!(unknown_tools_are_rejected, "execute_script", "{}", true);
rejects_call!(
    extra_arguments_are_rejected,
    "read_page",
    r#"{"page":"source","script":"x"}"#,
    true
);
rejects_call!(
    duplicate_arguments_are_rejected,
    "read_page",
    r#"{"page":"source","page":"workspace"}"#,
    true
);
rejects_call!(
    unknown_pages_are_rejected,
    "read_page",
    r#"{"page":"other"}"#,
    true
);
rejects_call!(missing_pages_are_rejected, "read_page", "{}", true);
rejects_call!(
    malformed_arguments_are_rejected,
    "read_page",
    "not json",
    true
);
rejects_call!(zero_link_ids_are_rejected, "open_link", r#"{"id":0}"#, true);
rejects_call!(
    negative_link_ids_are_rejected,
    "open_link",
    r#"{"id":-1}"#,
    true
);
rejects_call!(
    fractional_link_ids_are_rejected,
    "open_link",
    r#"{"id":1.5}"#,
    true
);
rejects_call!(
    overflowing_link_ids_are_rejected,
    "open_link",
    r#"{"id":4294967296}"#,
    true
);
rejects_call!(
    url_arguments_are_rejected,
    "open_link",
    r#"{"url":"https://example.com"}"#,
    true
);
rejects_call!(
    workspace_reads_need_automation,
    "read_page",
    r#"{"page":"workspace"}"#,
    false
);
rejects_call!(
    link_lists_need_automation,
    "list_links",
    r#"{"page":"source"}"#,
    false
);
rejects_call!(
    navigation_needs_automation,
    "open_link",
    r#"{"id":1}"#,
    false
);

#[test]
fn automation_accepts_workspace_reads() {
    // Arrange
    let body = call_body("read_page", r#"{"page":"workspace"}"#).to_string();
    // Act
    let reply = completion(&body, true);
    // Assert
    assert_eq!(
        reply,
        Ok(ToolChatReply::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::ReadPage(AgentPage::Workspace)
        }))
    );
}

#[test]
fn automation_accepts_source_link_lists() {
    // Arrange
    let body = call_body("list_links", r#"{"page":"source"}"#).to_string();
    // Act
    let reply = completion(&body, true);
    // Assert
    assert_eq!(
        reply,
        Ok(ToolChatReply::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::ListLinks(AgentPage::Source)
        }))
    );
}

#[test]
fn automation_accepts_navigation_by_link_id() {
    // Arrange
    let body = call_body("open_link", r#"{"id":1}"#).to_string();
    // Act
    let reply = completion(&body, true);
    // Assert
    assert_eq!(
        reply,
        Ok(ToolChatReply::Call(AgentToolCall {
            id: "call_1".to_owned(),
            tool: AgentTool::OpenLink(1)
        }))
    );
}

macro_rules! rejects_reply {
    ($test:ident, $body:expr) => {
        #[test]
        fn $test() {
            // Arrange
            let body = $body.to_string();
            // Act
            let reply = completion(&body, true);
            // Assert
            assert!(matches!(reply, Err(LlmError::Malformed(_))), "{reply:?}");
        }
    };
}

rejects_reply!(
    an_unfinished_call_is_rejected,
    json!({"choices":[{"finish_reason":"length","message":{"role":"assistant","tool_calls":[{"id":"x","type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]})
);
rejects_reply!(
    missing_call_ids_are_rejected,
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]})
);
rejects_reply!(
    empty_call_ids_are_rejected,
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":" ","type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]})
);
rejects_reply!(
    multiple_calls_are_rejected,
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"x","type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}},{"id":"x","type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]})
);
rejects_reply!(
    unsupported_call_types_are_rejected,
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"x","type":"custom","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]})
);
rejects_reply!(
    missing_content_is_rejected,
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant"}}]})
);
rejects_reply!(
    malformed_content_is_rejected,
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":[]}}]})
);
rejects_reply!(
    empty_content_is_rejected,
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":" "}}]})
);
rejects_reply!(
    unfinished_text_is_rejected,
    json!({"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"Partial"}}]})
);
rejects_reply!(
    legacy_function_calls_are_rejected,
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Ready","function_call":{"name":"open_link","arguments":"{\"id\":1}"}}}]})
);
rejects_reply!(
    multiple_choices_are_rejected,
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"First"}},{"finish_reason":"stop","message":{"role":"assistant","content":"Second"}}]})
);
rejects_reply!(
    duplicate_call_ids_are_rejected,
    r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"first","id":"second","type":"function","function":{"name":"open_link","arguments":"{\"id\":1}"}}]}}]}"#
);
rejects_reply!(
    a_missing_finish_reason_is_rejected,
    json!({"choices":[{"message":{"role":"assistant","content":"Ready"}}]})
);

#[test]
fn oversized_text_is_rejected() {
    // Arrange
    let body = json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"x".repeat(MAX_ANSWER_BYTES + 1)}}]}).to_string();
    // Act
    let reply = completion(&body, true);
    // Assert
    assert!(matches!(reply, Err(LlmError::TooLarge(_))));
}

#[test]
fn an_oversized_body_stops_before_an_action_is_exposed() {
    // Arrange
    let body = "x".repeat(MAX_KEPT_BYTES + 1);
    // Act
    let reply = completion(&body, true);
    // Assert
    assert!(matches!(reply, Err(LlmError::TooLarge(_))));
}

#[test]
fn automation_advertises_only_the_three_bounded_tools() {
    // Arrange
    let client = OpenAiClient::new(ScriptedHttp::new(
        200,
        &[
            r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Done"}}]}"#,
        ],
    ));
    // Act
    let _ = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: true,
        },
        &StopSignal::new(),
    );
    // Assert
    let sent = client.http.sent.borrow();
    let HttpMethod::Post(body) = &sent[0].method else {
        panic!("expected POST")
    };
    let body: Value = serde_json::from_slice(body).unwrap();
    assert_eq!(body["tools"].as_array().unwrap().len(), 3);
    assert_eq!(body["tools"][0]["function"]["name"], "read_page");
    assert_eq!(body["tools"][1]["function"]["name"], "list_links");
    assert_eq!(body["tools"][2]["function"]["name"], "open_link");
    assert_eq!(
        body["tools"][0]["function"]["parameters"]["properties"]["page"]["enum"],
        json!(["source", "workspace"])
    );
    assert_eq!(
        body["tools"][2]["function"]["parameters"]["properties"]["id"]["minimum"],
        1
    );
    assert_eq!(
        body["tools"][2]["function"]["parameters"]["additionalProperties"],
        false
    );
}

struct CancelAfterResponse(ScriptedHttp);

impl HttpPort for CancelAfterResponse {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        self.0.get(url)
    }

    fn stream(
        &self,
        request: &HttpRequest,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        let ended = self.0.stream(request, stop, on_event);
        stop.stop();
        ended
    }
}

#[test]
fn cancellation_after_a_complete_response_never_exposes_a_tool_call() {
    // Arrange
    let body = call_body("open_link", r#"{"id":1}"#).to_string();
    let client = OpenAiClient::new(CancelAfterResponse(ScriptedHttp::new(200, &[&body])));
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: true,
        },
        &StopSignal::new(),
    );
    // Assert
    assert!(matches!(reply, Err(LlmError::Api(_))));
}

#[test]
fn a_failed_transport_never_exposes_a_complete_tool_call() {
    // Arrange
    let body = call_body("open_link", r#"{"id":1}"#).to_string();
    let mut http = ScriptedHttp::new(200, &[&body]);
    http.ended = Err(HttpError("connection lost".to_owned()));
    let client = OpenAiClient::new(http);
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: true,
        },
        &StopSignal::new(),
    );
    // Assert
    assert_eq!(
        reply,
        Err(LlmError::Http(HttpError("connection lost".to_owned())))
    );
}

#[test]
fn a_refused_tool_completion_reports_the_endpoint_error() {
    // Arrange
    let client = OpenAiClient::new(ScriptedHttp::new(
        401,
        &[r#"{"error":{"message":"Wrong key"}}"#],
    ));
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: true,
        },
        &StopSignal::new(),
    );
    // Assert
    assert_eq!(
        reply,
        Err(LlmError::Status {
            status: 401,
            message: "Wrong key".to_owned()
        })
    );
}

#[test]
fn a_tool_completion_without_a_status_fails() {
    // Arrange
    let body = call_body("open_link", r#"{"id":1}"#).to_string();
    let mut http = ScriptedHttp::new(200, &[&body]);
    http.status = None;
    let client = OpenAiClient::new(http);
    // Act
    let reply = client.complete_with_tools(
        &ToolChatRequest {
            endpoint: &endpoint(),
            key: None,
            messages: &[],
            automation: true,
        },
        &StopSignal::new(),
    );
    // Assert
    assert!(matches!(reply, Err(LlmError::Malformed(_))));
}

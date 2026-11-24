use std::collections::VecDeque;
use std::rc::Rc;

use quickie_application::{
    AgentPage, AgentTool, AgentToolCall, ToolChatMessage, ToolChatReply, ToolChatRequest,
};
use serde_json::{Value, json};

use super::*;

const CHAT_ANSWER: &str =
    r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Ready."}}]}"#;
const CHAT_REASONED_ANSWER: &str = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Ready.","reasoning_content":"opaque final"}}]}"#;
const RESPONSES_ANSWER: &str = r#"{"object":"response","status":"completed","output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Ready.","annotations":[]}]}]}"#;
const RESPONSES_CALL: &str = r#"{"object":"response","status":"completed","output":[{"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},{"type":"function_call","id":"fc_1","call_id":"call_1","status":"completed","name":"read_page","arguments":"{\"page\":\"source\"}"}]}"#;
const CHAT_CALL: &str = r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":"I'll read the source.","reasoning_content":"opaque","tool_calls":[{"id":"call_1","type":"function","extra_content":{"signature":"opaque"},"function":{"name":"read_page","arguments":"{ \"page\" : \"source\" }"}}]}}]}"#;
const OLLAMA_TOOLS_UNAVAILABLE: &str = r#"{"error":{"type":"api_error","message":"registry.ollama.ai/library/small:latest does not support tools","code":null,"param":null}}"#;

#[derive(Clone, Default)]
struct Script {
    steps: Rc<RefCell<VecDeque<Step>>>,
    sent: Rc<RefCell<Vec<HttpRequest>>>,
}

struct Step {
    status: u16,
    body: Rc<str>,
    interrupt: bool,
    stop: bool,
}

impl Script {
    fn new(steps: &[(u16, &'static str)]) -> Self {
        Self {
            steps: Rc::new(RefCell::new(
                steps
                    .iter()
                    .map(|&(status, body)| Step {
                        status,
                        body: Rc::from(body),
                        interrupt: false,
                        stop: false,
                    })
                    .collect(),
            )),
            ..Self::default()
        }
    }

    fn paths(&self) -> Vec<String> {
        self.sent
            .borrow()
            .iter()
            .map(|request| request.url.path().to_owned())
            .collect()
    }

    fn body(&self, index: usize) -> Value {
        let requests = self.sent.borrow();
        let HttpMethod::Post(body) = &requests[index].method else {
            panic!("expected POST");
        };
        serde_json::from_slice(body).unwrap()
    }
}

impl HttpPort for Script {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        panic!("unexpected GET {url}");
    }

    fn stream(
        &self,
        request: &HttpRequest,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        self.sent.borrow_mut().push(request.clone());
        let response = self
            .steps
            .borrow_mut()
            .pop_front()
            .expect("unexpected request");
        if response.interrupt {
            return Err(HttpError("connection lost".to_owned()));
        }
        let _ = on_event(HttpEvent::Status(response.status));
        let _ = on_event(HttpEvent::Body(response.body.as_bytes()));
        if response.stop {
            stop.stop();
        }
        Ok(Ended::Complete)
    }
}

fn complete(
    client: &OpenAiClient<Script>,
    endpoint: &LlmEndpoint,
    messages: &[ToolChatMessage],
) -> Result<ToolChatReply, LlmError> {
    client.complete_with_tools(
        &ToolChatRequest {
            endpoint,
            key: None,
            messages,
            automation: false,
        },
        &StopSignal::new(),
    )
}

#[test]
fn missing_route_selects_responses_and_caches_it_for_that_model_at_that_endpoint() {
    let http = Script::new(&[
        (
            404,
            r#"{"error":{"code":"not_found","message":"Not found"}}"#,
        ),
        (200, RESPONSES_ANSWER),
        (200, RESPONSES_ANSWER),
        (200, CHAT_ANSWER),
        (200, CHAT_ANSWER),
    ]);
    let client = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let different_model =
        endpoint.moved(endpoint.base().clone(), ModelId::try_from("other").unwrap());
    let replies: Vec<_> = [&endpoint, &endpoint, &different_model]
        .into_iter()
        .map(|configured| complete(&client.for_turn(), configured, &[]))
        .collect();

    assert_eq!(
        replies,
        vec![Ok(ToolChatReply::Answer("Ready.".to_owned())); 3]
    );
    assert_eq!(
        http.paths(),
        [
            "/v1/chat/completions",
            "/v1/responses",
            "/v1/responses",
            "/v1/chat/completions"
        ]
    );
    assert_eq!(http.body(0)["tools"][0]["function"]["name"], "read_page");
    assert_eq!(http.body(1)["tools"][0]["name"], "read_page");
    assert_eq!(http.body(1)["store"], false);
}

#[test]
fn changing_only_the_endpoint_rechecks_the_route_and_keeps_its_prefix() {
    let http = Script::new(&[(405, ""), (200, RESPONSES_ANSWER), (200, CHAT_ANSWER)]);
    let client = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let new_endpoint = endpoint.moved(
        BaseUrl::try_from("https://api.example.com/proxy/v1").unwrap(),
        endpoint.model().clone(),
    );

    let first = complete(&client.for_turn(), &endpoint, &[]);
    let second = complete(&client.for_turn(), &new_endpoint, &[]);

    assert_eq!(first, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert_eq!(second, first);
    assert_eq!(
        http.paths(),
        [
            "/v1/chat/completions",
            "/v1/responses",
            "/proxy/v1/chat/completions"
        ]
    );
}

#[test]
fn the_openai_host_prefers_responses_and_can_use_chat_when_it_is_unavailable() {
    let http = Script::new(&[(405, ""), (200, CHAT_ANSWER)]);
    let client = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let official = endpoint.moved(
        BaseUrl::try_from("https://api.openai.com/v1").unwrap(),
        endpoint.model().clone(),
    );

    let reply = complete(&client.for_turn(), &official, &[]);

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert_eq!(http.paths(), ["/v1/responses", "/v1/chat/completions"]);
}

#[test]
fn responses_tool_results_replay_the_call_and_encrypted_reasoning() {
    let http = Script::new(&[(405, ""), (200, RESPONSES_CALL), (200, RESPONSES_ANSWER)]);
    let client = OpenAiClient::new(http.clone()).for_turn();
    let endpoint = endpoint();
    let prompt = ToolChatMessage::User("Read".to_owned());

    let reply = complete(&client, &endpoint, std::slice::from_ref(&prompt)).unwrap();
    let call = AgentToolCall {
        id: "call_1".to_owned(),
        tool: AgentTool::ReadPage(AgentPage::Source),
    };
    assert_eq!(reply, ToolChatReply::Call(call.clone()));
    let answer = complete(
        &client,
        &endpoint,
        &[
            prompt,
            ToolChatMessage::Call(call),
            ToolChatMessage::Result {
                id: "call_1".to_owned(),
                content: "page text".to_owned(),
            },
        ],
    );

    assert_eq!(answer, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert_eq!(
        http.paths(),
        ["/v1/chat/completions", "/v1/responses", "/v1/responses"]
    );
    let input = http.body(2)["input"].as_array().unwrap().clone();
    assert_eq!(input[1]["encrypted_content"], "opaque");
    assert_eq!(input[2]["id"], "fc_1");
    assert_eq!(input[2]["call_id"], "call_1");
    assert_eq!(
        input[3],
        json!({"type":"function_call_output","call_id":"call_1","output":"page text"})
    );
}

#[test]
fn chat_tool_results_replay_the_provider_metadata_on_the_same_route() {
    let http = Script::new(&[(200, CHAT_CALL), (200, CHAT_ANSWER)]);
    let client = OpenAiClient::new(http.clone()).for_turn();
    let endpoint = endpoint();
    let prompt = ToolChatMessage::User("Read".to_owned());

    let reply = complete(&client, &endpoint, std::slice::from_ref(&prompt)).unwrap();
    let ToolChatReply::Call(call) = reply else {
        panic!("expected a native call");
    };
    let result = ToolChatMessage::Result {
        id: call.id.clone(),
        content: "page text".to_owned(),
    };
    let answer = complete(
        &client,
        &endpoint,
        &[prompt, ToolChatMessage::Call(call), result],
    );

    assert_eq!(answer, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    let body = http.body(1);
    assert_eq!(body["messages"][1]["content"], "I'll read the source.");
    assert_eq!(body["messages"][1]["reasoning_content"], "opaque");
    assert_eq!(
        body["messages"][1]["tool_calls"][0]["extra_content"]["signature"],
        "opaque"
    );
    assert_eq!(
        body["messages"][1]["tool_calls"][0]["function"]["arguments"],
        r#"{ "page" : "source" }"#
    );
    assert_eq!(body["messages"][2]["tool_call_id"], "call_1");
}

#[test]
fn chat_replays_reasoning_in_the_next_turn_after_older_history_is_pruned() {
    let http = Script::new(&[
        (200, CHAT_ANSWER),
        (200, CHAT_CALL),
        (200, CHAT_REASONED_ANSWER),
        (200, CHAT_ANSWER),
    ]);
    let workspace = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let system = ToolChatMessage::System("Rules".to_owned());
    let older = ToolChatMessage::User("Older".to_owned());
    let read = ToolChatMessage::User("Read".to_owned());
    let answer_message = ToolChatMessage::Assistant("Ready.".to_owned());
    let first = workspace.for_turn();
    complete(&first, &endpoint, &[system.clone(), older.clone()]).unwrap();
    first.finish_turn();
    let current = workspace.for_turn();
    let mut history = vec![system, older, answer_message.clone(), read];
    let ToolChatReply::Call(call) = complete(&current, &endpoint, &history).unwrap() else {
        panic!("expected a native call");
    };
    history.push(ToolChatMessage::Call(call.clone()));
    history.push(ToolChatMessage::Result {
        id: call.id,
        content: "page text".to_owned(),
    });
    complete(&current, &endpoint, &history).unwrap();
    current.finish_turn();
    history.push(answer_message);
    history.drain(1..3);
    history.push(ToolChatMessage::User("Follow up".to_owned()));

    let reply = complete(&workspace.for_turn(), &endpoint, &history);

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    let sent = http.body(3);
    assert_eq!(sent["messages"][1]["content"], "Read");
    assert_eq!(sent["messages"][2]["reasoning_content"], "opaque");
    assert_eq!(sent["messages"][3]["tool_call_id"], "call_1");
    assert_eq!(sent["messages"][4]["reasoning_content"], "opaque final");
    assert_eq!(sent["messages"][5]["content"], "Follow up");
}

#[test]
fn a_final_answer_discarded_before_workspace_acceptance_leaves_no_replay() {
    let http = Script::new(&[(200, CHAT_REASONED_ANSWER), (200, CHAT_ANSWER)]);
    let workspace = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let system = ToolChatMessage::System("Rules".to_owned());
    let prompt = ToolChatMessage::User("Read".to_owned());
    let abandoned = workspace.for_turn();
    complete(&abandoned, &endpoint, &[system.clone(), prompt.clone()]).unwrap();
    drop(abandoned);

    let reply = complete(
        &workspace.for_turn(),
        &endpoint,
        &[
            system,
            prompt,
            ToolChatMessage::Assistant("Ready.".to_owned()),
            ToolChatMessage::User("Follow up".to_owned()),
        ],
    );

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert_eq!(
        http.body(1)["messages"][2]["reasoning_content"],
        Value::Null
    );
}

#[test]
fn a_fallback_rebuilds_history_without_the_old_formats_reasoning() {
    let http = Script::new(&[
        (200, CHAT_REASONED_ANSWER),
        (405, ""),
        (200, RESPONSES_ANSWER),
    ]);
    let workspace = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let mut history = vec![
        ToolChatMessage::System("Rules".to_owned()),
        ToolChatMessage::User("Read".to_owned()),
    ];
    let first = workspace.for_turn();
    complete(&first, &endpoint, &history).unwrap();
    first.finish_turn();
    history.push(ToolChatMessage::Assistant("Ready.".to_owned()));
    history.push(ToolChatMessage::User("Follow up".to_owned()));

    let reply = complete(&workspace.for_turn(), &endpoint, &history);

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert_eq!(
        http.body(1)["messages"][2]["reasoning_content"],
        "opaque final"
    );
    assert_eq!(
        http.body(2)["input"][2],
        json!({"role":"assistant","content":"Ready."})
    );
    assert!(!http.body(2).to_string().contains("opaque final"));
}

#[test]
fn a_cancelled_tool_turn_preserves_only_earlier_accepted_history() {
    let http = Script::new(&[
        (200, CHAT_REASONED_ANSWER),
        (200, CHAT_CALL),
        (200, CHAT_ANSWER),
    ]);
    let workspace = OpenAiClient::new(http.clone());
    let endpoint = endpoint();
    let mut history = vec![
        ToolChatMessage::System("Rules".to_owned()),
        ToolChatMessage::User("Begin".to_owned()),
    ];
    let first = workspace.for_turn();
    complete(&first, &endpoint, &history).unwrap();
    first.finish_turn();
    history.push(ToolChatMessage::Assistant("Ready.".to_owned()));
    history.push(ToolChatMessage::User("Read".to_owned()));
    let abandoned = workspace.for_turn();
    assert!(matches!(
        complete(&abandoned, &endpoint, &history),
        Ok(ToolChatReply::Call(_))
    ));
    drop(abandoned);
    history.pop();
    history.push(ToolChatMessage::User("Continue".to_owned()));

    let reply = complete(&workspace.for_turn(), &endpoint, &history);

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    let sent = http.body(2);
    assert_eq!(sent["messages"].as_array().unwrap().len(), 4);
    assert_eq!(sent["messages"][2]["reasoning_content"], "opaque final");
    assert_eq!(sent["messages"][3]["content"], "Continue");
    assert!(!sent.to_string().contains("call_1"));
}

#[test]
fn a_turn_stays_on_its_route_after_a_native_call() {
    let http = Script::new(&[(200, CHAT_CALL), (405, "")]);
    let client = OpenAiClient::new(http.clone()).for_turn();
    let endpoint = endpoint();
    let prompt = ToolChatMessage::User("Read".to_owned());

    let reply = complete(&client, &endpoint, std::slice::from_ref(&prompt)).unwrap();
    let ToolChatReply::Call(call) = reply else {
        panic!("expected a native call");
    };
    let result = ToolChatMessage::Result {
        id: call.id.clone(),
        content: "page text".to_owned(),
    };
    let follow_up = complete(
        &client,
        &endpoint,
        &[prompt, ToolChatMessage::Call(call), result],
    );

    assert!(matches!(
        follow_up,
        Err(LlmError::Status { status: 405, .. })
    ));
    assert_eq!(
        http.paths(),
        ["/v1/chat/completions", "/v1/chat/completions"]
    );
}

#[test]
fn only_an_explicit_unsupported_tool_parameter_triggers_alternate_api() {
    let http = Script::new(&[
        (
            400,
            r#"{"error":{"code":"unsupported_parameter","param":"tools"}}"#,
        ),
        (200, RESPONSES_ANSWER),
        (
            400,
            r#"{"error":{"code":"unsupported_parameter","param":"model"}}"#,
        ),
    ]);
    let client = OpenAiClient::new(http.clone());
    let endpoint = endpoint();

    let first = complete(&client.for_turn(), &endpoint, &[]);
    let second = complete(&client.for_turn(), &endpoint, &[]);

    assert_eq!(first, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert!(matches!(second, Err(LlmError::Status { status: 400, .. })));
    assert_eq!(
        http.paths(),
        ["/v1/chat/completions", "/v1/responses", "/v1/responses"]
    );
}

#[test]
fn both_explicit_rejections_confirm_native_tools_are_unavailable() {
    // Arrange
    let http = Script::new(&[
        (400, r#"{"error":{"code":"tools_not_supported"}}"#),
        (404, r#"{"error":{"code":"route_not_found"}}"#),
    ]);
    let client = OpenAiClient::new(http.clone()).for_turn();

    // Act
    let reply = complete(&client, &endpoint(), &[]);

    // Assert
    assert_eq!(reply, Err(LlmError::ToolsUnavailable));
    assert_eq!(http.paths(), ["/v1/chat/completions", "/v1/responses"]);
}

#[test]
fn ollama_tools_rejections_allow_plain_chat_when_responses_is_absent() {
    for (model, body) in [
        ("small", OLLAMA_TOOLS_UNAVAILABLE),
        (
            "team/small:base",
            r#"{"error":{"type":"api_error","message":"registry.ollama.ai/team/small:base does not support tools","code":null,"param":null}}"#,
        ),
        (
            "registry.example:5000/team/small",
            r#"{"error":{"type":"api_error","message":"registry.example:5000/team/small:latest does not support tools","code":null,"param":null}}"#,
        ),
        (
            "small",
            r#"{"error":{"type":"invalid_request_error","message":"'small' does not support tools","code":null,"param":null}}"#,
        ),
    ] {
        let configured = endpoint();
        let local = configured.moved(
            BaseUrl::try_from("http://127.0.0.1:11434/v1").unwrap(),
            ModelId::try_from(model).unwrap(),
        );
        let http = Script::new(&[(400, body), (404, "404 page not found")]);
        let client = OpenAiClient::new(http.clone()).for_turn();

        let reply = complete(&client, &local, &[]);

        assert_eq!(reply, Err(LlmError::ToolsUnavailable), "{body}");
        assert_eq!(http.paths(), ["/v1/chat/completions", "/v1/responses"]);
    }
}

#[test]
fn ollama_nearby_errors_leave_the_original_request_failure() {
    for (status, body) in [
        (401, OLLAMA_TOOLS_UNAVAILABLE),
        (422, OLLAMA_TOOLS_UNAVAILABLE),
        (
            404,
            r#"{"error":{"type":"api_error","message":"model 'small' not found","code":null,"param":null}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"api_error","message":"\"small\" does not support chat","code":null,"param":null}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"api_error","message":"registry.ollama.ai/library/other:latest does not support tools","code":null,"param":null}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"api_error","message":"could not load small: model does not support tools","code":null,"param":null}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"api_error","message":"registry.ollama.ai/library/small:latest does not support tools","code":null,"param":"model"}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"api_error","message":"registry.ollama.ai/library/small:latest does not support tools","param":null}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"authentication_error","message":"registry.ollama.ai/library/small:latest does not support tools","code":null,"param":null}}"#,
        ),
    ] {
        let http = Script::new(&[(status, body)]);
        let client = OpenAiClient::new(http.clone()).for_turn();

        let reply = complete(&client, &endpoint(), &[]);

        assert_eq!(
            reply,
            Err(LlmError::Status {
                status,
                message: serde_json::from_str::<Value>(body).unwrap()["error"]["message"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            })
        );
        assert_eq!(http.paths(), ["/v1/chat/completions"], "{body}");
    }
}

#[test]
fn ollama_chat_error_does_not_mark_the_responses_route_unavailable() {
    let http = Script::new(&[(405, ""), (400, OLLAMA_TOOLS_UNAVAILABLE)]);
    let client = OpenAiClient::new(http.clone()).for_turn();

    let reply = complete(&client, &endpoint(), &[]);

    assert_eq!(
        reply,
        Err(LlmError::Status {
            status: 400,
            message: "registry.ollama.ai/library/small:latest does not support tools".to_owned(),
        })
    );
    assert_eq!(http.paths(), ["/v1/chat/completions", "/v1/responses"]);
}

#[test]
fn a_rejected_alternate_route_preserves_its_authorization_failure() {
    // Arrange
    let http = Script::new(&[(405, ""), (401, r#"{"error":{"message":"Denied"}}"#)]);
    let client = OpenAiClient::new(http.clone()).for_turn();

    // Act
    let reply = complete(&client, &endpoint(), &[]);

    // Assert
    assert_eq!(
        reply,
        Err(LlmError::Status {
            status: 401,
            message: "Denied".into()
        })
    );
    assert_eq!(http.paths(), ["/v1/chat/completions", "/v1/responses"]);
}

#[test]
fn auth_rate_limits_and_model_errors_do_not_resend_the_prompt() {
    for (status, body) in [
        (401, r#"{"error":{"message":"Invalid credentials"}}"#),
        (403, r#"{"error":{"message":"Denied"}}"#),
        (
            404,
            r#"{"error":{"code":"model_not_found","message":"Model not found"}}"#,
        ),
        (429, r#"{"error":{"message":"Rate limited"}}"#),
        (503, r#"{"error":{"message":"Unavailable"}}"#),
    ] {
        let http = Script::new(&[(status, body)]);
        let client = OpenAiClient::new(http.clone()).for_turn();

        let reply = complete(&client, &endpoint(), &[]);

        assert!(matches!(reply, Err(LlmError::Status { status: actual, .. }) if actual == status));
        assert_eq!(http.paths(), ["/v1/chat/completions"]);
    }
}

#[test]
fn a_route_specific_model_error_can_select_the_advertised_alternate() {
    let http = Script::new(&[
        (
            404,
            r#"{"error":{"code":"model_not_found","message":"The selected model is not supported in v1/chat/completions. Please use v1/responses instead."}}"#,
        ),
        (200, RESPONSES_ANSWER),
    ]);
    let client = OpenAiClient::new(http.clone()).for_turn();

    let reply = complete(&client, &endpoint(), &[]);

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    assert_eq!(http.paths(), ["/v1/chat/completions", "/v1/responses"]);
}

#[test]
fn cancelling_a_missing_route_does_not_issue_a_fallback_request() {
    let http = Script::new(&[(404, "")]);
    http.steps.borrow_mut()[0].stop = true;
    let client = OpenAiClient::new(http.clone()).for_turn();

    let reply = complete(&client, &endpoint(), &[]);

    assert!(matches!(reply, Err(LlmError::Api(_))));
    assert_eq!(http.paths(), ["/v1/chat/completions"]);
}

#[test]
fn a_transport_error_does_not_resend_the_prompt() {
    let http = Script::new(&[(200, "")]);
    http.steps.borrow_mut()[0].interrupt = true;
    let client = OpenAiClient::new(http.clone()).for_turn();

    let reply = complete(&client, &endpoint(), &[]);

    assert_eq!(
        reply,
        Err(LlmError::Http(HttpError("connection lost".to_owned())))
    );
    assert_eq!(http.paths(), ["/v1/chat/completions"]);
}

#[test]
fn an_invalid_success_payload_does_not_resend_the_prompt() {
    let http = Script::new(&[(200, r#"{"choices":[]}"#)]);
    let client = OpenAiClient::new(http.clone()).for_turn();

    let reply = complete(&client, &endpoint(), &[]);

    assert!(matches!(reply, Err(LlmError::Malformed(_))));
    assert_eq!(http.paths(), ["/v1/chat/completions"]);
}

#[test]
fn large_reasoning_drops_only_the_oldest_completed_turn_before_running_a_tool() {
    let http = Script::default();
    let encrypted = "x".repeat(960 << 10);
    let large_answer: Rc<str> = Rc::from(
        RESPONSES_ANSWER.replace(
            "\"output\":[",
            &format!(
                r#""output":[{{"type":"reasoning","id":"rs_large","summary":[],"encrypted_content":"{encrypted}"}}, "#,
            ),
        ),
    );
    http.steps.borrow_mut().extend(
        std::iter::once(Rc::from(RESPONSES_CALL.replace("call_1", "old_call")))
            .chain(std::iter::repeat_n(large_answer, 9))
            .chain([Rc::from(RESPONSES_CALL), Rc::from(RESPONSES_ANSWER)])
            .map(|body| Step {
                status: 200,
                body,
                interrupt: false,
                stop: false,
            }),
    );
    let workspace = OpenAiClient::new(http.clone());
    let endpoint = LlmEndpoint::new(
        BaseUrl::try_from("https://api.openai.com/v1").unwrap(),
        ModelId::try_from("small").unwrap(),
    );
    let mut messages = vec![ToolChatMessage::System("Rules".to_owned())];
    let first = workspace.for_turn();
    messages.push(ToolChatMessage::User("Historical 0".to_owned()));
    let ToolChatReply::Call(call) = complete(&first, &endpoint, &messages).unwrap() else {
        panic!("expected the oldest turn to call a tool");
    };
    messages.push(ToolChatMessage::Call(call.clone()));
    messages.push(ToolChatMessage::Result {
        id: call.id,
        content: "Old result".to_owned(),
    });
    let answer = ToolChatMessage::Assistant("Ready.".to_owned());
    complete(&first, &endpoint, &messages).unwrap();
    first.finish_turn();
    messages.push(answer.clone());
    for index in 1..9 {
        let earlier = workspace.for_turn();
        messages.push(ToolChatMessage::User(format!("Historical {index}")));
        complete(&earlier, &endpoint, &messages).unwrap();
        earlier.finish_turn();
        messages.push(answer.clone());
    }
    let current = workspace.for_turn();
    messages.push(ToolChatMessage::User("Current prompt".to_owned()));

    let ToolChatReply::Call(call) = complete(&current, &endpoint, &messages).unwrap() else {
        panic!("expected the current turn to call a tool");
    };
    messages.push(ToolChatMessage::Call(call.clone()));
    messages.push(ToolChatMessage::Result {
        id: call.id,
        content: "Current result".to_owned(),
    });
    let reply = complete(&current, &endpoint, &messages);

    assert_eq!(reply, Ok(ToolChatReply::Answer("Ready.".to_owned())));
    let request = http.body(11);
    let input = request["input"].as_array().unwrap();
    assert_eq!(input[0]["content"], "Rules");
    assert_eq!(input[1]["content"], "Historical 1");
    assert_eq!(input[2]["encrypted_content"], encrypted);
    assert!(input.iter().all(|item| item["call_id"] != "old_call"));
    assert_eq!(input[input.len() - 4]["content"], "Current prompt");
    assert_eq!(input[input.len() - 3]["encrypted_content"], "opaque");
    assert_eq!(input[input.len() - 2]["call_id"], "call_1");
    assert_eq!(
        input[input.len() - 1],
        json!({"type":"function_call_output","call_id":"call_1","output":"Current result"})
    );
}

#[test]
fn an_oversized_tool_request_does_not_reach_the_endpoint() {
    let http = Script::default();
    let client = OpenAiClient::new(http.clone()).for_turn();
    let history = [ToolChatMessage::User("a".repeat(8 << 20))];

    let reply = complete(&client, &endpoint(), &history);

    assert!(matches!(reply, Err(LlmError::TooLarge(_))));
    assert_eq!(http.paths(), Vec::<String>::new());
}

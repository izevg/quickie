use quickie_application::AgentTool;
use quickie_domain_agents::{BaseUrl, LlmEndpoint, ModelId};

use super::*;

#[test]
fn completed_response_exposes_only_assistant_text() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","error":null,"output":[
        {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
        {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"commentary",
         "content":[{"type":"output_text","text":"Checking.","annotations":[]}]},
        {"type":"message","id":"msg_2","role":"assistant","status":"completed","phase":"final_answer",
         "content":[
            {"type":"output_text","text":"Read it.","annotations":[]}
        ]}
    ]}"#;

    // Act
    let (reply, _) = response_reply(body, true).unwrap();

    // Assert
    assert_eq!(reply, ToolChatReply::Answer("Read it.".to_owned()));
}

#[test]
fn completed_answer_retains_sealed_reasoning_and_phase_separate_from_display_text() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","output":[
        {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque",
         "content":[{"type":"reasoning_text","text":"private chain"}]},
        {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"final_answer",
         "content":[{"type":"output_text","text":"First answer.","annotations":[]}]}
    ]}"#;

    // Act
    let (reply, items) = response_reply(body, true).unwrap();

    // Assert
    assert_eq!(reply, ToolChatReply::Answer("First answer.".to_owned()));
    assert_eq!(
        Value::Array(items),
        json!([
            {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
            {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"final_answer",
                "content":[{"type":"output_text","text":"First answer.","annotations":[]}]},
        ])
    );
}

#[test]
fn next_user_prompt_replays_prior_assistant_items_in_place() {
    // Arrange
    let first_body = br#"{"status":"completed","output":[
        {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
        {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"final_answer",
         "content":[{"type":"output_text","text":"First answer.","annotations":[]}]}
    ]}"#;
    let (ToolChatReply::Answer(first_answer), output) = response_reply(first_body, true).unwrap()
    else {
        panic!("expected a native answer")
    };
    let messages = [
        ToolChatMessage::System("Rules".to_owned()),
        ToolChatMessage::User("First question".to_owned()),
        ToolChatMessage::Assistant(first_answer),
        ToolChatMessage::User("Second question".to_owned()),
    ];
    let endpoint = endpoint();
    let request = ToolChatRequest {
        endpoint: &endpoint,
        key: None,
        messages: &messages,
        automation: true,
    };

    // Act
    let sent: Value = serde_json::from_slice(&response_body(&request, &[(2, output)])).unwrap();

    // Assert
    assert_eq!(
        sent["input"],
        json!([
            {"role":"system","content":"Rules"},
            {"role":"user","content":"First question"},
            {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
            {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"final_answer",
             "content":[{"type":"output_text","text":"First answer.","annotations":[]}]},
            {"role":"user","content":"Second question"},
        ])
    );
}

#[test]
fn tool_continuation_sends_reasoning_and_output_with_the_call_id() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","output":[
        {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"commentary",
         "content":[{"type":"output_text","text":"Checking.","annotations":[]}]},
        {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
        {"type":"function_call","id":"fc_1","call_id":"call_9","status":"completed",
         "name":"open_link","arguments":"{\"id\":7}","metadata":"ignored"}
    ]}"#;
    let (ToolChatReply::Call(call), items) = response_reply(body, true).unwrap() else {
        panic!("expected a native function call")
    };
    let messages = [
        ToolChatMessage::System("Rules".to_owned()),
        ToolChatMessage::User("Open the link".to_owned()),
        ToolChatMessage::Call(call.clone()),
        ToolChatMessage::Result {
            id: call.id.clone(),
            content: "Opened".to_owned(),
        },
    ];
    let endpoint = endpoint();
    let request = ToolChatRequest {
        endpoint: &endpoint,
        key: None,
        messages: &messages,
        automation: true,
    };

    // Act
    let sent: Value = serde_json::from_slice(&response_body(&request, &[(2, items)])).unwrap();

    // Assert
    assert_eq!(
        call,
        AgentToolCall {
            id: "call_9".to_owned(),
            tool: AgentTool::OpenLink(7)
        }
    );
    assert_eq!(sent["store"], false);
    assert_eq!(sent["stream"], false);
    assert_eq!(sent["parallel_tool_calls"], false);
    assert_eq!(sent["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(sent["tools"][0]["type"], "function");
    assert_eq!(sent["tools"][0]["name"], "read_page");
    assert!(sent["tools"][0].get("function").is_none());
    assert_eq!(
        sent["input"],
        json!([
            {"role":"system","content":"Rules"},
            {"role":"user","content":"Open the link"},
            {"type":"message","id":"msg_1","role":"assistant","status":"completed","phase":"commentary",
                "content":[{"type":"output_text","text":"Checking.","annotations":[]}]},
            {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
            {"type":"function_call","id":"fc_1","call_id":"call_9","status":"completed",
                "name":"open_link","arguments":"{\"id\":7}"},
            {"type":"function_call_output","call_id":"call_9","output":"Opened"},
        ])
    );
}

#[test]
fn incomplete_response_cannot_run_a_tool() {
    // Arrange
    let body = br#"{"object":"response","status":"incomplete","output":[
        {"type":"function_call","id":"fc_1","call_id":"call_9","status":"completed",
         "name":"open_link","arguments":"{\"id\":7}"}
    ]}"#;

    // Act
    let reply = response_reply(body, true);

    // Assert
    assert!(matches!(reply, Err(LlmError::Api(_))));
}

#[test]
fn duplicate_call_ids_are_rejected_before_json_is_normalized() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","output":[
        {"type":"function_call","id":"fc_1","call_id":"call_1","call_id":"call_2",
         "status":"completed","name":"open_link","arguments":"{\"id\":7}"}
    ]}"#;

    // Act
    let reply = response_reply(body, true);

    // Assert
    assert!(matches!(reply, Err(LlmError::Malformed(_))));
}

#[test]
fn a_second_function_call_cannot_escape_the_serial_tool_limit() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","output":[
        {"type":"function_call","call_id":"call_1","name":"open_link","arguments":"{\"id\":7}"},
        {"type":"function_call","call_id":"call_2","name":"read_page","arguments":"{\"page\":\"source\"}"}
    ]}"#;

    // Act
    let reply = response_reply(body, true);

    // Assert
    assert!(matches!(reply, Err(LlmError::Malformed(_))));
}

#[test]
fn object_arguments_are_rejected_on_the_responses_wire() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","output":[
        {"type":"function_call","call_id":"call_1","name":"open_link","arguments":{"id":7}}
    ]}"#;

    // Act
    let reply = response_reply(body, true);

    // Assert
    assert!(matches!(reply, Err(LlmError::Malformed(_))));
}

#[test]
fn refusal_cannot_be_overridden_by_a_tool_item() {
    // Arrange
    let body = br#"{"object":"response","status":"completed","output":[
        {"type":"function_call","call_id":"call_1","name":"open_link","arguments":"{\"id\":7}"},
        {"type":"message","role":"assistant","status":"completed","content":[
            {"type":"refusal","refusal":"I decline."}
        ]}
    ]}"#;

    // Act
    let reply = response_reply(body, true);

    // Assert
    assert!(matches!(reply, Err(LlmError::Api(_))));
}

fn endpoint() -> LlmEndpoint {
    LlmEndpoint::new(
        BaseUrl::try_from("https://api.example.com/v1").unwrap(),
        ModelId::try_from("model").unwrap(),
    )
}

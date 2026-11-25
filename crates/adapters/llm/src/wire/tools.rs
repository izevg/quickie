//! The bounded built-in tool vocabulary on the `OpenAI` chat wire.

use quickie_application::{
    AgentPage, AgentTool, AgentToolCall, LlmError, ToolChatMessage, ToolChatReply, ToolChatRequest,
};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

use crate::MAX_ANSWER_BYTES;

pub(crate) fn tool_body(request: &ToolChatRequest<'_>, replay: &[(usize, Vec<Value>)]) -> Vec<u8> {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let previous = replay
                .iter()
                .find(|(position, _)| *position == index)
                .and_then(|(_, items)| items.first());
            message(item, previous)
        })
        .collect();
    json!({
        "model":request.endpoint.model().as_str(), "messages":messages,
        "stream":false, "tools":definitions(request.automation),
        "tool_choice":"auto", "parallel_tool_calls":false,
    })
    .to_string()
    .into_bytes()
}

pub(super) fn definitions(automation: bool) -> Vec<Value> {
    let pages = if automation {
        json!(["source", "workspace"])
    } else {
        json!(["source"])
    };
    let page_arguments = json!({
        "type":"object", "properties":{"page":{"type":"string","enum":pages}},
        "required":["page"], "additionalProperties":false,
    });
    let mut tools = vec![definition(
        "read_page",
        "Read the visible text of a page.",
        &page_arguments,
    )];
    if automation {
        tools.push(definition(
            "list_links",
            "List the page's links with numeric identifiers.",
            &page_arguments,
        ));
        tools.push(definition("open_link", "Open a previously listed link in the agent workspace.", &json!({
            "type":"object", "properties":{"id":{"type":"integer","minimum":1,"maximum":u32::MAX}},
            "required":["id"], "additionalProperties":false,
        })));
    }
    tools
}

fn definition(name: &str, description: &str, parameters: &Value) -> Value {
    json!({"type":"function","function":{"name":name,"description":description,"strict":true,"parameters":parameters}})
}

fn message(message: &ToolChatMessage, replay: Option<&Value>) -> Value {
    match message {
        ToolChatMessage::System(content) => json!({"role":"system","content":content}),
        ToolChatMessage::User(content) => json!({"role":"user","content":content}),
        ToolChatMessage::Assistant(content) => replay
            .cloned()
            .unwrap_or_else(|| json!({"role":"assistant","content":content})),
        ToolChatMessage::Result { id, content } => {
            json!({"role":"tool","tool_call_id":id,"content":content})
        }
        ToolChatMessage::Call(call) => replay.cloned().unwrap_or_else(|| call_message(call)),
    }
}

fn call_message(call: &AgentToolCall) -> Value {
    let (name, arguments) = call_parts(call);
    json!({"role":"assistant","content":null,"tool_calls":[{
        "id":call.id,"type":"function","function":{"name":name,"arguments":arguments},
    }]})
}

pub(super) fn call_parts(call: &AgentToolCall) -> (&'static str, String) {
    let (name, arguments) = match call.tool {
        AgentTool::ReadPage(page) => ("read_page", json!({"page":page_name(page)})),
        AgentTool::ListLinks(page) => ("list_links", json!({"page":page_name(page)})),
        AgentTool::OpenLink(id) => ("open_link", json!({"id":id})),
    };
    (name, arguments.to_string())
}

fn page_name(page: AgentPage) -> &'static str {
    match page {
        AgentPage::Source => "source",
        AgentPage::Workspace => "workspace",
    }
}

#[derive(Deserialize)]
struct Completion {
    error: Option<Value>,
    choices: Option<Vec<Choice>>,
}

#[derive(Deserialize)]
struct Choice {
    finish_reason: String,
    message: Assistant,
}

#[derive(Deserialize)]
struct Assistant {
    role: Option<String>,
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<Call>>,
    function_call: Option<Value>,
    refusal: Option<String>,
}

#[derive(Deserialize)]
struct Call {
    id: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Function,
    extra_content: Option<Value>,
}

#[derive(Deserialize)]
struct Function {
    name: String,
    arguments: Box<RawValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Page {
    Source,
    Workspace,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageArguments {
    page: Page,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkArguments {
    id: u32,
}

pub(crate) fn tool_reply(
    body: &[u8],
    automation: bool,
) -> Result<(ToolChatReply, Vec<Value>), LlmError> {
    let completion: Completion = serde_json::from_slice(body).map_err(|_| malformed())?;
    if let Some(error) = completion.error {
        return Err(LlmError::Api(super::message_of(&error)));
    }
    let [choice]: [Choice; 1] = completion
        .choices
        .ok_or_else(malformed)?
        .try_into()
        .map_err(|_| malformed())?;
    let message = choice.message;
    if message
        .role
        .as_deref()
        .is_some_and(|role| role != "assistant")
        || message.function_call.is_some()
        || message.refusal.is_some()
    {
        return Err(malformed());
    }
    if message
        .content
        .as_ref()
        .is_some_and(|text| text.len() > MAX_ANSWER_BYTES)
    {
        return Err(LlmError::TooLarge(format!(
            "an answer over {MAX_ANSWER_BYTES} bytes"
        )));
    }
    let calls = message.tool_calls.unwrap_or_default();
    if calls.is_empty() && choice.finish_reason == "stop" {
        let text = message
            .content
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(malformed)?;
        let mut reply = json!({"role":"assistant","content":text});
        if let Some(reasoning) = message.reasoning_content {
            reply["reasoning_content"] = Value::String(reasoning);
        }
        return Ok((ToolChatReply::Answer(text), vec![reply]));
    }
    if choice.finish_reason != "tool_calls" {
        return Err(malformed());
    }
    let [call]: [Call; 1] = calls.try_into().map_err(|_| malformed())?;
    if call.kind.as_deref().is_some_and(|kind| kind != "function") {
        return Err(malformed());
    }
    let raw = call.function.arguments.get();
    let decoded;
    let arguments = if raw.starts_with('"') {
        decoded = serde_json::from_str::<String>(raw).map_err(|_| malformed())?;
        &decoded
    } else {
        raw
    };
    let validated = validated_call(call.id, &call.function.name, arguments, automation)?;
    let mut replay = call_message(&validated);
    replay["content"] = json!(message.content);
    replay["tool_calls"][0]["function"]["arguments"] = json!(arguments);
    if let Some(reasoning) = message.reasoning_content {
        replay["reasoning_content"] = Value::String(reasoning);
    }
    if let Some(extra) = call.extra_content {
        replay["tool_calls"][0]["extra_content"] = extra;
    }
    Ok((ToolChatReply::Call(validated), vec![replay]))
}

pub(super) fn validated_call(
    id: String,
    name: &str,
    arguments: &str,
    automation: bool,
) -> Result<AgentToolCall, LlmError> {
    if id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(malformed());
    }
    let tool = match name {
        "read_page" | "list_links" => {
            let arguments: PageArguments =
                serde_json::from_str(arguments).map_err(|_| malformed())?;
            let page = match arguments.page {
                Page::Source => AgentPage::Source,
                Page::Workspace if automation => AgentPage::Workspace,
                Page::Workspace => return Err(malformed()),
            };
            match name {
                "read_page" => AgentTool::ReadPage(page),
                _ if automation => AgentTool::ListLinks(page),
                _ => return Err(malformed()),
            }
        }
        "open_link" if automation => {
            let arguments: LinkArguments =
                serde_json::from_str(arguments).map_err(|_| malformed())?;
            if arguments.id == 0 {
                return Err(malformed());
            }
            AgentTool::OpenLink(arguments.id)
        }
        _ => return Err(malformed()),
    };
    Ok(AgentToolCall { id, tool })
}

fn malformed() -> LlmError {
    LlmError::Malformed("an invalid tool completion".to_owned())
}

//! The native Responses API wire format.

#[cfg(test)]
mod tests;

use quickie_application::{
    AgentToolCall, LlmError, ToolChatMessage, ToolChatReply, ToolChatRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};

use super::tools::{call_parts, definitions, validated_call};
use crate::MAX_ANSWER_BYTES;

pub(crate) fn response_body(
    request: &ToolChatRequest<'_>,
    replay: &[(usize, Vec<Value>)],
) -> Vec<u8> {
    let mut input = Vec::new();
    for (index, message) in request.messages.iter().enumerate() {
        if matches!(
            message,
            ToolChatMessage::Assistant(_) | ToolChatMessage::Call(_)
        ) && let Some((_, items)) = replay.iter().find(|(known, _)| *known == index)
            && !items.is_empty()
        {
            input.extend(items.iter().cloned());
            continue;
        }
        match message {
            ToolChatMessage::System(content) => {
                input.push(json!({"role":"system","content":content}));
            }
            ToolChatMessage::User(content) => input.push(json!({"role":"user","content":content})),
            ToolChatMessage::Assistant(content) => {
                input.push(json!({"role":"assistant","content":content}));
            }
            ToolChatMessage::Result { id, content } => {
                input.push(json!({"type":"function_call_output","call_id":id,"output":content}));
            }
            ToolChatMessage::Call(call) => {
                let (name, arguments) = call_parts(call);
                input.push(json!({
                    "type":"function_call","call_id":call.id,"name":name,"arguments":arguments,
                }));
            }
        }
    }
    let tools: Vec<Value> = definitions(request.automation)
        .into_iter()
        .map(|mut tool| {
            let mut function = tool["function"].take();
            function["type"] = Value::String("function".to_owned());
            function
        })
        .collect();
    json!({
        "model":request.endpoint.model().as_str(), "input":input, "tools":tools,
        "stream":false, "store":false, "include":["reasoning.encrypted_content"],
        "tool_choice":"auto", "parallel_tool_calls":false,
    })
    .to_string()
    .into_bytes()
}

#[derive(Deserialize)]
struct Response {
    object: Option<String>,
    status: Option<String>,
    error: Option<Value>,
    output: Option<Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum Output {
    FunctionCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Reasoning {
        id: String,
        #[serde(default)]
        summary: Vec<Summary>,
        #[serde(skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Message {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        role: Role,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<Phase>,
        content: Vec<Content>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum Summary {
    SummaryText { text: String },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Assistant,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Commentary,
    FinalAnswer,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum Content {
    OutputText {
        text: String,
        #[serde(default)]
        annotations: Vec<Value>,
    },
    Refusal {
        refusal: String,
    },
}

pub(crate) fn response_reply(
    body: &[u8],
    automation: bool,
) -> Result<(ToolChatReply, Vec<Value>), LlmError> {
    let response: Response = serde_json::from_slice(body).map_err(|_| malformed())?;
    if response
        .object
        .as_deref()
        .is_some_and(|kind| kind != "response")
    {
        return Err(malformed());
    }
    if response.error.is_some() {
        return Err(LlmError::Api(
            "the Responses API reported an error".to_owned(),
        ));
    }
    match response.status.as_deref() {
        Some("completed") => {}
        Some("incomplete" | "failed" | "cancelled" | "queued" | "in_progress") => {
            return Err(incomplete());
        }
        _ => return Err(malformed()),
    }
    let output = response.output.ok_or_else(malformed)?;
    let items: Vec<Output> = serde_json::from_str(output.get()).map_err(|_| malformed())?;
    let mut reply = Decoded::default();
    for item in items {
        reply.record(item, automation)?;
    }
    reply.finish()
}

#[derive(Default)]
struct Decoded {
    call: Option<AgentToolCall>,
    text: String,
    replay: Vec<Value>,
}

impl Decoded {
    fn record(&mut self, mut item: Output, automation: bool) -> Result<(), LlmError> {
        match &mut item {
            Output::FunctionCall {
                call_id,
                name,
                arguments,
                status,
                ..
            } => {
                complete(status.as_deref())?;
                if self.call.is_some() {
                    return Err(malformed());
                }
                self.call = Some(validated_call(
                    call_id.clone(),
                    name,
                    arguments,
                    automation,
                )?);
            }
            Output::Message {
                content,
                phase,
                status,
                ..
            } => {
                complete(status.as_deref())?;
                self.append_text(content, !matches!(phase, Some(Phase::Commentary)))?;
            }
            Output::Reasoning {
                status,
                encrypted_content,
                ..
            } => {
                complete(status.as_deref())?;
                if encrypted_content.as_ref().is_none_or(String::is_empty) {
                    return Ok(());
                }
            }
        }
        self.replay.push(json!(item));
        Ok(())
    }

    fn append_text(&mut self, content: &mut [Content], final_answer: bool) -> Result<(), LlmError> {
        let mut text = String::new();
        for part in content {
            match part {
                Content::OutputText {
                    text: part,
                    annotations,
                } => {
                    text.push_str(part);
                    annotations.clear();
                }
                Content::Refusal { .. } => {
                    return Err(LlmError::Api("the model declined the request".to_owned()));
                }
            }
        }
        if text.len() > MAX_ANSWER_BYTES {
            return Err(answer_too_large());
        }
        if final_answer && !text.is_empty() {
            if !self.text.is_empty() {
                self.text.push_str("\n\n");
            }
            if self.text.len() + text.len() > MAX_ANSWER_BYTES {
                return Err(answer_too_large());
            }
            self.text.push_str(&text);
        }
        Ok(())
    }

    fn finish(self) -> Result<(ToolChatReply, Vec<Value>), LlmError> {
        if let Some(call) = self.call {
            Ok((ToolChatReply::Call(call), self.replay))
        } else if self.text.trim().is_empty() {
            Err(malformed())
        } else {
            Ok((ToolChatReply::Answer(self.text), self.replay))
        }
    }
}

fn complete(status: Option<&str>) -> Result<(), LlmError> {
    match status {
        None | Some("completed") => Ok(()),
        Some("incomplete" | "in_progress") => Err(incomplete()),
        Some(_) => Err(malformed()),
    }
}

fn incomplete() -> LlmError {
    LlmError::Api("the Responses API did not finish the response".to_owned())
}

fn malformed() -> LlmError {
    LlmError::Malformed("an invalid Responses completion".to_owned())
}

fn answer_too_large() -> LlmError {
    LlmError::TooLarge(format!("an answer over {MAX_ANSWER_BYTES} bytes"))
}

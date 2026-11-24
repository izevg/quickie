use super::*;
use crate::{AgentPage, AgentTool, AgentToolCall};

fn call(id: &str) -> AgentToolCall {
    AgentToolCall {
        id: id.to_owned(),
        tool: AgentTool::ReadPage(AgentPage::Source),
    }
}

#[test]
fn a_follow_up_can_use_a_completed_tool_result() {
    let mut conversation = Conversation::default();
    assert!(conversation.begin("Open the video"));
    let read = call("call_1");

    assert!(conversation.accept(&ToolChatReply::Call(read)));
    assert!(conversation.tool_result("call_1", "Video".to_owned()));
    assert!(conversation.accept(&ToolChatReply::Answer("Done".to_owned())));
    assert!(conversation.begin("What was it called?"));

    assert_eq!(
        &conversation.messages()[2..],
        &[
            ToolChatMessage::Call(call("call_1")),
            ToolChatMessage::Result {
                id: "call_1".to_owned(),
                content: "Video".to_owned(),
            },
            ToolChatMessage::Assistant("Done".to_owned()),
            ToolChatMessage::User("What was it called?".to_owned()),
        ],
    );
}

#[test]
fn a_pending_call_must_receive_its_own_result_once() {
    let mut conversation = Conversation::default();
    assert!(conversation.begin("Read the page"));
    assert!(conversation.accept(&ToolChatReply::Call(call("first"))));

    assert!(!conversation.begin("Overlapping request"));
    assert!(!conversation.tool_result("other", "Wrong result".to_owned()));
    assert!(!conversation.accept(&ToolChatReply::Call(call("other"))));
    assert!(conversation.tool_result("first", "Result".to_owned()));
    assert!(!conversation.tool_result("first", "Duplicate".to_owned()));
    assert!(!conversation.accept(&ToolChatReply::Call(call("first"))));
}

#[test]
fn stopping_settles_the_turn_before_a_late_reply_arrives() {
    let mut conversation = Conversation::default();
    assert!(conversation.begin("Open the first video"));
    assert!(conversation.accept(&ToolChatReply::Call(call("old"))));

    conversation.cancel();

    assert!(!conversation.tool_result("old", "Finished after stop".to_owned()));
    assert!(!conversation.accept(&ToolChatReply::Answer("Too late".to_owned())));
    assert!(conversation.begin("Try again"));
    assert_eq!(conversation.messages().len(), 2);
}

#[test]
fn model_tool_calls_are_bounded_per_user_turn() {
    let mut conversation = Conversation::default();
    assert!(conversation.begin("Read"));
    for step in 0..MAX_TOOLS {
        let id = step.to_string();
        assert!(conversation.accept(&ToolChatReply::Call(call(&id))));
        assert!(conversation.tool_result(&id, "Read".to_owned()));
    }

    assert!(!conversation.accept(&ToolChatReply::Call(call("overflow"))));
    assert!(conversation.accept(&ToolChatReply::Answer("Finished".to_owned())));
}

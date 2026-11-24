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

#[test]
fn a_history_at_the_limit_keeps_every_completed_turn() {
    // Arrange
    let mut conversation = Conversation::default();
    finish_turn(&mut conversation, "Read the original", 1);
    finish_plain_turns(&mut conversation, 13);
    let previous = conversation.messages().to_vec();
    assert_eq!(previous.len(), 31);
    let current = ToolChatMessage::User("Start the next turn".to_owned());

    // Act
    assert!(conversation.begin("Start the next turn"));

    // Assert
    assert_eq!(conversation.messages(), &[previous, vec![current]].concat());
}

#[test]
fn overflowing_history_evicts_an_entire_tool_turn_and_keeps_the_next_one() {
    // Arrange
    let mut conversation = Conversation::default();
    finish_turn(&mut conversation, "Read the original", 1);
    finish_turn(&mut conversation, "Read the next page", 1);
    finish_plain_turns(&mut conversation, 12);
    let previous = conversation.messages().to_vec();
    assert_eq!(previous.len(), 33);

    // Act
    assert!(conversation.begin("Start the next turn"));

    // Assert
    let mut expected = vec![previous[0].clone()];
    expected.extend_from_slice(&previous[5..]);
    expected.push(ToolChatMessage::User("Start the next turn".to_owned()));
    assert_eq!(conversation.messages(), expected);
}

#[test]
fn overflowing_history_evicts_only_the_turns_needed_to_fit() {
    // Arrange
    let mut conversation = Conversation::default();
    finish_turn(&mut conversation, "Oldest question", 0);
    finish_turn(&mut conversation, "Second question", 0);
    finish_turn(&mut conversation, "Keep this tool result", 1);
    finish_plain_turns(&mut conversation, 10);
    finish_turn(&mut conversation, "Read two passages", 2);
    let previous = conversation.messages().to_vec();
    assert_eq!(previous.len(), 35);

    // Act
    assert!(conversation.begin("Start the next turn"));

    // Assert
    let mut expected = vec![previous[0].clone()];
    expected.extend_from_slice(&previous[5..]);
    expected.push(ToolChatMessage::User("Start the next turn".to_owned()));
    assert_eq!(conversation.messages(), expected);
}

fn finish_turn(conversation: &mut Conversation, prompt: &str, tools: u8) {
    assert!(conversation.begin(prompt));
    for step in 0..tools {
        let id = format!("{prompt}-{step}");
        assert!(conversation.accept(&ToolChatReply::Call(call(&id))));
        assert!(conversation.tool_result(&id, format!("Result for {id}")));
    }
    assert!(conversation.accept(&ToolChatReply::Answer(format!("Answer to {prompt}"))));
}

fn finish_plain_turns(conversation: &mut Conversation, count: u8) {
    for index in 0..count {
        finish_turn(conversation, &format!("Question {index}"), 0);
    }
}

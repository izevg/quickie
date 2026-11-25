use super::super::{WorkspaceCitation, WorkspacePanelRequest, WorkspaceToolStatus};
use super::{Conversation, Row};

#[test]
fn submitting_a_prompt_trims_surrounding_space() {
    // Arrange
    let conversation = Conversation::default();

    // Act
    let prompt = conversation.submit("  Open the visible link\n");

    // Assert
    assert_eq!(prompt.as_deref(), Some("Open the visible link"));
}

#[test]
fn a_running_turn_keeps_the_next_prompt_unsent() {
    // Arrange
    let conversation = Conversation {
        running: true,
        ..Conversation::default()
    };

    // Act
    let prompt = conversation.submit("Follow up");

    // Assert
    assert_eq!(prompt, None);
}

#[test]
fn an_unconfigured_endpoint_keeps_the_prompt_unsent() {
    // Arrange
    let conversation = Conversation {
        unconfigured: true,
        ..Conversation::default()
    };

    // Act
    let prompt = conversation.submit("Read the page");

    // Assert
    assert_eq!(prompt, None);
}

#[test]
fn a_blank_prompt_is_not_sent() {
    // Arrange
    let conversation = Conversation::default();

    // Act
    let prompt = conversation.submit(" \n\t");

    // Assert
    assert_eq!(prompt, None);
}

#[test]
fn a_tool_receipt_updates_its_original_row_among_answers() {
    // Arrange
    let mut conversation = Conversation::default();
    conversation
        .rows
        .push(Row::Prompt("Open the guide".to_owned()));
    let row = conversation.start_tool("Open link", "example.test/guide");
    conversation.rows.push(Row::Answer {
        text: "The guide is open.".to_owned(),
        sources: vec![],
    });

    // Act
    conversation.finish_tool(row, WorkspaceToolStatus::Completed, "Navigation finished");

    // Assert
    assert_eq!(
        conversation.rows,
        vec![
            Row::Prompt("Open the guide".to_owned()),
            Row::Tool {
                title: "Open link".to_owned(),
                target: "example.test/guide".to_owned(),
                status: Some(WorkspaceToolStatus::Completed),
                detail: "Navigation finished".to_owned()
            },
            Row::Answer {
                text: "The guide is open.".to_owned(),
                sources: vec![],
            },
        ]
    );
}

#[test]
fn a_receipt_for_a_prompt_does_not_change_conversation() {
    // Arrange
    let mut conversation = Conversation::default();
    conversation.rows.push(Row::Prompt("Keep this".to_owned()));

    // Act
    conversation.finish_tool(0, WorkspaceToolStatus::Failed, "Not a tool");

    // Assert
    assert_eq!(conversation.rows, vec![Row::Prompt("Keep this".to_owned())]);
}

#[test]
fn an_unknown_tool_receipt_does_not_change_conversation() {
    // Arrange
    let mut conversation = Conversation::default();
    conversation.rows.push(Row::Prompt("Keep this".to_owned()));

    // Act
    conversation.finish_tool(usize::MAX, WorkspaceToolStatus::Cancelled, "Unknown");

    // Assert
    assert_eq!(conversation.rows, vec![Row::Prompt("Keep this".to_owned())]);
}

#[test]
fn a_cancelled_tool_retains_its_target_and_partial_result() {
    // Arrange
    let mut conversation = Conversation::default();
    let row = conversation.start_tool("Read page", "example.test");

    // Act
    conversation.finish_tool(row, WorkspaceToolStatus::Cancelled, "Stopped by you");

    // Assert
    assert_eq!(
        conversation.rows[row],
        Row::Tool {
            title: "Read page".to_owned(),
            target: "example.test".to_owned(),
            status: Some(WorkspaceToolStatus::Cancelled),
            detail: "Stopped by you".to_owned(),
        }
    );
}

#[test]
fn a_verified_source_keeps_its_identifier_after_follow_up_answers() {
    // Arrange
    let conversation = Conversation {
        rows: vec![
            Row::Answer {
                text: "The page limits sovereign power [42].".to_owned(),
                sources: vec![WorkspaceCitation {
                    id: 42,
                    quote: "Sovereign power is limited by law".to_owned(),
                }],
            },
            Row::Answer {
                text: "A later answer.".to_owned(),
                sources: vec![],
            },
        ],
        ..Conversation::default()
    };

    // Act
    let request = conversation.cite(42);

    // Assert
    assert_eq!(request, Some(WorkspacePanelRequest::Cite(42)));
}

#[test]
fn model_text_cannot_manufacture_a_verified_citation() {
    // Arrange
    let conversation = Conversation {
        rows: vec![Row::Answer {
            text: "[42](quickie-cite:42) and [42](https://example.test/42)".to_owned(),
            sources: vec![WorkspaceCitation {
                id: 7,
                quote: "A verified passage".to_owned(),
            }],
        }],
        ..Conversation::default()
    };

    // Act
    let request = conversation.cite(42);

    // Assert
    assert_eq!(request, None);
}

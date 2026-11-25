use super::*;

#[test]
fn parse_locate_reply_extracts_elements_bounds_and_signals() {
    // Arrange
    let raw = r#"{
        "elements": [
            {
                "element_id": "el-1",
                "role": "button",
                "name": "Submit Order",
                "tag": "button",
                "bounds": { "x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0 },
                "signals": {
                    "connected": true,
                    "visible": true,
                    "is_submit_control": true,
                    "in_form": true,
                    "is_file_input": false,
                    "is_editable": false,
                    "is_select": false,
                    "link_url": null,
                    "link_opens_new_window": false,
                    "point": { "x": 60.0, "y": 40.0 }
                }
            },
            {
                "element_id": "el-2",
                "role": "link",
                "name": "Docs",
                "tag": "a",
                "bounds": { "x": 0.0, "y": 0.0, "width": 50.0, "height": 20.0 },
                "signals": {
                    "connected": true,
                    "visible": true,
                    "link_url": "https://other.example/docs",
                    "link_opens_new_window": true,
                    "point": { "x": 25.0, "y": 10.0 }
                }
            }
        ],
        "truncated": false
    }"#;

    // Act
    let snapshot = parse_locate_reply(raw, 1).expect("valid locate payload");

    // Assert
    assert!(snapshot.truncated);
    assert_eq!(snapshot.elements.len(), 1);
    let first = &snapshot.elements[0];
    assert_eq!(first.element_id, "el-1");
    assert_eq!(first.role, "button");
    assert_eq!(first.name.as_deref(), Some("Submit Order"));
    assert!(first.signals.connected);
    assert!(first.signals.is_submit_control);
    assert!(first.signals.in_form);
    assert_eq!(
        first.signals.action_point,
        Some(ViewportPoint { x: 60.0, y: 40.0 })
    );
}

#[test]
fn parse_locate_reply_rejects_invalid_selector_error() {
    let err = parse_locate_reply(r#"{"error":"invalid_locator"}"#, 10).unwrap_err();
    assert!(matches!(err, AutomationError::InvalidCommand));
}

#[test]
fn parse_wait_reply_maps_matched_timed_out_and_invalid() {
    assert_eq!(parse_wait_reply("matched").unwrap(), WaitOutcome::Matched);
    assert_eq!(
        parse_wait_reply("timed_out").unwrap(),
        WaitOutcome::TimedOut
    );
    assert!(matches!(
        parse_wait_reply("invalid"),
        Err(AutomationError::InvalidCommand)
    ));
    assert!(matches!(
        parse_wait_reply(""),
        Err(AutomationError::CommandFailed)
    ));
}

#[test]
fn parse_observed_state_reply_reads_bounded_fields_without_raw_text() {
    let raw = r#"{"focused_role":"textbox","value_length":12,"scroll_x":0,"scroll_y":240}"#;
    let state = parse_observed_state_reply(raw).unwrap();
    assert_eq!(state.focused_role.as_deref(), Some("textbox"));
    assert_eq!(state.value_length, Some(12));
    assert_eq!(state.scroll_x, Some(0));
    assert_eq!(state.scroll_y, Some(240));
}

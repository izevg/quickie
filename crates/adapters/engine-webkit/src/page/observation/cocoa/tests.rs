use quickie_application::mcp::{SnapshotKind, SnapshotRequest};
use quickie_application::{DiagnosticSnapshot, EngineError};

use super::snapshot::{build_script, parse_reply};
use super::{capabilities, has_local_diagnostic_spi};

#[test]
fn stock_webkit_reports_unsupported_local_diagnostic_spi_without_window() {
    // Arrange & Act
    let spi = has_local_diagnostic_spi();
    let caps = capabilities();

    // Assert
    assert!(caps.dom);
    assert!(
        !caps.accessibility,
        "accessibility must stay false without native accessibility data"
    );
    assert_eq!(caps.console, spi);
    assert_eq!(caps.runtime_errors, spi);
    assert_eq!(caps.pending_network, spi);
    assert_eq!(caps.completed_network, spi);
    assert_eq!(caps.lifecycle, spi);
    if !crate::page::inspector::on_fork() {
        assert!(!spi, "stock WebKit must not report the fork diagnostic SPI");
    }
}

#[test]
fn accessibility_snapshot_is_rejected_while_dom_and_text_build_bounded_scripts() {
    // Arrange
    let a11y = SnapshotRequest::with_max_bounds(SnapshotKind::Accessibility, None);
    let dom = SnapshotRequest::new(SnapshotKind::Dom, Some("#main".to_owned()), 50, 4, 1024);
    let text = SnapshotRequest::new(SnapshotKind::Text, None, 10, 2, 512);

    // Act
    let a11y_res = build_script(&a11y);
    let dom_script = build_script(&dom).expect("dom script");
    let text_script = build_script(&text).expect("text script");

    // Assert
    assert!(a11y_res.is_err());
    assert!(dom_script.contains("const maxNodes = 50;"));
    assert!(dom_script.contains("const maxDepth = 4;"));
    assert!(dom_script.contains("const maxBytes = 1024;"));
    assert!(dom_script.contains("\"#main\""));
    assert!(text_script.contains("const maxBytes = 512;"));
}

#[test]
fn snapshot_reply_parser_enforces_byte_bound_and_surfaces_selector_errors() {
    // Arrange
    let ok_reply = r#"{"data":"hello world","truncated":false}"#;
    let invalid_selector = r#"{"error":"invalid_selector"}"#;
    let not_found = r#"{"error":"selector_not_found"}"#;

    // Act
    let clamped = parse_reply(ok_reply, 5).expect("clamped snapshot");
    let bad_sel = parse_reply(invalid_selector, 64);
    let missing = parse_reply(not_found, 64);

    // Assert
    assert_eq!(
        clamped,
        DiagnosticSnapshot {
            data: "hello".to_owned(),
            truncated: true,
        }
    );
    assert_eq!(
        bad_sel,
        Err(EngineError("invalid snapshot selector".to_owned()))
    );
    assert_eq!(
        missing,
        Err(EngineError(
            "snapshot selector matched no element".to_owned()
        ))
    );
}

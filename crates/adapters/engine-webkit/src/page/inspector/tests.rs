use quickie_application::DevToolsPanel;

use super::tab_identifier;

#[test]
fn every_panel_shows_a_tab_web_inspector_has() {
    // The tabs I1's `showTab:` knows (`_WKInspector.h`); any other identifier is ignored.
    let known = [
        "elements", "network", "sources", "timeline", "storage", "graphics", "layers", "audit",
        "console",
    ];
    let cases = [
        (DevToolsPanel::Elements, "elements"),
        (DevToolsPanel::Console, "console"),
        (DevToolsPanel::Network, "network"),
        (DevToolsPanel::Sources, "sources"),
        (DevToolsPanel::Performance, "timeline"),
        (DevToolsPanel::Application, "storage"),
        (DevToolsPanel::Memory, "timeline"),
    ];

    for (panel, tab) in cases {
        assert_eq!(tab_identifier(panel), tab, "{panel:?}");
        assert!(known.contains(&tab), "{tab}");
    }
}

//! The Extensions pane: two installed extensions, one off on a site; none yet; and a macOS
//! without web extensions. Its switch and buttons ask the browser.

use quickie_domain_settings::ExtensionId;
use quickie_ui_screens::{ExtensionState, ExtensionsState, Pane, SettingsAction};
use quickie_ui_shared::stories::Scene;

use super::{go, open};

const PALETTE: &str = "⌘P";

fn bitwarden() -> ExtensionState {
    ExtensionState {
        id: ExtensionId::from_digest(&[1; 16]),
        name: "Bitwarden Password Manager".to_owned(),
        about: "2025.10.1 · Chrome Web Store".to_owned(),
        enabled: true,
        access: vec![
            "Read and change your data on all websites".to_owned(),
            "Read your browsing history".to_owned(),
            "Keep data on this Mac".to_owned(),
        ],
        off_on: vec!["bank.example".to_owned()],
        on_on: Vec::new(),
    }
}

fn ublock() -> ExtensionState {
    ExtensionState {
        id: ExtensionId::from_digest(&[2; 16]),
        name: "uBlock Origin Lite".to_owned(),
        about: "2025.1105.1 · Chrome Web Store".to_owned(),
        enabled: false,
        access: vec!["Block content on any page".to_owned()],
        off_on: Vec::new(),
        on_on: Vec::new(),
    }
}

fn state(extensions: Vec<ExtensionState>, available: bool, status: &str) -> ExtensionsState {
    ExtensionsState {
        available,
        extensions,
        auto_update: false,
        palette_keys: PALETTE.to_owned(),
        status: status.to_owned(),
    }
}

/// Two extensions; flipping the second's switch and choosing a file ask the browser.
pub fn installed(scene: &mut Scene) {
    let settings = open(scene, None);
    settings.screen.set_extensions(&state(
        vec![bitwarden(), ublock()],
        true,
        "Bitwarden Password Manager updated to 2025.10.1",
    ));
    go(scene, Pane::Extensions, "Extensions");
    let named = scene.accessibility().contains("uBlock Origin Lite");
    scene.check(named, "each extension has its row");
    match scene.view_labelled("Choose…") {
        Some(button) => scene.click(&button),
        None => scene.check(false, "the choose button is labelled"),
    }
    scene.check_eq(
        settings.actions.borrow().clone(),
        vec![SettingsAction::ChooseExtensionFile],
        "choosing asks the browser for a file",
    );
}

pub fn empty(scene: &mut Scene) {
    let settings = open(scene, None);
    settings.screen.set_extensions(&state(Vec::new(), true, ""));
    go(scene, Pane::Extensions, "Extensions");
    let said = scene.accessibility().contains("No extension yet");
    scene.check(said, "an empty list says so");
}

pub fn unavailable(scene: &mut Scene) {
    let settings = open(scene, None);
    settings
        .screen
        .set_extensions(&state(Vec::new(), false, ""));
    go(scene, Pane::Extensions, "Extensions");
    let said = scene
        .accessibility()
        .contains("Extensions need macOS 15.4 or later");
    scene.check(said, "an old macOS is named");
}

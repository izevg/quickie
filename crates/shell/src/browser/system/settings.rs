//! The Settings window behind `⌘,`: made on first use, one instance, brought forward when it
//! is open. Its panes show what the use cases hold, and its buttons and switches go back to
//! them: the default browser, the downloads folder, the filter lists, the agent's endpoint
//! (the agent settings form), and `keybindings.json`.

use std::path::Path;

use objc2_app_kit::{NSWindow, NSWorkspace};
use quickie_application::{Clock, LoadedKeymap, load_keymap};
use quickie_domain_commands::{Catalog, CommandId, Keymap};
use quickie_domain_shields::{FilterListId, Refresh};
use quickie_store::KeybindingsFile;
use quickie_ui_screens::{
    GeneralState, KeysState, ListState, Pane, SettingsAction, SettingsScreen, ShieldsState,
};

use super::super::Browser;
use super::{DefaultBrowserRequest, file_url, is_default_browser, main_bundle};
use crate::{PLATFORM, SystemClock};

/// The keymap of `file` over the defaults, and a line per entry it left out; the defaults and
/// the reason when the file cannot be read.
pub fn keymap_from(file: &KeybindingsFile, catalog: &Catalog) -> (Keymap, Vec<String>) {
    match load_keymap(file, catalog, PLATFORM) {
        Ok(LoadedKeymap { keymap, rejected }) => {
            let problems = rejected
                .iter()
                .map(|bad| {
                    format!(
                        "entry {} (\"{}\" → {}): {}",
                        bad.position + 1,
                        bad.entry.key,
                        bad.entry.command,
                        bad.reason
                    )
                })
                .collect();
            (keymap, problems)
        }
        Err(error) => (
            Keymap::defaults(PLATFORM),
            vec![format!("not read, the defaults apply: {error}")],
        ),
    }
}

impl Browser {
    /// Opens the Settings window on `pane`, or brings it forward on the pane it showed last.
    pub(in crate::browser) fn open_settings(&self, pane: Option<Pane>) {
        let screen = self.settings_screen();
        if !screen.is_visible() {
            self.load_agent_settings();
        }
        self.refresh_settings();
        screen.fit_agent();
        screen.show(pane);
    }

    /// The window, made on first use.
    fn settings_screen(&self) -> &SettingsScreen {
        self.settings.get_or_init(|| {
            let screen = SettingsScreen::new(self.agent_ui.settings.view(), self.mtm);
            let this = self.this.clone();
            screen.on_action(move |action| {
                if let Some(browser) = this.upgrade() {
                    browser.settings_action(action);
                    browser.drain();
                }
            });
            screen
        })
    }

    /// Whether `window` is the Settings window.
    pub(in crate::browser) fn is_settings_window(&self, window: &NSWindow) -> bool {
        self.settings
            .get()
            .is_some_and(|screen| screen.window() == window)
    }

    /// `⌘W` in the Settings window: it closes, and a typed key that was not saved is gone.
    pub(in crate::browser) fn close_settings(&self) {
        if let Some(screen) = self.settings.get() {
            screen.close();
            self.agent_ui.settings.forget_key();
        }
    }

    /// The panes show the settings as they are now; nothing while the window was never made.
    pub(in crate::browser) fn refresh_settings(&self) {
        let Some(screen) = self.settings.get() else {
            return;
        };
        screen.set_general(&self.general_state());
        screen.set_shields(&self.shields_state());
        screen.set_keys(&KeysState {
            file: self.keybindings.path().display().to_string(),
            problems: self.keymap_problems.borrow().clone(),
        });
    }

    fn general_state(&self) -> GeneralState {
        let request = main_bundle().and_then(|bundle| DefaultBrowserRequest::for_bundle(&bundle));
        let (default_browser, can_make_default) = match request {
            None => (
                "This copy runs outside an app bundle, and macOS hands web links only to an \
                 app. Run the bundle (mise run bundle) to make Quickie the default browser."
                    .to_owned(),
                false,
            ),
            Some(request) if is_default_browser(&request.app) => {
                ("Quickie is your default browser.".to_owned(), false)
            }
            Some(_) => (
                format!(
                    "{} opens web links now. macOS asks you before it changes that.",
                    current_browser().unwrap_or_else(|| "Another app".to_owned())
                ),
                true,
            ),
        };
        GeneralState {
            default_browser,
            can_make_default,
            downloads_folder: self.downloads_folder.display().to_string(),
        }
    }

    fn shields_state(&self) -> ShieldsState {
        let now = SystemClock.now();
        let shields = self.shields.borrow();
        let lists = shields
            .lists()
            .iter()
            .map(|list| ListState {
                id: list.id,
                enabled: list.enabled,
                fetched_ago_ms: list.fetched.map(|fetch| now.millis_since(fetch.at)),
            })
            .collect();
        let site_keys = CommandId::try_from("quickie.shields.open")
            .ok()
            .and_then(|id| {
                let keymap = self.keymap.borrow();
                let platform = keymap.platform();
                keymap
                    .bindings_for(&id)
                    .next()
                    .map(|binding| binding.chord.glyphs(platform))
            })
            .unwrap_or_else(|| "the shields popover".to_owned());
        ShieldsState {
            lists,
            keep_fresh: shields.lists().refresh() == Refresh::KeepFresh,
            site_keys,
            status: self.lists_status.borrow().clone(),
        }
    }

    fn settings_action(&self, action: SettingsAction) {
        match action {
            SettingsAction::MakeDefaultBrowser => self.set_default_browser(),
            SettingsAction::ShowDownloads => self.show_folder(&self.downloads_folder),
            SettingsAction::SetList(id, on) => self.set_list(id, on),
            SettingsAction::RefreshLists => {
                let requested = self.shields.borrow().requested_fetches();
                if requested.is_empty() {
                    "Turn a list on first.".clone_into(&mut self.lists_status.borrow_mut());
                }
                self.fetch_lists(requested);
            }
            SettingsAction::KeepListsFresh(on) => {
                let refresh = if on {
                    Refresh::KeepFresh
                } else {
                    Refresh::OnRequest
                };
                let changed = self.shields.borrow_mut().set_refresh(refresh);
                match changed {
                    Ok(_) => {
                        let due = self.shields.borrow().due_fetches();
                        self.fetch_lists(due);
                    }
                    Err(error) => self.report(&error),
                }
            }
            SettingsAction::OpenKeybindings => self.open_keybindings(),
            SettingsAction::ReloadKeybindings => self.reload_keybindings(),
        }
        self.refresh_settings();
    }

    /// One list on or off; a list turned on that never arrived is fetched now.
    fn set_list(&self, id: FilterListId, on: bool) {
        let changed = if on {
            self.shields.borrow_mut().enable_lists(&[id])
        } else {
            self.shields.borrow_mut().disable_lists(&[id])
        };
        if let Err(error) = changed {
            return self.report(&error);
        }
        let due = self.shields.borrow().due_fetches();
        self.fetch_lists(due);
        self.refresh_shields_popover();
    }

    /// Creates `keybindings.json` when missing (on a worker thread: it is disk), then opens it
    /// in the app macOS opens JSON with.
    fn open_keybindings(&self) {
        let file = self.keybindings.clone();
        let work = move || file.create_if_missing().map_err(|error| error.to_string());
        if !self.off_main("quickie-keybindings", work, Self::keybindings_ready) {
            self.toast("Couldn't create keybindings.json");
        }
    }

    fn keybindings_ready(&self, created: Result<(), String>) {
        match created {
            Ok(()) => {
                if !NSWorkspace::sharedWorkspace().openURL(&file_url(self.keybindings.path())) {
                    self.toast("Couldn't open keybindings.json");
                }
            }
            Err(problem) => self.toast(&format!("Couldn't create keybindings.json: {problem}")),
        }
    }

    /// Reads `keybindings.json` again on a worker thread and applies it.
    fn reload_keybindings(&self) {
        let file = self.keybindings.clone();
        let catalog = self.catalog.clone();
        let work = move || keymap_from(&file, &catalog);
        if !self.off_main("quickie-keybindings", work, Self::keybindings_loaded) {
            self.toast("Couldn't read keybindings.json");
        }
    }

    fn keybindings_loaded(&self, (keymap, problems): (Keymap, Vec<String>)) {
        *self.keymap.borrow_mut() = keymap;
        let left_out = problems.len();
        *self.keymap_problems.borrow_mut() = problems;
        self.pending.take();
        self.toast(&match left_out {
            0 => "Key bindings applied".to_owned(),
            1 => "Key bindings applied; 1 entry left out".to_owned(),
            n => format!("Key bindings applied; {n} entries left out"),
        });
        self.refresh_settings();
    }

    fn show_folder(&self, folder: &Path) {
        if !NSWorkspace::sharedWorkspace().openURL(&file_url(folder)) {
            self.toast(&format!("Couldn't show {}", folder.display()));
        }
    }
}

/// The name of the app that opens web links now: `Safari`.
fn current_browser() -> Option<String> {
    let probe = objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(
        "http://example.com/",
    ))?;
    let handler = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&probe)?;
    let path = handler.path()?.to_string();
    Path::new(&path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

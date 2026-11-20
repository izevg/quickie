//! The Settings window behind `⌘,`: made on first use, one instance, brought forward when it
//! is open. Its panes show what the use cases hold, and its buttons and switches go back to
//! them: the default browser, the downloads folder, the filter lists, the agent's endpoint
//! (the agent settings form), and the keymap editor over `keybindings.json`.

use std::path::Path;

use objc2_app_kit::{NSEventModifierFlags, NSWindow, NSWorkspace};
use quickie_application::{
    AppError, Clock, LoadedKeymap, RejectedEntry, Rejection, check_binding, keymap_lines,
    load_keymap, rebind, reset_binding,
};
use quickie_domain_commands::{BindingError, Catalog, Chord, CommandId, Keymap, KeymapError};
use quickie_domain_settings::PageLayout;
use quickie_domain_shields::{FilterListId, Refresh};
use quickie_store::KeybindingsFile;
use quickie_ui_screens::{
    GeneralState, KeyPress, KeysState, ListState, Pane, Recorded, SettingsAction, SettingsScreen,
    ShieldsState,
};

use super::super::Browser;
use super::super::keys::modifiers;
use super::{DefaultBrowserRequest, file_url, is_default_browser, main_bundle};
use crate::input::keystroke::{Read, read};
use crate::{PLATFORM, SystemClock};

/// What the keymap left out of `keybindings.json`: the entries it refused, or why the file
/// was not read at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapProblems {
    pub rejected: Vec<RejectedEntry>,
    pub unread: Option<String>,
}

impl KeymapProblems {
    /// How many things went wrong.
    #[must_use]
    pub fn count(&self) -> usize {
        self.rejected.len() + usize::from(self.unread.is_some())
    }

    /// A line per problem, for the Keyboard pane.
    fn lines(&self) -> Vec<String> {
        self.unread
            .iter()
            .map(|error| format!("not read, the defaults apply: {error}"))
            .chain(self.rejected.iter().map(|bad| {
                format!(
                    "entry {} (\"{}\" → {}): {}",
                    bad.position + 1,
                    bad.entry.key,
                    bad.entry.command,
                    bad.reason
                )
            }))
            .collect()
    }
}

/// The keymap of `file` over the defaults, and the entries it left out; the defaults and the
/// reason when the file cannot be read.
pub fn keymap_from(file: &KeybindingsFile, catalog: &Catalog) -> (Keymap, KeymapProblems) {
    match load_keymap(file, catalog, PLATFORM) {
        Ok(LoadedKeymap { keymap, rejected }) => (
            keymap,
            KeymapProblems {
                rejected,
                unread: None,
            },
        ),
        Err(error) => (
            Keymap::defaults(PLATFORM),
            KeymapProblems {
                rejected: Vec::new(),
                unread: Some(error.to_string()),
            },
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
        screen.show(pane);
    }

    /// The window, made on first use.
    fn settings_screen(&self) -> &SettingsScreen {
        self.settings.get_or_init(|| {
            let screen = SettingsScreen::new(&self.agent_ui.settings, self.mtm);
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
        screen.set_extensions(&self.extensions_state());
        let problems = self.keymap_problems.borrow();
        screen.set_keys(&KeysState {
            file: self.keybindings.path().display().to_string(),
            lines: keymap_lines(&self.keymap.borrow(), &self.catalog, &problems.rejected),
            problems: problems.lines(),
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
            inset_page: self.inset_page(),
        }
    }

    /// Whether *Inset page* is on.
    pub(in crate::browser) fn inset_page(&self) -> bool {
        self.appearance.borrow().appearance().page() == PageLayout::Inset
    }

    /// Turns *Inset page* on or off, saved, and lays out every window's page at once.
    fn set_inset_page(&self, on: bool) {
        let page = if on {
            PageLayout::Inset
        } else {
            PageLayout::EdgeToEdge
        };
        let changed = self.appearance.borrow_mut().set_page(page);
        match changed {
            Ok(true) => {
                for frame in self.frames.borrow().values() {
                    frame.set_inset(on);
                }
            }
            Ok(false) => {}
            Err(error) => self.report(&error),
        }
    }

    fn shields_state(&self) -> ShieldsState {
        let now = SystemClock.now();
        let rules = self.rules_line();
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
            rules,
        }
    }

    fn settings_action(&self, action: SettingsAction) {
        match action {
            SettingsAction::MakeDefaultBrowser => self.set_default_browser(),
            SettingsAction::ShowDownloads => self.show_folder(&self.downloads_folder),
            SettingsAction::SetList(id, on) => self.set_list(id, on),
            SettingsAction::EnableLists => self.enable_lists(),
            SettingsAction::RefreshList(id) => {
                let requested: Vec<_> = self
                    .shields
                    .borrow()
                    .requested_fetches()
                    .into_iter()
                    .filter(|fetch| fetch.list == id)
                    .collect();
                if requested.is_empty() {
                    "Turn the list on first.".clone_into(&mut self.lists_status.borrow_mut());
                }
                self.fetch_lists(requested);
            }
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
            SettingsAction::InsetPage(on) => self.set_inset_page(on),
            SettingsAction::OpenKeybindings => self.open_keybindings(),
            SettingsAction::ReloadKeybindings => self.reload_keybindings(),
            // The recording goes on: nothing else changed, so nothing is refreshed.
            SettingsAction::Record(command, press) => return self.recorded(&command, &press),
            SettingsAction::Rebind(command, chord) => self.rebind(command, chord),
            SettingsAction::ResetBinding(command) => self.reset_binding(command),
            SettingsAction::SetExtension(id, on) => self.set_extension(&id, on),
            SettingsAction::RemoveExtension(id) => self.remove_extension(&id),
            SettingsAction::ChooseExtensionFile => self.install_extension_file(),
            SettingsAction::CheckExtensionUpdates => self.check_extension_updates(),
            SettingsAction::AutoUpdateExtensions(on) => self.auto_update_extensions(on),
        }
        self.refresh_settings();
    }

    /// The first keys bound to `command`, as keycaps: `⌘P`; empty when none are.
    pub(in crate::browser) fn keys_of(&self, command: &str) -> String {
        CommandId::try_from(command)
            .ok()
            .and_then(|id| {
                let keymap = self.keymap.borrow();
                let platform = keymap.platform();
                keymap
                    .bindings_for(&id)
                    .next()
                    .map(|binding| binding.chord.glyphs(platform))
            })
            .unwrap_or_default()
    }

    /// A key pressed while `command` records: read as the keymap reads every press, and
    /// checked against the keymap, so a conflict shows before anything is saved.
    fn recorded(&self, command: &CommandId, press: &KeyPress) {
        let Some(screen) = self.settings.get() else {
            return;
        };
        let function = press.flags.contains(NSEventModifierFlags::Function);
        let recorded = match read(press.code, &press.typed, modifiers(press.flags), function) {
            Read::Stroke(stroke) => {
                let chord = Chord::from(stroke);
                let keymap = self.keymap.borrow();
                let keys = chord.glyphs(keymap.platform());
                let problem = check_binding(&keymap, &self.catalog, command, chord)
                    .err()
                    .map(|refused| self.refusal(&keys, &refused));
                Recorded {
                    chord: Some(chord),
                    keys,
                    problem,
                }
            }
            Read::Unnamed | Read::System => Recorded {
                chord: None,
                keys: String::new(),
                problem: Some("Quickie can't bind that key".to_owned()),
            },
        };
        screen.show_recorded(command, recorded);
    }

    /// Why the keymap refuses `keys`, in the pane's words.
    fn refusal(&self, keys: &str, refused: &Rejection) -> String {
        match refused {
            Rejection::Invalid(KeymapError::Conflict(conflict)) => {
                let other = self.catalog.get(&conflict.first).map_or_else(
                    || conflict.first.to_string(),
                    |other| other.title.to_owned(),
                );
                format!("{keys} already runs {other}")
            }
            Rejection::Malformed(BindingError::Typing(_)) => {
                format!("{keys} types text; add ⌘, ⌥ or ⌃")
            }
            other => other.to_string(),
        }
    }

    /// Binds `command` to `chord` in `keybindings.json`, on a worker thread (it is disk).
    fn rebind(&self, command: CommandId, chord: Chord) {
        let file = self.keybindings.clone();
        let catalog = self.catalog.clone();
        let work = move || {
            rebind(&file, &catalog, PLATFORM, &command, chord).map_err(|error| error.to_string())
        };
        if !self.off_main("quickie-keybindings", work, Self::keymap_edited) {
            self.toast("Couldn't save keybindings.json");
        }
    }

    /// Gives `command` its defaults back in `keybindings.json`, on a worker thread.
    fn reset_binding(&self, command: CommandId) {
        let file = self.keybindings.clone();
        let catalog = self.catalog.clone();
        let work = move || {
            reset_binding(&file, &catalog, PLATFORM, &command)
                .map_err(|error: AppError| error.to_string())
        };
        if !self.off_main("quickie-keybindings", work, Self::keymap_edited) {
            self.toast("Couldn't save keybindings.json");
        }
    }

    /// The keymap editor saved `keybindings.json`: its keymap applies at once.
    fn keymap_edited(&self, edited: Result<LoadedKeymap, String>) {
        match edited {
            Ok(LoadedKeymap { keymap, rejected }) => {
                *self.keymap.borrow_mut() = keymap;
                *self.keymap_problems.borrow_mut() = KeymapProblems {
                    rejected,
                    unread: None,
                };
                self.pending.take();
            }
            Err(problem) => self.toast(&format!("Couldn't save keybindings.json: {problem}")),
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
        match changed {
            Err(error) => return self.report(&error),
            Ok(changed) if !changed.is_empty() => self.lists_changed_by_user(),
            Ok(_) => {}
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

    fn keybindings_loaded(&self, (keymap, problems): (Keymap, KeymapProblems)) {
        *self.keymap.borrow_mut() = keymap;
        let left_out = problems.count();
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

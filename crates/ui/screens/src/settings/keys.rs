//! Keys: where the user's `keybindings.json` is, buttons to open it in the default editor and
//! to apply it again, and the entries that were left out and why.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSButton, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use quickie_ui_shared::tokens::{TextRole, color, space};

use super::{
    Column, PaneView, SettingsTarget, button, heading, label, paragraph, set, wrapped_height,
};

/// What the Keys pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysState {
    /// The path of `keybindings.json`.
    pub file: String,
    /// One line per entry that was left out, or why the file was not read.
    pub problems: Vec<String>,
}

pub struct Keys {
    view: Retained<PaneView>,
    intro: Retained<NSTextField>,
    file: Retained<NSTextField>,
    open: Retained<NSButton>,
    reload: Retained<NSButton>,
    problems_heading: Retained<NSTextField>,
    problems: Retained<NSTextField>,
}

impl Keys {
    pub fn new(target: &SettingsTarget, mtm: MainThreadMarker) -> Self {
        let keys = Self {
            view: PaneView::new(mtm),
            intro: paragraph(
                "Your key bindings live in keybindings.json, in VS Code's format: a list of \
                 { \"key\", \"command\", \"when\" } entries, \"mod\" for ⌘, and \"-command\" \
                 to remove a default. Reload applies your changes.",
                mtm,
            ),
            file: label("", TextRole::RowTitle, &color::label(), mtm),
            open: button("Open keybindings.json", sel!(openKeybindings:), target, mtm),
            reload: button("Reload", sel!(reloadKeybindings:), target, mtm),
            problems_heading: heading("Left out", mtm),
            problems: paragraph("", mtm),
        };
        keys.file.setFont(Some(&TextRole::Caption.mono_font()));
        keys.problems.setFont(Some(&TextRole::Caption.mono_font()));
        for view in [
            &*keys.intro as &NSView,
            &keys.file,
            &keys.open,
            &keys.reload,
            &keys.problems_heading,
            &keys.problems,
        ] {
            keys.view.addSubview(view);
        }
        keys
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Shows `state` and lays the pane out; returns its height.
    pub fn update(&self, state: &KeysState) -> CGFloat {
        set(&self.file, &state.file);
        if state.problems.is_empty() {
            set(&self.problems, "Nothing: every entry applies.");
            self.problems.setTextColor(Some(&color::secondary_label()));
        } else {
            set(&self.problems, &state.problems.join("\n"));
            self.problems.setTextColor(Some(&color::destructive()));
        }
        let mut column = Column::new();
        column.full(&self.intro, wrapped_height(&self.intro));
        column.gap(space::SM);
        column.full(&self.file, (TextRole::Caption.point_size() * 1.5).ceil());
        column.gap(space::SM);
        column.buttons(&[&self.open, &self.reload]);
        column.gap(space::SECTION);
        column.full(
            &self.problems_heading,
            self.problems_heading.frame().size.height,
        );
        column.gap(space::XS);
        column.full(&self.problems, wrapped_height(&self.problems));
        column.height()
    }
}

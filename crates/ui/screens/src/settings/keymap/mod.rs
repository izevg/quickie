//! Keyboard: the keymap editor. A search over every command and its keys, the buttons that
//! open `keybindings.json` and apply it again, the table of bindings with where each comes
//! from (a user entry the keymap refused is a red row that says what its keys already run),
//! and a footer with the table's keys and the entries that were left out.
//!
//! - [`table`]: the table, its keys, and the recording.
//! - [`row`]: one binding as a row (`binding_row`).

mod row;
mod table;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSBorderType, NSScrollView, NSSearchField, NSTableViewColumnAutoresizingStyle,
    NSTableViewGridLineStyle, NSTableViewStyle, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_application::BindingLine;
use quickie_domain_commands::{Chord, CommandId};
use quickie_ui_shared::tokens::{TextRole, color, space};

use super::chrome::{HEIGHT, PANE_WIDTH, Page, SettingsTarget, button, hairline, label, set};
use row::Column;
use table::{KeymapSource, KeymapTable};

/// What the Keyboard pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysState {
    /// The path of `keybindings.json`.
    pub file: String,
    /// Every binding, as the keymap editor lists them.
    pub lines: Vec<BindingLine>,
    /// One line per entry that was left out, or why the file was not read.
    pub problems: Vec<String>,
}

/// What the browser made of a key press while a command was recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The chord the press is; `None` for a key the keymap cannot name.
    pub chord: Option<Chord>,
    /// The chord in glyphs, for the keycaps: `⇧⌘E`.
    pub keys: String,
    /// Why the keymap would refuse it: "⌘T already runs Open New Tab".
    pub problem: Option<String>,
}

pub struct Keys {
    page: Page,
    _search: Retained<NSSearchField>,
    open: Retained<objc2_app_kit::NSButton>,
    _table: Retained<KeymapTable>,
    source: Retained<KeymapSource>,
    hint: Retained<NSTextField>,
    problems: Retained<NSTextField>,
}

impl Keys {
    /// The footer's height.
    const FOOTER: CGFloat = 40.0;

    pub fn new(target: &Retained<SettingsTarget>, mtm: MainThreadMarker) -> Self {
        let page = Page::new("Keyboard", mtm);
        let search = NSSearchField::new(mtm);
        search.setPlaceholderString(Some(&NSString::from_str("Search commands and keys")));
        search.setSendsSearchStringImmediately(true);
        let open = button("Open keybindings.json", sel!(openKeybindings:), target, mtm);
        let reload = button("Reload", sel!(reloadKeybindings:), target, mtm);
        reload.setToolTip(Some(&NSString::from_str(
            "Read keybindings.json again and apply it",
        )));
        let hint = label("", TextRole::Caption, &color::secondary_label(), mtm);
        let problems = label("", TextRole::Caption, &color::destructive(), mtm);
        let (table, source) = KeymapSource::new(target.clone(), &search, hint.clone(), mtm);
        for kind in Column::ALL {
            table.addTableColumn(&table::column(kind, mtm));
        }
        table.setStyle(NSTableViewStyle::FullWidth);
        table.setUsesAlternatingRowBackgroundColors(true);
        table.setGridStyleMask(NSTableViewGridLineStyle::empty());
        table.setIntercellSpacing(NSSize::new(0.0, 0.0));
        table.setAllowsColumnReordering(false);
        table.setAllowsColumnResizing(false);
        table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::NoColumnAutoresizing);

        // A toolbar row under the header: the search, then the file's buttons.
        let row_top = Page::HEADER + 1.0 + space::MD;
        let button_height = open.frame().size.height;
        let search_height = 24.0_f64.max(search.frame().size.height);
        let row_height = button_height.max(search_height);
        let mut right = PANE_WIDTH - space::XL;
        for button in [&reload, &open] {
            let size = button.frame().size;
            right -= size.width;
            button.setFrameOrigin(NSPoint::new(
                right,
                row_top + ((row_height - size.height) / 2.0).round(),
            ));
            right -= space::SM;
        }
        search.setFrame(NSRect::new(
            NSPoint::new(
                space::XL,
                row_top + ((row_height - search_height) / 2.0).round(),
            ),
            NSSize::new(right - space::XL - space::SM, search_height),
        ));

        // The table, edge to edge, between the toolbar row and the footer.
        let table_top = row_top + row_height + space::MD;
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, table_top),
                NSSize::new(PANE_WIDTH, HEIGHT - Self::FOOTER - table_top),
            ),
        );
        scroll.setDocumentView(Some(&table));
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        table.setFrameSize(NSSize::new(PANE_WIDTH, table.frame().size.height));

        // The footer: what the table's keys do, and what was left out of the file.
        let footer_top = HEIGHT - Self::FOOTER;
        let line = hairline(
            NSRect::new(
                NSPoint::new(0.0, footer_top - 1.0),
                NSSize::new(PANE_WIDTH, 1.0),
            ),
            mtm,
        );
        let text_height = hint.frame().size.height;
        let text_top = footer_top + ((Self::FOOTER - text_height) / 2.0).round();
        hint.setFrame(NSRect::new(
            NSPoint::new(space::XL, text_top),
            NSSize::new(PANE_WIDTH - space::XL * 2.0, text_height),
        ));
        problems.setFrame(NSRect::new(
            NSPoint::new(PANE_WIDTH / 2.0, text_top),
            NSSize::new(PANE_WIDTH / 2.0 - space::XL, text_height),
        ));
        problems.setAlignment(objc2_app_kit::NSTextAlignment::Right);

        for view in [
            &*search as &NSView,
            &open,
            &reload,
            &scroll,
            &line,
            &hint,
            &problems,
        ] {
            page.view().addSubview(view);
        }
        Self {
            page,
            _search: search,
            open,
            _table: table,
            source,
            hint,
            problems,
        }
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    /// Shows `state`; a recording goes on when the lines are the same.
    pub fn update(&self, state: &KeysState) {
        self.open.setToolTip(Some(&NSString::from_str(&state.file)));
        let (text, tip) = match state.problems.len() {
            0 => (String::new(), String::new()),
            1 => (
                "1 entry of keybindings.json is left out".to_owned(),
                state.problems.join("\n"),
            ),
            n => (
                format!("{n} entries of keybindings.json are left out"),
                state.problems.join("\n"),
            ),
        };
        set(&self.problems, &text);
        self.problems.setToolTip(
            (!tip.is_empty())
                .then(|| NSString::from_str(&tip))
                .as_deref(),
        );
        // The hint makes room for the problems at the trailing end.
        let frame = self.problems.frame();
        let width = if text.is_empty() {
            0.0
        } else {
            self.problems.sizeToFit();
            self.problems.frame().size.width.ceil()
        };
        let right = PANE_WIDTH - space::XL;
        self.problems.setFrame(NSRect::new(
            NSPoint::new(right - width, frame.origin.y),
            NSSize::new(width, frame.size.height),
        ));
        let hint = self.hint.frame();
        self.hint.setFrameSize(NSSize::new(
            right - hint.origin.x - width - if width > 0.0 { space::MD } else { 0.0 },
            hint.size.height,
        ));
        self.source.set_lines(state.lines.clone());
    }

    /// What the browser made of the last press while `command` was recording.
    pub fn show_recorded(&self, command: &CommandId, recorded: Recorded) {
        self.source.show_recorded(command, recorded);
    }

    /// A recording, if any, gives up: the window closed or another pane showed.
    pub fn stop_recording(&self) {
        self.source.stop_recording();
    }

    /// The search takes the keyboard.
    pub fn focus_search(&self) {
        self.source.focus_search();
    }
}

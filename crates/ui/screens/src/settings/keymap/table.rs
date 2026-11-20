//! The keymap table and what it shows: every binding line the search keeps, one of them maybe
//! recording.
//!
//! The table takes the keyboard: `⏎` or a double-click records the selected line's keys, `⌫`
//! gives its command the defaults back, `⌘F` goes to the search. While it records, every key
//! press is the recording's, `⌘`-shortcuts too, until `⏎` saves the keys or `⎋` (or a click on
//! another line, or the focus leaving) gives up. A press is handed to the browser, which reads
//! it as the keymap does and says whether the keymap would take it.

use std::cell::RefCell;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAnimationContext, NSBeep, NSControl, NSControlTextEditingDelegate, NSEvent,
    NSEventModifierFlags, NSResponder, NSSearchField, NSSearchFieldDelegate, NSTableColumn,
    NSTableRowView, NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTextField,
    NSTextFieldDelegate, NSTextView, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSIndexSet, NSInteger, NSNotification, NSSize, NSString};
use quickie_application::{BindingLine, BindingSource};
use quickie_domain_commands::CommandId;
use quickie_ui_shared::tokens::color;

use super::Recorded;
use super::row::{self, Column, Metrics, RowState};
use crate::settings::chrome::row_view::{RowView, Tone};
use crate::settings::chrome::{SettingsTarget, set};
use crate::settings::{KeyPress, SettingsAction};

/// Key codes the table reads by position (`kVK_*` in Carbon's `Events.h`).
mod code {
    pub const RETURN: u16 = 0x24;
    pub const KEYPAD_ENTER: u16 = 0x4C;
    pub const ESCAPE: u16 = 0x35;
    pub const DELETE: u16 = 0x33;
    pub const FORWARD_DELETE: u16 = 0x75;
    /// The `F` key on a US layout.
    pub const F: u16 = 0x03;
}

/// The hint under the table, at rest and while recording.
const HINT: &str = "⏎ or double-click records new keys · ⌫ restores the defaults · ⌘F searches";
const HINT_RECORDING: &str = "Type the new keys · ⏎ saves · ⎋ cancels";

/// How a key press reached the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    /// `keyDown:`, as the first responder.
    Down,
    /// `performKeyEquivalent:`, which the window offers every view, focused or not.
    Equivalent,
}

/// A line that waits for keys, and what was pressed so far.
struct Recording {
    line: usize,
    recorded: Option<Recorded>,
}

/// The lines, which of them the search keeps, and the recording.
#[derive(Default)]
struct Model {
    lines: Vec<BindingLine>,
    query: String,
    /// Indexes into `lines`, in order.
    visible: Vec<usize>,
    recording: Option<Recording>,
}

impl Model {
    fn refilter(&mut self) {
        let query = &self.query;
        self.visible = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.matches(query))
            .map(|(index, _)| index)
            .collect();
    }

    fn line_at(&self, row: NSInteger) -> Option<(usize, &BindingLine)> {
        let index = *self.visible.get(usize::try_from(row).ok()?)?;
        Some((index, self.lines.get(index)?))
    }

    fn row_of(&self, line: usize) -> Option<usize> {
        self.visible.iter().position(|&index| index == line)
    }

    fn state(&self, line: usize) -> RowState<'_> {
        match &self.recording {
            Some(recording) if recording.line == line => {
                RowState::Recording(recording.recorded.as_ref())
            }
            _ => RowState::Idle,
        }
    }
}

pub struct TableIvars {
    source: RefCell<Option<Retained<KeymapSource>>>,
}

define_class!(
    // SAFETY: `NSTableView` has no subclassing requirements, and `KeymapTable` implements no
    // `Drop`; `keyDown:`, `performKeyEquivalent:`, and `resignFirstResponder` are public
    // responder API meant to be overridden, and each calls the superclass when it does not
    // handle the event.
    #[unsafe(super(NSTableView, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieKeymapTable"]
    #[ivars = TableIvars]
    pub struct KeymapTable;

    impl KeymapTable {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.source().is_some_and(|source| source.key(event, Press::Down)) {
                // SAFETY: the superclass moves the selection and handles the rest.
                let () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let handled = self
                .source()
                .is_some_and(|source| source.key(event, Press::Equivalent));
            // SAFETY: the superclass offers the event to the table's own views.
            handled || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            // SAFETY: the superclass gives up the focus.
            let resigned: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if resigned && let Some(source) = self.source() {
                source.stop_recording();
            }
            resigned
        }
    }
);

impl KeymapTable {
    fn source(&self) -> Option<Retained<KeymapSource>> {
        self.ivars().source.borrow().clone()
    }
}

pub struct SourceIvars {
    model: RefCell<Model>,
    metrics: Metrics,
    target: Retained<SettingsTarget>,
    hint: Retained<NSTextField>,
    table: RefCell<Weak<KeymapTable>>,
    search: RefCell<Weak<NSSearchField>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `KeymapSource` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieKeymapSource"]
    #[ivars = SourceIvars]
    pub struct KeymapSource;

    unsafe impl NSObjectProtocol for KeymapSource {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSTableViewDataSource for KeymapSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            NSInteger::try_from(self.ivars().model.borrow().visible.len()).unwrap_or_default()
        }
    }

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSControlTextEditingDelegate for KeymapSource {
        /// In the search field, `↓` and `⏎` go down to the table.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn search_command(&self, _control: &NSControl, _text: &NSTextView, command: Sel) -> bool {
            let down = command == sel!(moveDown:) || command == sel!(insertNewline:);
            if down {
                self.focus_table();
            }
            down
        }
    }

    unsafe impl NSTextFieldDelegate for KeymapSource {}

    unsafe impl NSSearchFieldDelegate for KeymapSource {}

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl NSTableViewDelegate for KeymapSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            _table: &NSTableView,
            column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            self.cell(column, row)
        }

        #[unsafe(method(tableView:heightOfRow:))]
        fn height_of_row(&self, _table: &NSTableView, row: NSInteger) -> CGFloat {
            let model = self.ivars().model.borrow();
            model.line_at(row).map_or(1.0, |(index, line)| {
                self.ivars().metrics.row_height(line, model.state(index))
            })
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view(&self, _table: &NSTableView, row: NSInteger) -> Option<Retained<NSTableRowView>> {
            let model = self.ivars().model.borrow();
            let line = model.line_at(row);
            let tone = line.map_or(Tone::Plain, |(index, line)| model.state(index).tone(line));
            let view = RowView::new(tone, self.mtm());
            // The row reads whole: `VoiceOver` stops at rows, not at their cells.
            if let Some((index, line)) = line {
                let spoken = row::spoken(line, model.state(index));
                view.setAccessibilityLabel(Some(&NSString::from_str(&spoken)));
            }
            Some(view.into_super())
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, _notification: &NSNotification) {
            let recording = self.ivars().model.borrow().recording.as_ref().map(|r| r.line);
            if recording.is_some() && recording != self.selected_line() {
                self.stop_recording();
            }
        }
    }

    impl KeymapSource {
        #[unsafe(method(rowDoubleClicked:))]
        fn row_double_clicked(&self, table: &NSTableView) {
            if table.clickedRow() >= 0 {
                self.start_recording();
            }
        }

        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, field: &NSSearchField) {
            self.search(&field.stringValue().to_string());
        }
    }
);

impl KeymapSource {
    /// The table and its source, wired to each other and to `search`; the hint under the
    /// table follows what the table is doing.
    pub fn new(
        target: Retained<SettingsTarget>,
        search: &NSSearchField,
        hint: Retained<NSTextField>,
        mtm: MainThreadMarker,
    ) -> (Retained<KeymapTable>, Retained<Self>) {
        let table: Retained<KeymapTable> = {
            let this = KeymapTable::alloc(mtm).set_ivars(TableIvars {
                source: RefCell::new(None),
            });
            // SAFETY: `initWithFrame:` is `NSTableView`'s designated initializer and the ivars
            // are set.
            unsafe { msg_send![super(this), initWithFrame: objc2_foundation::NSRect::ZERO] }
        };
        let source = {
            let this = Self::alloc(mtm).set_ivars(SourceIvars {
                model: RefCell::new(Model::default()),
                metrics: Metrics::measure(mtm),
                target,
                hint,
                table: RefCell::new(Weak::from_retained(&table)),
                search: RefCell::new(Weak::new(search)),
            });
            let source: Retained<Self> =
                // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
                unsafe { msg_send![super(this), init] };
            source
        };
        *table.ivars().source.borrow_mut() = Some(source.clone());
        // SAFETY: the table holds its data source and delegate weakly; the table itself keeps
        // the source alive.
        unsafe { table.setDataSource(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above.
        unsafe { table.setDelegate(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above, and the source implements `rowDoubleClicked:`.
        unsafe { table.setTarget(Some(&source)) };
        // SAFETY: as above.
        unsafe { table.setDoubleAction(Some(sel!(rowDoubleClicked:))) };
        // SAFETY: the field holds its delegate weakly; the table keeps the source alive as
        // long as the pane keeps both.
        unsafe { search.setDelegate(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above, and the source implements `searchChanged:`.
        unsafe { search.setTarget(Some(&source)) };
        // SAFETY: as above.
        unsafe { search.setAction(Some(sel!(searchChanged:))) };
        source.show_hint();
        (table, source)
    }

    fn table(&self) -> Option<Retained<KeymapTable>> {
        self.ivars().table.borrow().load()
    }

    fn cell(&self, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
        let column = column?;
        let kind = Column::from_identifier(&column.identifier().to_string())?;
        let model = self.ivars().model.borrow();
        let (index, line) = model.line_at(row)?;
        let state = model.state(index);
        let height = self.ivars().metrics.row_height(line, state);
        Some(row::cell(
            kind,
            line,
            state,
            NSSize::new(column.width(), height),
            self.ivars().metrics,
            self.mtm(),
        ))
    }

    /// Shows `lines`; nothing changes when they are the ones shown, so a refresh for another
    /// pane does not stop a recording.
    pub fn set_lines(&self, lines: Vec<BindingLine>) {
        let selected = self.selected_command();
        {
            let mut model = self.ivars().model.borrow_mut();
            if model.lines == lines {
                return;
            }
            model.lines = lines;
            model.recording = None;
            model.refilter();
        }
        self.reload();
        if let Some(command) = selected {
            let row = {
                let model = self.ivars().model.borrow();
                model
                    .visible
                    .iter()
                    .position(|&index| model.lines[index].command == command)
            };
            if let Some(row) = row {
                self.select_row(row);
            }
        }
        self.show_hint();
    }

    fn search(&self, query: &str) {
        {
            let mut model = self.ivars().model.borrow_mut();
            query.clone_into(&mut model.query);
            model.recording = None;
            model.refilter();
        }
        self.reload();
        if !self.ivars().model.borrow().visible.is_empty() {
            self.select_row(0);
        }
        self.show_hint();
    }

    fn reload(&self) {
        if let Some(table) = self.table() {
            table.reloadData();
        }
    }

    fn select_row(&self, row: usize) {
        if let Some(table) = self.table() {
            table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false);
            table.scrollRowToVisible(NSInteger::try_from(row).unwrap_or_default());
        }
    }

    fn selected_line(&self) -> Option<usize> {
        let row = self.table()?.selectedRow();
        self.ivars()
            .model
            .borrow()
            .line_at(row)
            .map(|(index, _)| index)
    }

    fn selected_command(&self) -> Option<CommandId> {
        let line = self.selected_line()?;
        Some(self.ivars().model.borrow().lines.get(line)?.command.clone())
    }

    fn focus_table(&self) {
        let Some(table) = self.table() else {
            return;
        };
        if let Some(window) = table.window() {
            window.makeFirstResponder(Some(&table));
        }
        if table.selectedRow() < 0 && !self.ivars().model.borrow().visible.is_empty() {
            self.select_row(0);
        }
    }

    pub fn focus_search(&self) {
        let search = self.ivars().search.borrow().load();
        if let Some(search) = search
            && let Some(window) = search.window()
        {
            window.makeFirstResponder(Some(&search));
        }
    }

    /// The selected line waits for keys.
    fn start_recording(&self) {
        let Some(line) = self.selected_line() else {
            return;
        };
        self.ivars().model.borrow_mut().recording = Some(Recording {
            line,
            recorded: None,
        });
        self.focus_table();
        self.redraw(line);
        self.show_hint();
    }

    /// The recording, if any, gives up.
    pub fn stop_recording(&self) {
        let stopped = self.ivars().model.borrow_mut().recording.take();
        if let Some(stopped) = stopped {
            self.redraw(stopped.line);
            self.show_hint();
        }
    }

    /// What the browser made of the press while `command` was recording.
    pub fn show_recorded(&self, command: &CommandId, recorded: Recorded) {
        let line = {
            let mut model = self.ivars().model.borrow_mut();
            let Some(recording) = model.recording.as_ref() else {
                return;
            };
            let line = recording.line;
            if model.lines.get(line).map(|line| &line.command) != Some(command) {
                return;
            }
            if let Some(recording) = model.recording.as_mut() {
                recording.recorded = Some(recorded);
            }
            line
        };
        self.redraw(line);
    }

    /// Draws `line`'s row again: its cells, its tint, and its height.
    fn redraw(&self, line: usize) {
        let (Some(table), Some(row)) = (self.table(), self.ivars().model.borrow().row_of(line))
        else {
            return;
        };
        let rows = NSIndexSet::indexSetWithIndex(row);
        let tone = {
            let model = self.ivars().model.borrow();
            model.state(line).tone(&model.lines[line])
        };
        let index = NSInteger::try_from(row).unwrap_or_default();
        if let Some(view) = table
            .rowViewAtRow_makeIfNecessary(index, false)
            .and_then(|view| view.downcast::<RowView>().ok())
        {
            view.set_tone(tone);
        }
        // A keyboard-initiated change never waits for an animation: the row takes its new
        // height at once.
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(0.0);
        table.noteHeightOfRowsWithIndexesChanged(&rows);
        NSAnimationContext::endGrouping();
        let columns = NSIndexSet::indexSetWithIndexesInRange(objc2_foundation::NSRange::new(
            0,
            Column::ALL.len(),
        ));
        table.reloadDataForRowIndexes_columnIndexes(&rows, &columns);
    }

    fn show_hint(&self) {
        let hint = &self.ivars().hint;
        if self.ivars().model.borrow().recording.is_some() {
            set(hint, HINT_RECORDING);
            hint.setTextColor(Some(&color::accent()));
        } else {
            set(hint, HINT);
            hint.setTextColor(Some(&color::secondary_label()));
        }
    }

    /// Handles a key press if it is the table's.
    fn key(&self, event: &NSEvent, press: Press) -> bool {
        let Some(table) = self.table() else {
            return false;
        };
        let focused = table
            .window()
            .and_then(|window| window.firstResponder())
            .is_some_and(|responder| {
                std::ptr::eq(
                    Retained::as_ptr(&responder).cast::<AnyObject>(),
                    Retained::as_ptr(&table).cast::<AnyObject>(),
                )
            });
        let code = event.keyCode();
        let flags = event.modifierFlags() & shortcut_flags();
        let plain = flags.is_empty();
        let recording = self.ivars().model.borrow().recording.is_some();
        if recording {
            if !focused {
                return false;
            }
            match code {
                code::ESCAPE if plain => self.stop_recording(),
                code::RETURN | code::KEYPAD_ENTER if plain => self.save(),
                _ => self.record(event),
            }
            return true;
        }
        if flags == NSEventModifierFlags::Command && is_f(event, code) {
            self.focus_search();
            return true;
        }
        if press == Press::Equivalent || !plain {
            return false;
        }
        match code {
            code::RETURN | code::KEYPAD_ENTER => {
                self.start_recording();
                true
            }
            code::DELETE | code::FORWARD_DELETE => {
                self.reset();
                true
            }
            _ => false,
        }
    }

    /// Hands a press to the browser while recording.
    fn record(&self, event: &NSEvent) {
        let command = {
            let model = self.ivars().model.borrow();
            let Some(recording) = model.recording.as_ref() else {
                return;
            };
            model.lines[recording.line].command.clone()
        };
        let press = KeyPress {
            code: event.keyCode(),
            typed: event
                .charactersIgnoringModifiers()
                .map_or_else(String::new, |typed| typed.to_string()),
            flags: event.modifierFlags(),
        };
        self.ivars()
            .target
            .ask(SettingsAction::Record(command, press));
    }

    /// `⏎` while recording: keys the keymap takes are saved; anything else beeps.
    fn save(&self) {
        let rebind = {
            let model = self.ivars().model.borrow();
            model.recording.as_ref().and_then(|recording| {
                let recorded = recording.recorded.as_ref()?;
                if recorded.problem.is_some() {
                    return None;
                }
                Some((model.lines[recording.line].command.clone(), recorded.chord?))
            })
        };
        match rebind {
            Some((command, chord)) => {
                self.stop_recording();
                self.ivars()
                    .target
                    .ask(SettingsAction::Rebind(command, chord));
            }
            None => NSBeep(),
        }
    }

    /// `⌫`: the selected line's command gets its defaults back, when the user changed any of
    /// its bindings; otherwise there is nothing to restore, and it beeps.
    fn reset(&self) {
        let command = self.selected_line().and_then(|line| {
            let model = self.ivars().model.borrow();
            let command = &model.lines.get(line)?.command;
            let changed = model
                .lines
                .iter()
                .any(|line| line.command == *command && line.source == BindingSource::User);
            changed.then(|| command.clone())
        });
        match command {
            Some(command) => self
                .ivars()
                .target
                .ask(SettingsAction::ResetBinding(command)),
            None => NSBeep(),
        }
    }
}

/// The modifiers a shortcut is made of.
fn shortcut_flags() -> NSEventModifierFlags {
    NSEventModifierFlags::Command
        | NSEventModifierFlags::Shift
        | NSEventModifierFlags::Option
        | NSEventModifierFlags::Control
}

/// Whether the press is the `F` key: the letter the layout types when it is a Latin one, the
/// key's US position otherwise, as the keymap reads letters.
fn is_f(event: &NSEvent, code: u16) -> bool {
    let typed = event
        .charactersIgnoringModifiers()
        .map_or_else(String::new, |typed| typed.to_string());
    let mut chars = typed.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => c.eq_ignore_ascii_case(&'f'),
        _ => code == code::F,
    }
}

/// A column of the table, titled.
pub fn column(kind: Column, mtm: MainThreadMarker) -> Retained<NSTableColumn> {
    let column = NSTableColumn::initWithIdentifier(
        NSTableColumn::alloc(mtm),
        &NSString::from_str(kind.identifier()),
    );
    column.setTitle(&NSString::from_str(kind.title()));
    column.setWidth(kind.width());
    column
}

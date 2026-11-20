//! One binding as a row of the keymap table (`docs/design/inventory.md`, `binding_row`; it
//! lives in this slice until an `entities` crate exists): the command's title and id, its keys
//! as keycaps, the clause, and where the binding comes from. Default, recording (an accent box
//! that waits for keys), conflict (red: the keys already run another command), and user or
//! default source. The row's tint and selection are the window's own row view's.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSImageView, NSLineBreakMode, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_application::{BindingLine, BindingSource};
use quickie_ui_shared::form::Paint;
use quickie_ui_shared::tokens::{TextRole, color, radius, space};
use quickie_ui_shared::{KeycapStyle, Plate, keycap_group_styled, symbol};

use super::Recorded;
use crate::settings::chrome::row_view::Tone;
use crate::settings::chrome::{Flipped, label};

/// The table's columns, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Command,
    Keys,
    When,
    Source,
}

impl Column {
    pub const ALL: [Self; 4] = [Self::Command, Self::Keys, Self::When, Self::Source];

    pub fn title(self) -> &'static str {
        match self {
            Self::Command => "Command",
            Self::Keys => "Keys",
            Self::When => "When",
            Self::Source => "Source",
        }
    }

    /// Its width; together they fill the pane.
    pub fn width(self) -> CGFloat {
        match self {
            Self::Command => 252.0,
            Self::Keys => 148.0,
            Self::When => 156.0,
            Self::Source => 84.0,
        }
    }

    pub fn identifier(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Keys => "keys",
            Self::When => "when",
            Self::Source => "source",
        }
    }

    pub fn from_identifier(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|column| column.identifier() == identifier)
    }
}

/// What a row shows besides its line.
#[derive(Clone, Copy, Debug)]
pub enum RowState<'a> {
    Idle,
    /// Waiting for keys; what was pressed so far, if anything.
    Recording(Option<&'a Recorded>),
}

impl RowState<'_> {
    /// The line under the title that says what is wrong, if anything is.
    fn problem(self, line: &BindingLine) -> Option<String> {
        match self {
            Self::Recording(recorded) => recorded.and_then(|recorded| recorded.problem.clone()),
            Self::Idle => line
                .conflict
                .as_ref()
                .map(|other| format!("Already runs {other}")),
        }
    }

    pub fn tone(self, line: &BindingLine) -> Tone {
        match (self, self.problem(line)) {
            (_, Some(_)) => Tone::Conflict,
            (Self::Recording(_), None) => Tone::Recording,
            (Self::Idle, None) => Tone::Plain,
        }
    }
}

/// Line heights, measured once from the current text size.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    title: CGFloat,
    detail: CGFloat,
}

impl Metrics {
    pub fn measure(mtm: MainThreadMarker) -> Self {
        let line = |role: TextRole, mono: bool| {
            let field = NSTextField::labelWithString(&NSString::from_str("Ag"), mtm);
            let font = if mono { role.mono_font() } else { role.font() };
            field.setFont(Some(&font));
            field.sizeToFit();
            field.frame().size.height.ceil()
        };
        Self {
            title: line(TextRole::RowTitle, false),
            detail: line(TextRole::Caption, true),
        }
    }

    /// A row's height: two lines, or three with a problem under them.
    pub fn row_height(self, line: &BindingLine, state: RowState<'_>) -> CGFloat {
        let problem = if state.problem(line).is_some() {
            self.detail + 2.0
        } else {
            0.0
        };
        space::SM * 2.0 + self.title + 2.0 + self.detail + problem
    }
}

/// The cell of `line` in `column`, `width` by `height`.
pub fn cell(
    column: Column,
    line: &BindingLine,
    state: RowState<'_>,
    size: NSSize,
    metrics: Metrics,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let view = Flipped::new(NSRect::new(NSPoint::ZERO, size), mtm);
    match column {
        Column::Command => command(&view, line, state, metrics, mtm),
        Column::Keys => keys(&view, line, state, mtm),
        Column::When => when(&view, line, mtm),
        Column::Source => source(&view, line, state, mtm),
    }
    view.into_super()
}

/// Title, id, and what is wrong.
fn command(
    view: &NSView,
    line: &BindingLine,
    state: RowState<'_>,
    metrics: Metrics,
    mtm: MainThreadMarker,
) {
    let problem = state.problem(line);
    let (title_tint, id_tint) = if problem.is_some() {
        (color::destructive(), color::destructive())
    } else {
        (color::label(), color::secondary_label())
    };
    let width = view.frame().size.width - space::MD * 2.0;
    let mut x = space::MD;
    let mut top = space::SM;
    if problem.is_some()
        && let Some(warning) = warning_symbol(mtm)
    {
        let size = warning.frame().size;
        warning.setFrameOrigin(NSPoint::new(
            x,
            top + ((metrics.title - size.height) / 2.0).round(),
        ));
        view.addSubview(&warning);
        x += size.width + space::XS;
    }
    let title = text(line.title, TextRole::RowTitle, false, &title_tint, mtm);
    title.setFrame(NSRect::new(
        NSPoint::new(x, top),
        NSSize::new(width - (x - space::MD), metrics.title),
    ));
    view.addSubview(&title);
    top += metrics.title + 2.0;
    let id = text(
        line.command.as_str(),
        TextRole::Caption,
        true,
        &id_tint,
        mtm,
    );
    id.setFrame(NSRect::new(
        NSPoint::new(space::MD, top),
        NSSize::new(width, metrics.detail),
    ));
    view.addSubview(&id);
    if let Some(problem) = problem {
        top += metrics.detail + 2.0;
        let note = text(
            &problem,
            TextRole::Caption,
            false,
            &color::destructive(),
            mtm,
        );
        note.setFrame(NSRect::new(
            NSPoint::new(space::MD, top),
            NSSize::new(width, metrics.detail),
        ));
        note.setToolTip(Some(&NSString::from_str(&problem)));
        view.addSubview(&note);
    }
}

fn warning_symbol(mtm: MainThreadMarker) -> Option<Retained<NSImageView>> {
    symbol(
        "exclamationmark.triangle.fill",
        "Conflict",
        TextRole::Caption,
        &color::destructive(),
        mtm,
    )
}

/// The keycaps, a placeholder when nothing is bound, or the box that records keys.
fn keys(view: &NSView, line: &BindingLine, state: RowState<'_>, mtm: MainThreadMarker) {
    let size = view.frame().size;
    let content: Retained<NSView> = match state {
        RowState::Recording(recorded) => recording_box(recorded, size.width - space::MD, mtm),
        RowState::Idle => {
            if let Some(keys) = &line.keys {
                let style = if line.conflict.is_some() {
                    KeycapStyle::Conflict
                } else {
                    KeycapStyle::Plain
                };
                keycap_group_styled(keys, style, mtm)
            } else {
                let placeholder = match line.source {
                    BindingSource::User => "Removed",
                    BindingSource::Default => "—",
                };
                label(
                    placeholder,
                    TextRole::Caption,
                    &color::tertiary_label(),
                    mtm,
                )
                .into_super()
                .into_super()
            }
        }
    };
    let content_size = content.frame().size;
    content.setFrameOrigin(NSPoint::new(
        space::MD,
        ((size.height - content_size.height) / 2.0).round(),
    ));
    view.addSubview(&content);
}

/// An accent box that waits for keys: "Type a shortcut", or the keys pressed, in red when the
/// keymap refuses them.
fn recording_box(
    recorded: Option<&Recorded>,
    width: CGFloat,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let refused = recorded.is_some_and(|recorded| recorded.problem.is_some());
    let border = if refused {
        color::destructive_line
    } else {
        color::accent_line
    };
    let plate = Plate::new(None, Some(border), radius::KEYCAP, mtm);
    let content: Retained<NSView> = match recorded {
        Some(recorded) if !recorded.keys.is_empty() => {
            let style = if refused {
                KeycapStyle::Conflict
            } else {
                KeycapStyle::Selected
            };
            keycap_group_styled(&recorded.keys, style, mtm)
        }
        _ => label("Type a shortcut", TextRole::Caption, &color::accent(), mtm)
            .into_super()
            .into_super(),
    };
    let content_size = content.frame().size;
    let height = (content_size.height + space::XS * 2.0).ceil();
    plate.setFrameSize(NSSize::new(width - space::SM, height));
    content.setFrameOrigin(NSPoint::new(
        space::SM,
        ((height - content_size.height) / 2.0).round(),
    ));
    plate.addSubview(&content);
    plate.into_super()
}

/// The clause in SF Mono on a chip, red when the row is in conflict; `—` for a binding that
/// holds everywhere.
fn when(view: &NSView, line: &BindingLine, mtm: MainThreadMarker) {
    let size = view.frame().size;
    let Some(clause) = &line.when else {
        let dash = label("—", TextRole::Caption, &color::tertiary_label(), mtm);
        let height = dash.frame().size.height;
        dash.setFrameOrigin(NSPoint::new(
            space::MD,
            ((size.height - height) / 2.0).round(),
        ));
        view.addSubview(&dash);
        return;
    };
    let conflict = line.conflict.is_some();
    let (tint, fill, border): (Retained<NSColor>, Paint, Paint) = if conflict {
        (
            color::destructive(),
            color::destructive_fill,
            color::destructive_line,
        )
    } else {
        (color::label(), color::group_fill, color::separator)
    };
    let field = text(clause, TextRole::Caption, true, &tint, mtm);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    field.setToolTip(Some(&NSString::from_str(clause)));
    let text_size = field.frame().size;
    let room = size.width - space::MD * 2.0;
    let chip_size = NSSize::new(
        (text_size.width + space::SM * 2.0).min(room).ceil(),
        (text_size.height + space::XS).ceil(),
    );
    let chip = Plate::new(Some(fill), Some(border), radius::KEYCAP - 2.0, mtm);
    chip.setFrame(NSRect::new(
        NSPoint::new(space::MD, ((size.height - chip_size.height) / 2.0).round()),
        chip_size,
    ));
    field.setFrame(NSRect::new(
        NSPoint::new(
            space::SM,
            ((chip_size.height - text_size.height) / 2.0).round(),
        ),
        NSSize::new(chip_size.width - space::SM * 2.0, text_size.height),
    ));
    chip.addSubview(&field);
    view.addSubview(&chip);
}

/// What the row says about where its binding comes from: `Default`, `User`, `Conflict`, or
/// `Recording`, and in what color.
fn origin(line: &BindingLine, state: RowState<'_>) -> (&'static str, Paint) {
    match (state, line.source) {
        (RowState::Recording(_), _) => ("Recording", color::accent),
        (RowState::Idle, _) if line.conflict.is_some() => ("Conflict", color::destructive),
        (RowState::Idle, BindingSource::User) => ("User", color::label),
        (RowState::Idle, BindingSource::Default) if line.keys.is_none() => {
            ("", color::tertiary_label)
        }
        (RowState::Idle, BindingSource::Default) => ("Default", color::tertiary_label),
    }
}

/// What `VoiceOver` reads for a row: the command, its keys, when they hold, and their source.
pub fn spoken(line: &BindingLine, state: RowState<'_>) -> String {
    let mut parts = vec![line.title.to_owned()];
    parts.push(line.keys.clone().unwrap_or_else(|| "no keys".to_owned()));
    if let Some(clause) = &line.when {
        parts.push(format!("when {clause}"));
    }
    let (source, _) = origin(line, state);
    if !source.is_empty() {
        parts.push(source.to_owned());
    }
    if let Some(problem) = state.problem(line) {
        parts.push(problem);
    }
    parts.join(", ")
}

/// Where the binding comes from, in SF Mono.
fn source(view: &NSView, line: &BindingLine, state: RowState<'_>, mtm: MainThreadMarker) {
    let (text_value, tint) = origin(line, state);
    let size = view.frame().size;
    let field = text(text_value, TextRole::Caption, true, &tint(), mtm);
    let height = field.frame().size.height;
    field.setFrame(NSRect::new(
        NSPoint::new(space::MD, ((size.height - height) / 2.0).round()),
        NSSize::new(size.width - space::MD * 2.0, height),
    ));
    view.addSubview(&field);
}

fn text(
    value: &str,
    role: TextRole,
    mono: bool,
    tint: &NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = label(value, role, tint, mtm);
    if mono {
        field.setFont(Some(&role.mono_font()));
        field.sizeToFit();
    }
    field
}

#[cfg(test)]
mod tests {
    use quickie_application::{BindingLine, BindingSource};
    use quickie_domain_commands::CommandId;

    use super::{RowState, spoken};

    fn line(keys: Option<&str>, when: Option<&str>, source: BindingSource) -> BindingLine {
        BindingLine {
            command: CommandId::try_from("quickie.tab.reload").unwrap(),
            title: "Reload Page",
            keys: keys.map(str::to_owned),
            when: when.map(str::to_owned),
            source,
            conflict: None,
        }
    }

    #[test]
    fn a_row_reads_its_command_keys_clause_and_source() {
        assert_eq!(
            spoken(
                &line(Some("⌘R"), Some("pageFocused"), BindingSource::Default),
                RowState::Idle
            ),
            "Reload Page, ⌘R, when pageFocused, Default"
        );
        assert_eq!(
            spoken(&line(None, None, BindingSource::User), RowState::Idle),
            "Reload Page, no keys, User"
        );
        assert_eq!(
            spoken(&line(None, None, BindingSource::Default), RowState::Idle),
            "Reload Page, no keys"
        );
    }

    #[test]
    fn a_conflict_says_what_the_keys_already_run() {
        let mut taken = line(Some("⌘T"), None, BindingSource::User);
        taken.conflict = Some("Open New Tab".to_owned());

        assert_eq!(
            spoken(&taken, RowState::Idle),
            "Reload Page, ⌘T, Conflict, Already runs Open New Tab"
        );
        assert_eq!(
            spoken(&taken, RowState::Recording(None)),
            "Reload Page, ⌘T, Recording"
        );
    }
}

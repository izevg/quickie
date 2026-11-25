use std::cell::{Cell, RefCell};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::sel;
use objc2_app_kit::{
    NSAccessibility, NSBezelStyle, NSButton, NSControlSize, NSLineBreakMode, NSTextAlignment,
    NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::NSString;
use quickie_ui_shared::format::answer::markdown::{Span, Style, document};
use quickie_ui_shared::format::answer::text::AnswerText;
use quickie_ui_shared::format::answer::transcript::Edit;
use quickie_ui_shared::tokens::{TextRole, color, space};

use super::WorkspaceToolStatus;
use super::model::Row;
use super::native::{Target, WorkspaceView, button, label, place};

const TOOL_HEIGHT: CGFloat = 28.0;
const CAPTION_HEIGHT: CGFloat = 18.0;
const CHIP_HEIGHT: CGFloat = 24.0;

pub(super) struct ConversationRow {
    view: Retained<WorkspaceView>,
    text: AnswerText,
    tool: Option<ToolRow>,
    sources: Vec<Retained<NSButton>>,
    sources_caption: Option<Retained<NSTextField>>,
    expanded: Cell<bool>,
    measured: Cell<Option<(CGFloat, CGFloat)>>,
    row: RefCell<Row>,
}

impl ConversationRow {
    pub fn new(row: &Row, index: usize, target: &Retained<Target>, mtm: MainThreadMarker) -> Self {
        let view = WorkspaceView::new(mtm);
        let text = AnswerText::embedded(mtm);
        let links = target.clone();
        text.on_link(move |url| links.open(url));
        let tool = matches!(row, Row::Tool { .. }).then(|| ToolRow::new(index, target, mtm));
        view.addSubview(text.view());
        if let Some(tool) = &tool {
            view.addSubview(&tool.disclosure);
            view.addSubview(&tool.target);
            view.addSubview(&tool.state);
        }
        let sources: Vec<_> = if let Row::Answer { sources, .. } = row {
            sources
                .iter()
                .map(|source| {
                    let chip = button(&chip_title(&source.quote), sel!(cite:), target, mtm);
                    chip.setBezelStyle(NSBezelStyle::AccessoryBarAction);
                    chip.setControlSize(NSControlSize::Small);
                    chip.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                    chip.setContentTintColor(Some(&color::label()));
                    chip.setTag(isize::try_from(source.id).unwrap_or(isize::MAX));
                    let title = format!("“{}”", source.quote);
                    chip.setAccessibilityLabel(Some(&NSString::from_str(&title)));
                    chip.setToolTip(Some(&NSString::from_str(&source.quote)));
                    chip.sizeToFit();
                    view.addSubview(&chip);
                    chip
                })
                .collect()
        } else {
            Vec::new()
        };
        let sources_caption = (!sources.is_empty()).then(|| {
            let caption = label(TextRole::Caption, &color::secondary_label(), mtm);
            caption.setStringValue(&NSString::from_str("Sources"));
            view.addSubview(&caption);
            caption
        });
        let shown = Self {
            view,
            text,
            tool,
            sources,
            sources_caption,
            expanded: Cell::new(false),
            measured: Cell::new(None),
            row: RefCell::new(row.clone()),
        };
        shown.update(row);
        shown
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    pub fn update(&self, row: &Row) {
        self.measured.set(None);
        self.row.replace(row.clone());
        self.text.clear();
        let spans = match row {
            Row::Prompt(prompt) => vec![Span::new(prompt, Style::Strong)],
            Row::Answer { text: answer, .. } => document(answer),
            Row::Notice { text, failed } => vec![Span::new(
                text,
                if *failed { Style::Problem } else { Style::Note },
            )],
            Row::Tool {
                title,
                target,
                status,
                detail,
            } => {
                let expanded = self.expanded.get();
                if let Some(tool) = &self.tool {
                    tool.update(title, target, *status, expanded);
                }
                let mut spans = Vec::new();
                if expanded && !target.is_empty() {
                    spans.push(Span::new(target, Style::Note));
                }
                if expanded && !detail.is_empty() {
                    let separator = if spans.is_empty() { "" } else { "\n" };
                    spans.push(Span::new(
                        format!("{separator}{detail}"),
                        if *status == Some(WorkspaceToolStatus::Failed) {
                            Style::Problem
                        } else {
                            Style::Note
                        },
                    ));
                }
                spans
            }
        };
        self.text.view().setHidden(spans.is_empty());
        self.text.apply(&Edit { from: 0, spans });
    }

    pub fn toggle_detail(&self) {
        self.expanded.set(!self.expanded.get());
        let row = self.row.borrow().clone();
        self.update(&row);
    }

    pub fn expire_citation(&self, id: u32) -> Option<&NSButton> {
        let chip = self
            .sources
            .iter()
            .find(|chip| chip.isEnabled() && u32::try_from(chip.tag()).ok() == Some(id))?;
        chip.setTitle(&NSString::from_str(&format!(
            "{} · Unavailable",
            chip.title()
        )));
        let quote = chip.toolTip().unwrap_or_else(|| chip.title());
        chip.setAccessibilityLabel(Some(&NSString::from_str(&format!(
            "Source unavailable: {quote}"
        ))));
        chip.setEnabled(false);
        self.measured.set(None);
        Some(chip)
    }

    pub fn spacing_after(&self) -> CGFloat {
        if self.tool.is_some() {
            space::XS
        } else {
            space::LG
        }
    }

    pub fn layout(&self, width: CGFloat) -> CGFloat {
        if let Some((measured_width, height)) = self.measured.get()
            && (width - measured_width).abs() < CGFloat::EPSILON
        {
            return height;
        }
        let mut height = self.tool.as_ref().map_or(0.0, |tool| tool.layout(width));
        if !self.text.view().isHidden() {
            let text_height = self.text.content_height(width).max(1.0) + space::XS;
            place(self.text.view(), 0.0, height, width, text_height);
            height += text_height;
        }
        if let Some(caption) = &self.sources_caption {
            height += space::SM;
            place(caption, 0.0, height, width, CAPTION_HEIGHT);
            height += CAPTION_HEIGHT + space::XS;
        }
        let mut x = 0.0;
        for chip in &self.sources {
            let chip_width = chip.fittingSize().width.min(width);
            if x > 0.0 && x + chip_width > width {
                height += CHIP_HEIGHT + space::XS;
                x = 0.0;
            }
            place(chip, x, height, chip_width, CHIP_HEIGHT);
            x += chip_width + space::XS;
        }
        if x > 0.0 {
            height += CHIP_HEIGHT;
        }
        self.measured.set(Some((width, height)));
        height
    }
}

struct ToolRow {
    disclosure: Retained<NSButton>,
    target: Retained<NSTextField>,
    state: Retained<NSTextField>,
}

impl ToolRow {
    fn new(index: usize, owner: &Retained<Target>, mtm: MainThreadMarker) -> Self {
        let disclosure = button("", sel!(details:), owner, mtm);
        disclosure.setBordered(false);
        disclosure.setAlignment(NSTextAlignment::Left);
        disclosure.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        disclosure.setTag(isize::try_from(index).unwrap_or(isize::MAX));
        let target = label(TextRole::Caption, &color::secondary_label(), mtm);
        target.setAccessibilityElement(false);
        let state = label(TextRole::Caption, &color::secondary_label(), mtm);
        state.setAlignment(NSTextAlignment::Right);
        state.setAccessibilityElement(false);
        Self {
            disclosure,
            target,
            state,
        }
    }

    fn update(
        &self,
        title: &str,
        target: &str,
        status: Option<WorkspaceToolStatus>,
        expanded: bool,
    ) {
        let (marker, state) = match status {
            None => ("◷", "Running"),
            Some(WorkspaceToolStatus::Completed) => ("✓", "Completed"),
            Some(WorkspaceToolStatus::Failed) => ("!", "Failed"),
            Some(WorkspaceToolStatus::Cancelled) => ("–", "Cancelled"),
        };
        let arrow = if expanded { "▾" } else { "▸" };
        self.disclosure
            .setTitle(&NSString::from_str(&format!("{arrow} {title}")));
        self.disclosure
            .setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{title}, {state}, {} details",
                if expanded { "hide" } else { "show" }
            ))));
        let target_value = NSString::from_str(target);
        self.disclosure.setAccessibilityHelp(Some(&target_value));
        self.disclosure.setAccessibilityExpanded(expanded);
        self.target.setStringValue(&target_value);
        self.target.setToolTip(Some(&target_value));
        self.target.setHidden(target.is_empty() || expanded);
        self.state
            .setStringValue(&NSString::from_str(&format!("{marker} {state}")));
    }

    fn layout(&self, width: CGFloat) -> CGFloat {
        let state_width = self.state.fittingSize().width.min(width);
        let title_width = self
            .disclosure
            .fittingSize()
            .width
            .min((width - state_width - space::SM).max(0.0));
        place(&self.disclosure, 0.0, 0.0, title_width, TOOL_HEIGHT);
        place_tool_label(&self.state, width - state_width, state_width);
        let x = title_width + space::SM;
        place_tool_label(
            &self.target,
            x,
            (width - state_width - space::SM - x).max(0.0),
        );
        TOOL_HEIGHT
    }
}

fn place_tool_label(field: &NSTextField, x: CGFloat, width: CGFloat) {
    // NSTextField draws at the top of an oversized frame. Center its fitted frame
    // alongside the button, which centers its own title.
    let height = field.fittingSize().height.min(TOOL_HEIGHT);
    let y = ((TOOL_HEIGHT - height) / 2.0).round();
    place(field, x, y, width, height);
}

fn chip_title(quote: &str) -> String {
    quote.char_indices().nth(36).map_or_else(
        || format!("“{quote}”"),
        |(end, _)| format!("“{}…”", quote[..end].trim_end()),
    )
}

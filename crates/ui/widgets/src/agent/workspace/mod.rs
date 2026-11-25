//! The persistent conversation beside the Agent workspace's page.

mod model;
mod native;
mod row;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, Message, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey,
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityPostNotificationWithUserInfo,
    NSAutoresizingMaskOptions, NSBox, NSBoxType, NSButton, NSColor, NSFocusRingType, NSResponder,
    NSScrollView, NSTextField, NSTextFieldBezelStyle, NSView, NSWindowOrderingMode,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, QueryField};
use url::Url;

use model::{Conversation, Row};
use native::{Target, WorkspaceView, button, label, place};
use row::ConversationRow;

const MIN_WIDTH: CGFloat = 320.0;
const MAX_WIDTH: CGFloat = 480.0;
const MIN_HEIGHT: CGFloat = 240.0;

fn card_width(window_width: CGFloat) -> CGFloat {
    (window_width * overlay::ANSWER_PANEL_SHARE)
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min((window_width - space::XL - space::LG).max(0.0))
        .round()
}

#[cfg(test)]
mod layout_tests {
    use super::card_width;

    #[test]
    fn the_answer_card_uses_a_bounded_share_of_the_workspace() {
        for (window, expected) in [
            (800.0, 320.0),
            (840.0, 336.0),
            (1080.0, 432.0),
            (1600.0, 480.0),
        ] {
            assert!((card_width(window) - expected).abs() < f64::EPSILON);
        }
    }
}

const STATUS_HEIGHT: CGFloat = 20.0;
const CONTROL_HEIGHT: CGFloat = 28.0;
const COMPOSER_HEIGHT: CGFloat = CONTROL_HEIGHT + space::LG * 2.0;
const HEADER_HEIGHT: CGFloat = CONTROL_HEIGHT + space::LG + space::XS;
const SETUP_HEIGHT: CGFloat = STATUS_HEIGHT + CONTROL_HEIGHT + space::SM;

type Handler = Rc<dyn Fn(WorkspacePanelRequest)>;

/// What the user asks of the Agent workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspacePanelRequest {
    Submit(String),
    Stop,
    Close,
    TogglePreview,
    Settings,
    /// Open an HTTP(S) link explicitly clicked in an answer.
    Open(Url),
    /// Find a caller-verified passage after an explicit citation click.
    Cite(u32),
}

/// A passage verified by the caller against its source page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceCitation {
    pub id: u32,
    pub quote: String,
}

/// A receipt's terminal state, supplied by the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceToolStatus {
    Completed,
    Failed,
    Cancelled,
}

/// The floating answer card and the optional isolated page preview.
pub struct AgentWorkspacePanel {
    view: Retained<WorkspaceView>,
    page: RefCell<Option<Retained<NSView>>>,
    surface: GlassSurface,
    card: Retained<WorkspaceView>,
    height: Cell<CGFloat>,
    glyph: Retained<NSTextField>,
    source: Retained<NSTextField>,
    activity: Retained<NSTextField>,
    stop: Retained<NSButton>,
    close: Retained<NSButton>,
    preview: Retained<NSButton>,
    scroll: Retained<NSScrollView>,
    document: Retained<WorkspaceView>,
    composer: QueryField,
    divider: Retained<NSBox>,
    send: Retained<NSButton>,
    settings: Retained<NSButton>,
    setup: Retained<NSTextField>,
    target: Retained<Target>,
    conversation: RefCell<Conversation>,
    rows: RefCell<Vec<ConversationRow>>,
    handler: RefCell<Option<Handler>>,
    mtm: MainThreadMarker,
}

impl AgentWorkspacePanel {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        let target = Target::new(mtm);
        let view = WorkspaceView::new(mtm);
        let card = WorkspaceView::new(mtm);
        let document = WorkspaceView::new(mtm);
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let divider = NSBox::new(mtm);
        divider.setBoxType(NSBoxType::Separator);
        let glyph = label(TextRole::Title, &color::agent(), mtm);
        glyph.setFont(Some(&TextRole::Title.mono_font()));
        glyph.setStringValue(&NSString::from_str("?"));
        glyph.setAccessibilityElement(false);
        let setup = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        setup.setStringValue(&NSString::from_str(
            "Set up an agent endpoint to send a message.",
        ));
        let scroll = NSScrollView::new(mtm);
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDocumentView(Some(&document));
        let composer = QueryField::new(&TextRole::RowTitle.font(), "Message the agent…", mtm);
        composer.view().setBezeled(true);
        composer
            .view()
            .setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        composer.view().setDrawsBackground(true);
        composer
            .view()
            .setBackgroundColor(Some(&NSColor::textBackgroundColor()));
        composer.view().setFocusRingType(NSFocusRingType::Exterior);
        composer
            .view()
            .setAccessibilityLabel(Some(&NSString::from_str("Message the agent")));
        // SAFETY: the panel owns the target for the field's lifetime.
        unsafe { composer.view().setTarget(Some(&target)) };
        // SAFETY: the target implements `submit:` with an optional sender.
        unsafe { composer.view().setAction(Some(sel!(submit:))) };
        let stop = button("Stop", sel!(stop:), &target, mtm);
        stop.setContentTintColor(Some(&color::destructive()));
        stop.setAccessibilityLabel(Some(&NSString::from_str("Stop agent")));
        let close = button("×", sel!(closeAgent:), &target, mtm);
        let close_label = NSString::from_str("Close Agent");
        close.setAccessibilityLabel(Some(&close_label));
        close.setToolTip(Some(&close_label));
        let preview = button("Agent page", sel!(togglePreview:), &target, mtm);
        preview.setHidden(true);
        let send = button("Send", sel!(submit:), &target, mtm);
        send.setContentTintColor(Some(&color::accent()));
        let panel = Rc::new(Self {
            view,
            page: RefCell::default(),
            surface,
            card,
            height: Cell::new(MIN_HEIGHT),
            glyph,
            document,
            scroll,
            composer,
            divider,
            stop,
            close,
            preview,
            send,
            source: label(TextRole::RowTitle, &color::label(), mtm),
            activity: label(TextRole::Caption, &color::secondary_label(), mtm),
            settings: button("Agent Settings…", sel!(settings:), &target, mtm),
            setup,
            target,
            conversation: RefCell::new(Conversation::default()),
            rows: RefCell::new(Vec::new()),
            handler: RefCell::new(None),
            mtm,
        });
        panel.view.set_owner(Rc::downgrade(&panel));
        panel.target.set_owner(Rc::downgrade(&panel));
        panel.view.addSubview(panel.surface.view());
        panel.surface.content_view().addSubview(&panel.card);
        for child in [
            &*panel.glyph as &NSView,
            &panel.source,
            &panel.activity,
            &panel.stop,
            &panel.close,
            &panel.preview,
            &panel.scroll,
            &panel.divider,
            &panel.setup,
            &panel.settings,
            panel.composer.view(),
            &panel.send,
        ] {
            panel.card.addSubview(child);
        }
        let weak = Rc::downgrade(&panel);
        panel.composer.on_change(move || {
            if let Some(panel) = weak.upgrade() {
                panel.refresh_send();
            }
        });
        panel.set_source("Current tab");
        panel.set_activity("", false);
        panel
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Places the card over an existing browser page and follows the host's size.
    pub fn show_in(&self, host: &NSView) {
        self.view.setFrame(host.bounds());
        self.view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        host.addSubview(&self.view);
    }

    /// The answer card, inside the workspace view.
    #[must_use]
    pub fn card_view(&self) -> &NSView {
        &self.card
    }

    /// Installs the caller-owned page view in the live viewport beside the answer card.
    pub fn attach_page(&self, page: &NSView) {
        if let Some(previous) = self.page.replace(Some(page.retain())) {
            previous.removeFromSuperview();
        }
        self.view.addSubview_positioned_relativeTo(
            page,
            NSWindowOrderingMode::Below,
            Some(self.surface.view()),
        );
        self.resize();
    }

    /// Gives the composer the keyboard after an explicit workspace open.
    pub fn focus_composer(&self) {
        if let Some(window) = self.view.window() {
            window.makeFirstResponder(Some(self.composer.view()));
        }
    }

    /// Whether the card, its editor, or the preview owns the window's keyboard.
    #[must_use]
    pub fn has_focus(&self) -> bool {
        let Some(responder) = self
            .view
            .window()
            .and_then(|window| window.firstResponder())
        else {
            return false;
        };
        if self.composer.view().currentEditor().is_some_and(|editor| {
            let editor: &NSResponder = &editor;
            Retained::as_ptr(&responder) == std::ptr::from_ref(editor)
        }) {
            return true;
        }
        responder
            .downcast::<NSView>()
            .is_ok_and(|view| view.isDescendantOf(&self.view))
    }

    /// Offers the alternate page once the isolated preview has a document.
    pub fn set_preview(&self, available: bool, shown: bool) {
        let (title, label) = if shown {
            ("Source page", "Show source page")
        } else {
            ("Agent page", "Show isolated Agent page")
        };
        self.preview.setTitle(&NSString::from_str(title));
        let label = NSString::from_str(label);
        self.preview.setAccessibilityLabel(Some(&label));
        self.preview.setToolTip(Some(&label));
        self.preview.setHidden(!available);
        self.refresh_controls();
    }

    pub fn on_request(&self, handler: impl Fn(WorkspacePanelRequest) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_source(&self, source: &str) {
        let label = if source == "Source page unavailable" {
            source.to_owned()
        } else {
            format!("Source · {source}")
        };
        let label = NSString::from_str(&label);
        self.source.setStringValue(&label);
        self.source.setToolTip(Some(&label));
    }

    pub fn set_activity(&self, message: &str, running: bool) {
        let running_changed = {
            let mut conversation = self.conversation.borrow_mut();
            std::mem::replace(&mut conversation.running, running) != running
        };
        let hidden = message.is_empty();
        let visibility_changed = self.activity.isHidden() != hidden;
        let text = NSString::from_str(message);
        self.activity.setStringValue(&text);
        self.activity.setAccessibilityLabel(Some(&text));
        self.activity.setHidden(hidden);
        if running_changed || visibility_changed {
            self.refresh_controls();
        }
        if running_changed && running {
            self.announce(message);
        }
    }

    /// Retains a terminal turn message in chronological order with the answers.
    pub fn append_notice(&self, message: &str, failed: bool) {
        self.append(Row::Notice {
            text: message.to_owned(),
            failed,
        });
        self.announce(message);
    }

    pub fn append_prompt(&self, text: &str) {
        self.append(Row::Prompt(text.to_owned()));
    }

    /// Appends a complete answer, using the existing safe Markdown renderer.
    pub fn append_answer(&self, text: &str) {
        self.append_answer_with_sources(text, &[]);
    }

    /// Appends an answer and explicit, caller-verified source controls.
    /// Citation-like text in the answer never creates a source control.
    pub fn append_answer_with_sources(&self, answer: &str, sources: &[WorkspaceCitation]) {
        self.append(Row::Answer {
            text: answer.to_owned(),
            sources: sources.to_vec(),
        });
        self.announce("Answer ready");
    }

    /// Marks a citation in place when its document can no longer offer the passage.
    pub fn expire_citation(&self, id: u32) {
        let rows = self.rows.borrow();
        let Some(button) = rows.iter().find_map(|row| row.expire_citation(id)) else {
            return;
        };
        let citation: &NSResponder = button;
        if self
            .view
            .window()
            .and_then(|window| window.firstResponder())
            .is_some_and(|responder| Retained::as_ptr(&responder) == std::ptr::from_ref(citation))
        {
            self.focus_composer();
        }
        drop(rows);
        self.layout(false, false);
        self.announce("This source passage is no longer available.");
    }

    /// Inserts an actual invocation; its returned index identifies the same row until close.
    pub fn start_tool(&self, title: &str, target: &str) -> usize {
        let following = self.following();
        let mut conversation = self.conversation.borrow_mut();
        let index = conversation.start_tool(title, target);
        let row = conversation.rows[index].clone();
        drop(conversation);
        self.add_row(&row, index);
        self.layout(following, true);
        index
    }

    pub fn finish_tool(&self, row: usize, status: WorkspaceToolStatus, detail: &str) {
        let following = self.following();
        let mut conversation = self.conversation.borrow_mut();
        conversation.finish_tool(row, status, detail);
        if let Some(Row::Tool { .. }) = conversation.rows.get(row)
            && let Some(shown) = self.rows.borrow().get(row)
        {
            shown.update(&conversation.rows[row]);
        }
        drop(conversation);
        self.layout(following, true);
    }

    pub fn set_unconfigured(&self, unconfigured: bool) {
        self.conversation.borrow_mut().unconfigured = unconfigured;
        self.refresh_controls();
    }

    fn append(&self, row: Row) {
        let following = self.following();
        let index = self.conversation.borrow().rows.len();
        self.add_row(&row, index);
        self.conversation.borrow_mut().rows.push(row);
        self.layout(following, true);
    }

    fn add_row(&self, row: &Row, index: usize) {
        let shown = ConversationRow::new(row, index, &self.target, self.mtm);
        self.document.addSubview(shown.view());
        self.rows.borrow_mut().push(shown);
        if let Some(window) = self.view.window() {
            window.recalculateKeyViewLoop();
        }
    }

    fn request(&self, request: WorkspacePanelRequest) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }

    fn announce(&self, message: &str) {
        let Some(window) = self.view.window().filter(|window| window.isKeyWindow()) else {
            return;
        };
        let text = NSString::from_str(message);
        let value: &AnyObject = &text;
        // SAFETY: AppKit exports both constants. The window is live on the main thread,
        // and the user info holds the announcement string under its documented key.
        unsafe {
            let info = NSDictionary::from_slices(&[NSAccessibilityAnnouncementKey], &[value]);
            NSAccessibilityPostNotificationWithUserInfo(
                &window,
                NSAccessibilityAnnouncementRequestedNotification,
                Some(&info),
            );
        }
    }

    fn submit(&self) {
        let prompt = self.conversation.borrow().submit(&self.composer.text());
        if let Some(prompt) = prompt {
            self.composer.set_text("");
            self.refresh_send();
            self.request(WorkspacePanelRequest::Submit(prompt));
        }
    }

    fn refresh_controls(&self) {
        let conversation = self.conversation.borrow();
        let stop: &NSResponder = &self.stop;
        let stop_was_focused = !conversation.running
            && self
                .view
                .window()
                .and_then(|window| window.firstResponder())
                .is_some_and(|responder| Retained::as_ptr(&responder) == std::ptr::from_ref(stop));
        self.stop.setHidden(!conversation.running);
        self.stop.setEnabled(conversation.running);
        self.settings.setHidden(!conversation.unconfigured);
        self.setup.setHidden(!conversation.unconfigured);
        drop(conversation);
        self.refresh_send();
        if stop_was_focused {
            self.focus_composer();
        }
        if let Some(window) = self.view.window() {
            window.recalculateKeyViewLoop();
        }
        self.layout(self.following(), false);
    }

    fn refresh_send(&self) {
        self.send.setEnabled(
            self.conversation
                .borrow()
                .submit(&self.composer.text())
                .is_some(),
        );
    }

    fn toggle_detail(&self, index: usize) {
        if let Some(row) = self.rows.borrow().get(index) {
            row.toggle_detail();
        }
        self.layout(false, true);
    }

    fn cite(&self, id: u32) {
        let request = self.conversation.borrow().cite(id);
        if let Some(request) = request {
            self.request(request);
        }
    }

    fn following(&self) -> bool {
        let visible = self.scroll.contentView().documentVisibleRect();
        visible.origin.y + visible.size.height >= self.document.frame().size.height - space::XS
    }

    fn resize(&self) {
        self.layout(self.following(), true);
    }

    fn layout(&self, following: bool, grow: bool) {
        let bounds = self.view.bounds().size;
        let card_width = card_width(bounds.width);
        let card_x = (bounds.width - space::XL - card_width).max(0.0);
        if let Some(page) = self.page.borrow().as_ref() {
            place(page, 0.0, 0.0, (card_x - space::LG).max(0.0), bounds.height);
        }
        let conversation = self.conversation.borrow();
        let running = conversation.running;
        let setup = if conversation.unconfigured {
            SETUP_HEIGHT
        } else {
            0.0
        };
        drop(conversation);
        let header = self.layout_header(card_width, running);

        let origin = self.scroll.contentView().documentVisibleRect().origin.y;
        self.scroll
            .setFrameSize(NSSize::new(card_width, self.scroll.frame().size.height));
        let document_width = self.scroll.contentSize().width;
        let row_width = (document_width - space::LG * 2.0).max(1.0);
        let mut restored_origin = None;
        let mut y = space::XS;
        for row in self.rows.borrow().iter() {
            let previous = row.view().frame();
            let height = row.layout(row_width);
            let spacing = row.spacing_after();
            if !following
                && restored_origin.is_none()
                && previous.size.height > 0.0
                && origin < previous.origin.y + previous.size.height + spacing
            {
                let offset = (origin - previous.origin.y).min(height + spacing);
                restored_origin = Some(y + offset);
            }
            place(row.view(), space::LG, y, row_width, height);
            y += height + spacing;
        }

        let maximum_height =
            (bounds.height - space::LG * 2.0).clamp(0.0, overlay::ANSWER_PANEL_MAX_HEIGHT);
        let desired_height = if grow {
            header + y + COMPOSER_HEIGHT + setup
        } else {
            MIN_HEIGHT
        };
        let card_height = self
            .height
            .get()
            .max(desired_height.ceil())
            .min(maximum_height);
        self.height.set(card_height.max(MIN_HEIGHT));
        let frame = NSRect::new(
            NSPoint::new(card_x, space::LG),
            NSSize::new(card_width, card_height),
        );
        if self.surface.view().frame() != frame {
            self.surface.set_frame(frame);
            self.card.setFrameSize(frame.size);
        }

        let bottom = (card_height - COMPOSER_HEIGHT - setup).max(header);
        place(
            &self.scroll,
            0.0,
            header,
            card_width,
            (bottom - header).max(0.0),
        );
        self.layout_composer(card_width, bottom, setup);

        let visible = self.scroll.contentSize();
        let height = y.max(visible.height);
        self.document
            .setFrameSize(NSSize::new(visible.width, height));
        let end = (height - visible.height).max(0.0);
        let origin = if following {
            end
        } else {
            restored_origin.unwrap_or(origin).clamp(0.0, end)
        };
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, origin));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
    }

    fn layout_header(&self, card_width: CGFloat, running: bool) -> CGFloat {
        let padding = space::LG;
        let width = (card_width - padding * 2.0).max(1.0);
        let close_x = card_width - padding - CONTROL_HEIGHT;
        place(
            &self.close,
            close_x,
            padding,
            CONTROL_HEIGHT,
            CONTROL_HEIGHT,
        );
        let right = close_x - space::XS;
        let preview_width = self.preview.fittingSize().width;
        let preview_space = if self.preview.isHidden() {
            0.0
        } else {
            preview_width + space::XS
        };
        place(
            &self.preview,
            right - preview_width,
            padding,
            preview_width,
            CONTROL_HEIGHT,
        );
        let stop_right = right - preview_space;
        let stop_width = self.stop.fittingSize().width.max(52.0);
        let stop_space = if running { stop_width + space::SM } else { 0.0 };
        let source_x = padding * 2.0 + space::SM;
        place(&self.glyph, padding, padding, space::LG, CONTROL_HEIGHT);
        place(
            &self.source,
            source_x,
            padding,
            (stop_right - source_x - stop_space).max(0.0),
            CONTROL_HEIGHT,
        );
        place(
            &self.activity,
            padding,
            HEADER_HEIGHT - space::XS,
            width,
            STATUS_HEIGHT,
        );
        place(
            &self.stop,
            stop_right - stop_width,
            padding,
            stop_width,
            CONTROL_HEIGHT,
        );
        HEADER_HEIGHT
            + if self.activity.isHidden() {
                0.0
            } else {
                STATUS_HEIGHT
            }
    }

    fn layout_composer(&self, card_width: CGFloat, bottom: CGFloat, setup: CGFloat) {
        let padding = space::LG;
        let width = (card_width - padding * 2.0).max(1.0);
        place(&self.divider, padding, bottom, width, 1.0);
        let send_width = self.send.fittingSize().width.max(52.0);
        let composer_y = bottom + setup + space::LG;
        place(
            self.composer.view(),
            padding,
            composer_y,
            (width - send_width - space::SM).max(1.0),
            CONTROL_HEIGHT,
        );
        place(
            &self.send,
            card_width - padding - send_width,
            composer_y,
            send_width,
            CONTROL_HEIGHT,
        );
        place(
            &self.setup,
            padding,
            bottom + space::SM,
            width,
            STATUS_HEIGHT,
        );
        place(
            &self.settings,
            padding,
            bottom + space::SM + STATUS_HEIGHT,
            self.settings.fittingSize().width.min(width),
            CONTROL_HEIGHT,
        );
    }
}

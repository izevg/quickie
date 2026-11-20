//! The agent's answer panel (`docs/design/mocks/agent/ask/`, `ask-empty/`): an E2 surface docked
//! at the right of the window, so the page stays readable beside it and a cited passage scrolls
//! into view next to the answer.
//!
//! - The header names what was asked and the model chip where it runs.
//! - The status line says what the agent does; `Stop ⌘.` while it answers, the measured speed
//!   (`24 tok/s · 0.4 s TTFT`) once it is done.
//! - The answer streams in as plain styled text ([`markdown`]); only its last line is laid out
//!   again as tokens come ([`transcript`]). Nothing in it is fetched: no image, no link.
//! - "Sources from this page": a chip per citation; choosing one finds the quote in the page.
//! - Without an endpoint, the empty state offers the settings and looking for a local LLM.
//!
//! The panel only shows; the browser runs every request.

pub mod markdown;
pub mod text;
pub mod transcript;
pub mod wording;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSBox, NSBoxType, NSButton, NSColor, NSControlSize, NSLineBreakMode,
    NSProgressIndicator, NSProgressIndicatorStyle, NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_application::AnswerEvent;
use quickie_domain_agents::Citation;
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, hint_bar, motion};
use url::Url;

use markdown::{LinePart, Span, Style};
use text::AnswerText;
use transcript::{Edit, Transcript};
pub use wording::Activity;

/// The most citation chips shown; the quotes stay highlighted in the answer either way.
const MAX_CHIPS: usize = 12;
/// The most characters of a quote on its chip.
const CHIP_CHARS: usize = 36;
/// A chip's height.
const CHIP_HEIGHT: CGFloat = 24.0;

/// What the panel asks of the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelRequest {
    /// Find this text in the page the answer is about, and scroll to it.
    Cite(String),
    /// Open this web link in a new tab: the user clicked it.
    Open(Url),
    /// Stop the answer.
    Stop,
    /// Show the agent settings.
    Settings,
    /// Look for a local LLM server.
    DetectLocal,
}

/// What an answer is about, for the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnswerHeader {
    /// The question, or the skill's name.
    pub title: String,
    /// The model chip: the model, and where it runs.
    pub model: String,
}

/// Where the answer stands.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// No endpoint is set up.
    Unconfigured,
    Working(Activity),
    Done(String),
    Stopped,
    Failed,
}

type Handler = Rc<dyn Fn(PanelRequest)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
    anchors: RefCell<Vec<String>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `PanelTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAnswerPanelTarget"]
    #[ivars = Ivars]
    pub struct PanelTarget;

    unsafe impl NSObjectProtocol for PanelTarget {}

    impl PanelTarget {
        #[unsafe(method(cite:))]
        fn cite(&self, sender: Option<&NSButton>) {
            let index = sender.and_then(|button| usize::try_from(button.tag()).ok());
            let anchor = index.and_then(|index| self.ivars().anchors.borrow().get(index).cloned());
            if let Some(anchor) = anchor {
                self.ask(PanelRequest::Cite(anchor));
            }
        }

        #[unsafe(method(stop:))]
        fn stop(&self, _sender: Option<&NSButton>) {
            self.ask(PanelRequest::Stop);
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: Option<&NSButton>) {
            self.ask(PanelRequest::Settings);
        }

        #[unsafe(method(detect:))]
        fn detect(&self, _sender: Option<&NSButton>) {
            self.ask(PanelRequest::DetectLocal);
        }
    }
);

impl PanelTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
            anchors: RefCell::new(Vec::new()),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn ask(&self, request: PanelRequest) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }
}

/// The empty state's views.
struct Empty {
    title: Retained<NSTextField>,
    body: Retained<NSTextField>,
    settings: Retained<NSButton>,
    detect: Retained<NSButton>,
}

pub struct AgentAnswerPanel {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    glyph: Retained<NSTextField>,
    title: Retained<NSTextField>,
    chip: Retained<NSTextField>,
    spinner: Retained<NSProgressIndicator>,
    status: Retained<NSTextField>,
    stop: Retained<NSButton>,
    readout: Retained<NSTextField>,
    separator: Retained<NSBox>,
    text: AnswerText,
    sources: Retained<NSTextField>,
    chips: RefCell<Vec<Retained<NSButton>>>,
    empty: Empty,
    hints: RefCell<Retained<NSView>>,
    transcript: RefCell<Transcript>,
    phase: RefCell<Phase>,
    target: Retained<PanelTarget>,
    open: Cell<bool>,
    mtm: MainThreadMarker,
}

impl AgentAnswerPanel {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let target = PanelTarget::new(mtm);
        let target_object: &AnyObject = target.as_ref();
        let glyph = label(TextRole::Title, &color::agent(), mtm);
        glyph.setFont(Some(&TextRole::Title.mono_font()));
        set(&glyph, "?");
        let spinner = NSProgressIndicator::new(mtm);
        spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        spinner.setControlSize(NSControlSize::Small);
        spinner.setDisplayedWhenStopped(false);
        let stop = button("Stop ⌘.", sel!(stop:), target_object, mtm);
        let readout = label(TextRole::Caption, &color::secondary_label(), mtm);
        readout.setFont(Some(&TextRole::Caption.mono_font()));
        readout.setAlignment(NSTextAlignment::Right);
        let chip = label(TextRole::Caption, &color::secondary_label(), mtm);
        chip.setFont(Some(&TextRole::Caption.mono_font()));
        chip.setAlignment(NSTextAlignment::Right);
        let sources = label(TextRole::Caption, &color::secondary_label(), mtm);
        set(&sources, "Sources from this page");
        let body = NSTextField::wrappingLabelWithString(
            &NSString::from_str(
                "The agent answers through an OpenAI-compatible endpoint you choose: a local \
                 server such as Ollama or LM Studio, or a hosted API. Quickie sends the page \
                 only when you ask, and only to that endpoint.",
            ),
            mtm,
        );
        body.setFont(Some(&TextRole::RowTitle.font()));
        body.setTextColor(Some(&color::secondary_label()));
        body.setSelectable(false);
        let empty_title = label(TextRole::Title, &color::label(), mtm);
        set(&empty_title, "No LLM endpoint set up");
        let empty = Empty {
            title: empty_title,
            body,
            settings: button("Agent Settings…", sel!(settings:), target_object, mtm),
            detect: button("Detect Local LLM", sel!(detect:), target_object, mtm),
        };
        let separator = NSBox::new(mtm);
        separator.setBoxType(NSBoxType::Separator);
        let panel = Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface: GlassSurface::new(Elevation::E2, mtm),
            glyph,
            title: label(TextRole::RowTitle, &color::label(), mtm),
            chip,
            spinner,
            status: label(TextRole::RowDetail, &color::secondary_label(), mtm),
            stop,
            readout,
            separator,
            text: AnswerText::new(mtm),
            sources,
            chips: RefCell::new(Vec::new()),
            empty,
            hints: RefCell::new(hint_bar(&[("⎋", "close")], mtm)),
            transcript: RefCell::new(Transcript::default()),
            phase: RefCell::new(Phase::Unconfigured),
            target,
            open: Cell::new(false),
            mtm,
        };
        let content = panel.surface.content_view();
        for view in [
            &*panel.glyph as &NSView,
            &panel.title,
            &panel.chip,
            &panel.spinner,
            &panel.status,
            &panel.stop,
            &panel.readout,
            &panel.separator,
            panel.text.view(),
            &panel.sources,
            &panel.empty.title,
            &panel.empty.body,
            &panel.empty.settings,
            &panel.empty.detect,
        ] {
            content.addSubview(view);
        }
        content.addSubview(&panel.hints.borrow());
        panel
    }

    /// Runs `handler` with what the user asks: a citation, a link, stopping, the settings.
    pub fn on_request(&self, handler: impl Fn(PanelRequest) + 'static) {
        let handler: Handler = Rc::new(handler);
        let links = Rc::clone(&handler);
        self.text.on_link(move |url| links(PanelRequest::Open(url)));
        *self.target.ivars().handler.borrow_mut() = Some(handler);
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The panel's surface, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// The answer as shown, for copying.
    #[must_use]
    pub fn answer(&self) -> String {
        self.text.string()
    }

    /// Opens over `host` with what it holds: the answer so far, or the empty state.
    pub fn show(&self, host: &NSView, trigger: Trigger) {
        let reopened = self.is_open();
        self.lay_out(host);
        if reopened {
            return;
        }
        if let Some(scrim) = &self.scrim {
            scrim.view().setFrame(host.bounds());
            host.addSubview(scrim.view());
        }
        host.addSubview(self.surface.view());
        motion::show(self.surface.view(), motion::OVERLAY_ENTER, trigger);
        self.open.set(true);
    }

    /// Takes the panel away; what it holds stays for [`Self::show`].
    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        motion::hide(self.surface.view(), motion::FADE_FAST, Trigger::Keyboard);
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.open.set(false);
    }

    /// No endpoint is set up: the panel offers to set one.
    pub fn unconfigured(&self) {
        self.reset();
        set(&self.title, "Ask the agent");
        set(&self.chip, "");
        *self.phase.borrow_mut() = Phase::Unconfigured;
        self.refresh();
    }

    /// A new answer begins: what it is about, the agent reading the page.
    pub fn start(&self, header: &AnswerHeader) {
        self.reset();
        set(&self.title, &header.title);
        set(&self.chip, &header.model);
        *self.phase.borrow_mut() = Phase::Working(Activity::Reading);
        self.refresh();
    }

    /// What the agent is doing now. Only a change of what the status line says costs a
    /// layout; a new speed alone costs nothing here (the badge shows it).
    pub fn set_activity(&self, activity: Activity) {
        let mut phase = self.phase.borrow_mut();
        let Phase::Working(was) = *phase else {
            return;
        };
        *phase = Phase::Working(activity);
        drop(phase);
        if std::mem::discriminant(&was) != std::mem::discriminant(&activity) {
            self.refresh();
        }
    }

    /// The answer's events since the last call, in order. Text and citations go into the
    /// answer in one edit; the last event may end it.
    pub fn receive(&self, events: &[AnswerEvent]) {
        let mut pieces = Vec::new();
        let mut cited = Vec::new();
        let mut ended = None;
        for event in events {
            match event {
                AnswerEvent::Token(token) => pieces.push(LinePart::Text(token.clone())),
                AnswerEvent::Citation(citation) => {
                    pieces.push(LinePart::Cite(citation.quote.clone()));
                    cited.push(citation.clone());
                }
                AnswerEvent::Done(speed) => ended = Some(Phase::Done(wording::speed(*speed))),
                AnswerEvent::Stopped => ended = Some(Phase::Stopped),
                AnswerEvent::Error(error) => {
                    self.problem(&wording::ask_error(error));
                    ended = Some(Phase::Failed);
                }
            }
        }
        if !pieces.is_empty() {
            let edit = self.transcript.borrow_mut().push(pieces);
            self.text.apply(&edit);
        }
        let more_chips = !cited.is_empty();
        for citation in cited {
            self.add_chip(&citation);
        }
        if let Some(ended) = ended {
            *self.phase.borrow_mut() = ended;
            self.refresh();
        } else if more_chips {
            self.relayout();
        }
    }

    /// The answer could not start or failed: `message`, in plain words, under what came.
    pub fn fail(&self, message: &str) {
        self.problem(message);
        *self.phase.borrow_mut() = Phase::Failed;
        self.refresh();
    }

    /// True while the agent works on the answer.
    #[must_use]
    pub fn is_working(&self) -> bool {
        matches!(*self.phase.borrow(), Phase::Working(_))
    }

    fn problem(&self, message: &str) {
        let lead = if self.text.string().is_empty() {
            ""
        } else {
            "\n\n"
        };
        self.text.apply(&Edit {
            from: usize::MAX,
            spans: vec![Span::new(format!("{lead}{message}"), Style::Problem)],
        });
    }

    fn reset(&self) {
        *self.transcript.borrow_mut() = Transcript::default();
        self.text.clear();
        for chip in self.chips.borrow_mut().drain(..) {
            chip.removeFromSuperview();
        }
        self.target.ivars().anchors.borrow_mut().clear();
    }

    fn add_chip(&self, citation: &Citation) {
        let mut anchors = self.target.ivars().anchors.borrow_mut();
        let index = anchors.len();
        anchors.push(citation.anchor.clone());
        drop(anchors);
        if index >= MAX_CHIPS {
            return;
        }
        let target: &AnyObject = self.target.as_ref();
        let chip = button(&chip_title(&citation.quote), sel!(cite:), target, self.mtm);
        chip.setBezelStyle(NSBezelStyle::AccessoryBarAction);
        chip.setControlSize(NSControlSize::Small);
        chip.setContentTintColor(Some(&color::label()));
        chip.setTag(isize::try_from(index).unwrap_or(isize::MAX));
        chip.setToolTip(Some(&NSString::from_str(&citation.quote)));
        chip.sizeToFit();
        self.surface.content_view().addSubview(&chip);
        self.chips.borrow_mut().push(chip);
    }

    /// Puts the status, the buttons, and the hints in line with the phase.
    fn refresh(&self) {
        let phase = self.phase.borrow().clone();
        let (status, tint) = match &phase {
            Phase::Unconfigured => (String::new(), color::secondary_label()),
            Phase::Working(Activity::Reading) => {
                ("Reading the page…".to_owned(), color::secondary_label())
            }
            Phase::Working(Activity::Asking) => (
                "Waiting for the first token…".to_owned(),
                color::secondary_label(),
            ),
            Phase::Working(Activity::Answering(_)) => {
                ("Answering…".to_owned(), color::secondary_label())
            }
            Phase::Done(_) => (
                "Answered from this page".to_owned(),
                color::secondary_label(),
            ),
            Phase::Stopped => (
                "Stopped; what came so far stays".to_owned(),
                color::secondary_label(),
            ),
            Phase::Failed => ("Didn't finish".to_owned(), color::destructive()),
        };
        set(&self.status, &status);
        self.status.setTextColor(Some(&tint));
        let working = matches!(phase, Phase::Working(_));
        if working {
            // SAFETY: starting a spinner takes any sender, including none.
            unsafe { self.spinner.startAnimation(None) };
        } else {
            // SAFETY: stopping a spinner takes any sender, including none.
            unsafe { self.spinner.stopAnimation(None) };
        }
        self.stop.setHidden(!working);
        set(
            &self.readout,
            match &phase {
                Phase::Done(readout) => readout,
                _ => "",
            },
        );
        let hints: &[(&str, &str)] = if working {
            &[("⌘.", "stop"), ("⎋", "close")]
        } else {
            &[("⎋", "close")]
        };
        let fresh = hint_bar(hints, self.mtm);
        self.hints.borrow().removeFromSuperview();
        self.surface.content_view().addSubview(&fresh);
        *self.hints.borrow_mut() = fresh;
        self.relayout();
    }

    fn relayout(&self) {
        // SAFETY: a getter; the superview is retained while it is used.
        if let Some(host) = unsafe { self.surface.view().superview() } {
            self.lay_out(&host);
        }
    }

    /// Sizes the panel to `host` and places everything, top down.
    fn lay_out(&self, host: &NSView) {
        let bounds = host.bounds().size;
        let width = (bounds.width * overlay::ANSWER_PANEL_SHARE)
            .clamp(overlay::POPOVER_WIDTH, overlay::ANSWER_PANEL_WIDTH)
            .min(bounds.width - space::XL * 2.0)
            .round();
        let height = (bounds.height - space::LG * 2.0)
            .min(overlay::ANSWER_PANEL_MAX_HEIGHT)
            .round();
        self.surface
            .place_top_right(host, NSSize::new(width, height));
        let padding = space::PALETTE_PADDING;
        let inner = width - padding * 2.0;
        let mut top = height - padding;

        let header = 28.0;
        let chip_width = (inner * 0.4).round();
        place(&self.glyph, padding, top, 20.0, header);
        place(
            &self.title,
            padding + 20.0 + space::SM,
            top,
            inner - 20.0 - space::SM - chip_width - space::SM,
            header,
        );
        place(
            &self.chip,
            width - padding - chip_width,
            top,
            chip_width,
            header,
        );
        top -= header + space::SM;

        let unconfigured = *self.phase.borrow() == Phase::Unconfigured;
        for view in [
            &*self.spinner as &NSView,
            &self.status,
            &self.readout,
            &self.separator,
            self.text.view(),
        ] {
            view.setHidden(unconfigured);
        }
        for view in [
            &*self.empty.title as &NSView,
            &self.empty.body,
            &self.empty.settings,
            &self.empty.detect,
        ] {
            view.setHidden(!unconfigured);
        }
        let hint_height = self.hints.borrow().frame().size.height;
        self.hints.borrow().setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(inner, hint_height),
        ));
        let bottom = padding + hint_height + space::MD;
        if unconfigured {
            self.sources.setHidden(true);
            self.stop.setHidden(true);
            self.lay_out_empty(padding, inner, top);
            return;
        }

        let row = 20.0;
        self.spinner.setFrame(NSRect::new(
            NSPoint::new(padding, top - row + 2.0),
            NSSize::new(16.0, 16.0),
        ));
        let status_left = padding + 16.0 + space::SM;
        let side = (inner * 0.45).round();
        place(
            &self.status,
            status_left,
            top,
            inner - 16.0 - space::SM - side,
            row,
        );
        place(&self.readout, width - padding - side, top, side, row);
        let stop_size = self.stop.frame().size;
        self.stop.setFrameOrigin(NSPoint::new(
            width - padding - stop_size.width,
            top - row + ((row - stop_size.height) / 2.0).round(),
        ));
        top -= row + space::SM;
        self.separator.setFrame(NSRect::new(
            NSPoint::new(padding, top - 1.0),
            NSSize::new(inner, 1.0),
        ));
        top -= 1.0 + space::MD;

        let sources_height = self.lay_out_chips(padding, inner, bottom);
        self.text.view().setFrame(NSRect::new(
            NSPoint::new(padding, bottom + sources_height),
            NSSize::new(inner, (top - bottom - sources_height).max(0.0)),
        ));
    }

    /// Places the sources caption and the chips, wrapping, above `bottom`; returns the height
    /// they take, nothing without citations.
    fn lay_out_chips(&self, padding: CGFloat, inner: CGFloat, bottom: CGFloat) -> CGFloat {
        let chips = self.chips.borrow();
        self.sources.setHidden(chips.is_empty());
        if chips.is_empty() {
            return 0.0;
        }
        let mut rows: Vec<Vec<&Retained<NSButton>>> = vec![Vec::new()];
        let mut used = 0.0;
        for chip in chips.iter() {
            let chip_width = chip.frame().size.width.min(inner);
            if used > 0.0 && used + chip_width > inner {
                rows.push(Vec::new());
                used = 0.0;
            }
            used += chip_width + space::SM;
            if let Some(row) = rows.last_mut() {
                row.push(chip);
            }
        }
        let mut y = bottom;
        for row in rows.iter().rev() {
            let mut x = padding;
            for chip in row {
                let chip_width = chip.frame().size.width.min(inner);
                chip.setFrame(NSRect::new(
                    NSPoint::new(x, y),
                    NSSize::new(chip_width, CHIP_HEIGHT),
                ));
                x += chip_width + space::SM;
            }
            y += CHIP_HEIGHT + space::XS;
        }
        let caption = self.sources.frame().size.height;
        self.sources.setFrame(NSRect::new(
            NSPoint::new(padding, y),
            NSSize::new(inner, caption),
        ));
        y + caption + space::MD - bottom
    }

    fn lay_out_empty(&self, padding: CGFloat, inner: CGFloat, top: CGFloat) {
        let mut top = top - space::XL;
        let title_height = self.empty.title.frame().size.height;
        place(&self.empty.title, padding, top, inner, title_height);
        top -= title_height + space::MD;
        self.empty.body.setPreferredMaxLayoutWidth(inner);
        let fitting = self
            .empty
            .body
            .cell()
            .map_or(60.0, |cell| {
                cell.cellSizeForBounds(NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(inner, 1000.0),
                ))
                .height
            })
            .ceil();
        self.empty.body.setFrame(NSRect::new(
            NSPoint::new(padding, top - fitting),
            NSSize::new(inner, fitting),
        ));
        top -= fitting + space::XL;
        let settings = self.empty.settings.frame().size;
        self.empty
            .settings
            .setFrameOrigin(NSPoint::new(padding, top - settings.height));
        let detect = self.empty.detect.frame().size;
        self.empty.detect.setFrameOrigin(NSPoint::new(
            padding + settings.width + space::MD,
            top - detect.height,
        ));
    }
}

/// A chip's title: the start of the quote, cut after a whole word when it has spaces.
fn chip_title(quote: &str) -> String {
    if quote.chars().count() <= CHIP_CHARS {
        return format!("“{quote}”");
    }
    let kept: String = quote.chars().take(CHIP_CHARS - 1).collect();
    let cut_in_a_word = quote
        .chars()
        .nth(CHIP_CHARS - 1)
        .is_some_and(|next| !next.is_whitespace());
    let kept = match kept.rsplit_once(' ') {
        Some((words, _)) if cut_in_a_word => words,
        _ => kept.as_str(),
    };
    format!("“{}…”", kept.trim_end())
}

/// Puts `field` at `x`, its top at `top`, `width` wide, centered in a row `height` tall.
fn place(field: &NSTextField, x: CGFloat, top: CGFloat, width: CGFloat, height: CGFloat) {
    let own = field.frame().size.height.min(height);
    field.setFrame(NSRect::new(
        NSPoint::new(x, top - height + ((height - own) / 2.0).round()),
        NSSize::new(width.max(0.0), own),
    ));
}

fn set(field: &NSTextField, value: &str) {
    field.setStringValue(&NSString::from_str(value));
}

fn label(role: TextRole, tint: &NSColor, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(" "), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

/// A push button sending `action` to `target`, which never takes the keyboard from the page.
fn button(
    title: &str,
    action: Sel,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button.setFont(Some(&TextRole::RowDetail.font()));
    // SAFETY: the target is weak; the panel keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.setRefusesFirstResponder(true);
    button.sizeToFit();
    button
}

#[cfg(test)]
mod tests {
    use super::chip_title;

    #[test]
    fn a_short_quote_fills_its_chip() {
        assert_eq!(chip_title("the price is 42"), "“the price is 42”");
    }

    #[test]
    fn a_long_quote_is_cut_on_its_chip() {
        let title = chip_title("one two three four five six seven eight nine ten");

        assert_eq!(title, "“one two three four five six seven…”");
        assert_eq!(chip_title(&"é".repeat(36)).chars().count(), 38);
        assert_eq!(chip_title(&"é".repeat(37)).chars().count(), 38);
        assert!(chip_title(&"é".repeat(37)).ends_with("é…”"));
    }

    #[test]
    fn a_cut_at_a_space_keeps_the_last_whole_word() {
        let quote = format!("{} tail", "a".repeat(34));

        assert_eq!(chip_title(&quote), format!("“{}…”", "a".repeat(34)));
    }
}

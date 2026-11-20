//! The agent's answer panel (`docs/design/mocks/agent/ask/`, `ask-empty/`): an E2 surface docked
//! at the right of the window, so the page stays readable beside it and a cited passage scrolls
//! into view next to the answer. It is as tall as its answer, between [`MIN_HEIGHT`] and
//! `ANSWER_PANEL_MAX_HEIGHT`, and grows as the answer streams in.
//!
//! - The header names what was asked and the model chip where it runs.
//! - The status line says what the agent does; `Stop ⌘.` while it answers, the measured speed
//!   (`24 tok/s · 0.4 s TTFT`) once it is done, a red warning when it failed.
//! - The answer streams in as plain styled text ([`markdown`]); only its last line is laid out
//!   again as tokens come ([`transcript`]). Nothing in it is fetched: no image, no link.
//! - "Sources from this page": a chip per citation; choosing one finds the quote in the page.
//! - The footer ([`chrome`]) offers what comes next: follow up and copy, or retry.
//! - Without an endpoint, the empty state offers the settings and looking for a local LLM.
//!
//! The panel only shows; the browser runs every request.

mod chrome;
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
    NSBezelStyle, NSBox, NSBoxType, NSButton, NSColor, NSControlSize, NSImageView, NSLineBreakMode,
    NSProgressIndicator, NSProgressIndicatorStyle, NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_application::AnswerEvent;
use quickie_domain_agents::Citation;
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, motion, symbol};
use url::Url;

use chrome::{Empty, Footer, FooterKind};
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
/// The panel at its shortest: a question with nothing come yet.
pub const MIN_HEIGHT: CGFloat = 240.0;
/// The header row's height.
const HEADER: CGFloat = 28.0;
/// The status row's height.
const STATUS: CGFloat = 20.0;

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
    /// Ask the last question again: the answer failed.
    Retry,
    /// Put the whole answer on the pasteboard.
    Copy,
    /// Ask something more: the palette, ready for a question.
    FollowUp,
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

impl Phase {
    fn footer(&self) -> FooterKind {
        match self {
            Self::Unconfigured => FooterKind::Setup,
            Self::Working(_) => FooterKind::Working,
            Self::Done(_) | Self::Stopped => FooterKind::Answered,
            Self::Failed => FooterKind::Failed,
        }
    }
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

        #[unsafe(method(retry:))]
        fn retry(&self, _sender: Option<&NSButton>) {
            self.ask(PanelRequest::Retry);
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&NSButton>) {
            self.ask(PanelRequest::Copy);
        }

        #[unsafe(method(follow:))]
        fn follow(&self, _sender: Option<&NSButton>) {
            self.ask(PanelRequest::FollowUp);
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

pub struct AgentAnswerPanel {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    glyph: Retained<NSTextField>,
    title: Retained<NSTextField>,
    chip: RefCell<Retained<NSView>>,
    spinner: Retained<NSProgressIndicator>,
    warning: Option<Retained<NSImageView>>,
    status: Retained<NSTextField>,
    stop: Retained<NSButton>,
    readout: Retained<NSTextField>,
    separator: Retained<NSBox>,
    text: AnswerText,
    sources: Retained<NSTextField>,
    chips: RefCell<Vec<Retained<NSButton>>>,
    empty: Empty,
    footer: Footer,
    transcript: RefCell<Transcript>,
    phase: RefCell<Phase>,
    /// How tall the panel is now, to grow it only when the answer needs more.
    height: Cell<CGFloat>,
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
        stop.setContentTintColor(Some(&color::destructive()));
        let readout = label(TextRole::Caption, &color::secondary_label(), mtm);
        readout.setFont(Some(&TextRole::Caption.mono_font()));
        readout.setAlignment(NSTextAlignment::Right);
        let sources = label(TextRole::Caption, &color::secondary_label(), mtm);
        set(&sources, "Sources from this page");
        let separator = NSBox::new(mtm);
        separator.setBoxType(NSBoxType::Separator);
        // SAFETY: the target implements `settings:` and `detect:`, and the panel keeps it
        // alive for as long as the empty state's buttons.
        let empty = unsafe { Empty::new(target_object, mtm) };
        let panel = Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface: GlassSurface::new(Elevation::E2, mtm),
            glyph,
            title: label(TextRole::RowTitle, &color::label(), mtm),
            chip: RefCell::new(NSView::new(mtm)),
            spinner,
            warning: symbol(
                "exclamationmark.triangle.fill",
                "Failed",
                TextRole::RowDetail,
                &color::destructive(),
                mtm,
            ),
            status: label(TextRole::RowDetail, &color::secondary_label(), mtm),
            stop,
            readout,
            separator,
            text: AnswerText::new(mtm),
            sources,
            chips: RefCell::new(Vec::new()),
            empty,
            footer: Footer::new(mtm),
            transcript: RefCell::new(Transcript::default()),
            phase: RefCell::new(Phase::Unconfigured),
            height: Cell::new(MIN_HEIGHT),
            target,
            open: Cell::new(false),
            mtm,
        };
        let content = panel.surface.content_view();
        for view in [
            &*panel.glyph as &NSView,
            &panel.title,
            &panel.spinner,
            &panel.status,
            &panel.stop,
            &panel.readout,
            &panel.separator,
            panel.text.view(),
            &panel.sources,
            panel.empty.view(),
            panel.footer.view(),
        ] {
            content.addSubview(view);
        }
        if let Some(warning) = &panel.warning {
            content.addSubview(warning);
        }
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

    /// What `⏎` does now: follow up on an answer, retry a failed one; nothing while the agent
    /// works or without an endpoint.
    #[must_use]
    pub fn return_request(&self) -> Option<PanelRequest> {
        match self.phase.borrow().footer() {
            FooterKind::Answered => Some(PanelRequest::FollowUp),
            FooterKind::Failed => Some(PanelRequest::Retry),
            FooterKind::Setup | FooterKind::Working => None,
        }
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
        self.set_chip("");
        *self.phase.borrow_mut() = Phase::Unconfigured;
        self.refresh();
    }

    /// A new answer begins: what it is about, the agent reading the page.
    pub fn start(&self, header: &AnswerHeader) {
        self.reset();
        set(&self.title, &header.title);
        self.set_chip(&header.model);
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
    /// answer in one edit; the last event may end it. The panel grows with the text until it
    /// reaches its tallest.
    pub fn receive(&self, events: &[AnswerEvent]) {
        let mut pieces = Vec::new();
        let mut cited = Vec::new();
        let mut ended = None;
        let mut problem = None;
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
                    problem = Some(wording::ask_error(error));
                    ended = Some(Phase::Failed);
                }
            }
        }
        if !pieces.is_empty() {
            let edit = self.transcript.borrow_mut().push(pieces);
            self.text.apply(&edit);
        }
        // After the text: its edit rewrites the last line, which would take the problem along.
        if let Some(problem) = problem {
            self.problem(&problem);
        }
        let more_chips = !cited.is_empty();
        for citation in cited {
            self.add_chip(&citation);
        }
        if let Some(ended) = ended {
            *self.phase.borrow_mut() = ended;
            self.refresh();
        } else if more_chips || self.wants_to_grow() {
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
        self.height.set(MIN_HEIGHT);
    }

    fn set_chip(&self, model: &str) {
        let chip = chrome::model_chip(model, self.mtm);
        self.chip.borrow().removeFromSuperview();
        self.surface.content_view().addSubview(&chip);
        *self.chip.borrow_mut() = chip;
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

    /// Puts the status, the buttons, and the footer in line with the phase.
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
        if let Some(warning) = &self.warning {
            warning.setHidden(phase != Phase::Failed);
        }
        set(
            &self.readout,
            match &phase {
                Phase::Done(readout) => readout,
                _ => "",
            },
        );
        let target: &AnyObject = self.target.as_ref();
        // SAFETY: the target implements the footer's actions, and the panel keeps it alive
        // for as long as the footer's buttons.
        unsafe { self.footer.set(phase.footer(), target) };
        self.relayout();
    }

    fn relayout(&self) {
        // SAFETY: a getter; the superview is retained while it is used.
        if let Some(host) = unsafe { self.surface.view().superview() } {
            self.lay_out(&host);
        }
    }

    /// The panel's width over a host `bounds` wide.
    fn width(bounds: NSSize) -> CGFloat {
        (bounds.width * overlay::ANSWER_PANEL_SHARE)
            .clamp(overlay::POPOVER_WIDTH, overlay::ANSWER_PANEL_WIDTH)
            .min(bounds.width - space::XL * 2.0)
            .round()
    }

    /// The tallest the panel may be over a host `bounds` high.
    fn tallest(bounds: NSSize) -> CGFloat {
        (bounds.height - space::LG * 2.0)
            .min(overlay::ANSWER_PANEL_MAX_HEIGHT)
            .round()
    }

    /// True when the text has outgrown the panel and the panel can still grow.
    fn wants_to_grow(&self) -> bool {
        // SAFETY: a getter; the superview is retained while it is used.
        let Some(host) = (unsafe { self.surface.view().superview() }) else {
            return false;
        };
        let bounds = host.bounds().size;
        let height = self.height.get();
        if height >= Self::tallest(bounds) {
            return false;
        }
        let inner = Self::width(bounds) - space::PALETTE_PADDING * 2.0;
        let visible = self.text.view().frame().size.height;
        self.text.content_height(inner) > visible + 1.0
    }

    /// Sizes the panel to `host` and its content, and places everything, top down.
    fn lay_out(&self, host: &NSView) {
        let bounds = host.bounds().size;
        let width = Self::width(bounds);
        let padding = space::PALETTE_PADDING;
        let inner = width - padding * 2.0;
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
        self.empty.view().setHidden(!unconfigured);
        if unconfigured {
            self.sources.setHidden(true);
            self.stop.setHidden(true);
            if let Some(warning) = &self.warning {
                warning.setHidden(true);
            }
        }
        let footer = self.footer.lay_out(inner);
        let chips = self.chips_height(inner);
        let body = if unconfigured {
            self.empty.lay_out(inner)
        } else {
            STATUS + space::SM + 1.0 + space::MD + self.text.content_height(inner) + chips
        };
        let wanted = padding + HEADER + space::SM + body + space::MD + footer + padding;
        let height = wanted
            .max(self.height.get())
            .clamp(MIN_HEIGHT, Self::tallest(bounds).max(MIN_HEIGHT))
            .ceil();
        self.height.set(height);
        self.surface
            .place_top_right(host, NSSize::new(width, height));

        let mut top = height - padding;
        self.lay_out_header(padding, inner, top);
        top -= HEADER + space::SM;
        self.footer
            .view()
            .setFrameOrigin(NSPoint::new(padding, padding));
        let bottom = padding + footer + space::MD;
        if unconfigured {
            let column = self.empty.view().frame().size.height;
            self.empty
                .view()
                .setFrameOrigin(NSPoint::new(padding, top - column));
            return;
        }
        self.lay_out_status(padding, inner, top);
        top -= STATUS + space::SM;
        self.separator.setFrame(NSRect::new(
            NSPoint::new(padding, top - 1.0),
            NSSize::new(inner, 1.0),
        ));
        top -= 1.0 + space::MD;
        self.lay_out_chips(padding, inner, bottom);
        self.text.view().setFrame(NSRect::new(
            NSPoint::new(padding, bottom + chips),
            NSSize::new(inner, (top - bottom - chips).max(0.0)),
        ));
    }

    /// The glyph, the question, and the model chip at the right.
    fn lay_out_header(&self, padding: CGFloat, inner: CGFloat, top: CGFloat) {
        let chip = self.chip.borrow();
        let chip_size = chip.frame().size;
        let chip_width = chip_size.width.min((inner * 0.45).round());
        chip.setFrame(NSRect::new(
            NSPoint::new(
                padding + inner - chip_width,
                top - HEADER + ((HEADER - chip_size.height) / 2.0).round(),
            ),
            NSSize::new(chip_width, chip_size.height),
        ));
        place(&self.glyph, padding, top, 20.0, HEADER);
        let title_left = padding + 20.0 + space::SM;
        place(
            &self.title,
            title_left,
            top,
            inner - 20.0 - space::SM - chip_width - space::SM,
            HEADER,
        );
    }

    /// The spinner or the warning, what the agent does, and Stop or the measured speed.
    fn lay_out_status(&self, padding: CGFloat, inner: CGFloat, top: CGFloat) {
        let width = padding * 2.0 + inner;
        self.spinner.setFrame(NSRect::new(
            NSPoint::new(padding, top - STATUS + 2.0),
            NSSize::new(16.0, 16.0),
        ));
        if let Some(warning) = &self.warning {
            let size = warning.frame().size;
            warning.setFrameOrigin(NSPoint::new(
                padding,
                top - STATUS + ((STATUS - size.height) / 2.0).round(),
            ));
        }
        let status_left = padding + 16.0 + space::SM;
        let side = (inner * 0.45).round();
        place(
            &self.status,
            status_left,
            top,
            inner - 16.0 - space::SM - side,
            STATUS,
        );
        place(&self.readout, width - padding - side, top, side, STATUS);
        let stop_size = self.stop.frame().size;
        self.stop.setFrameOrigin(NSPoint::new(
            width - padding - stop_size.width,
            top - STATUS + ((STATUS - stop_size.height) / 2.0).round(),
        ));
    }

    /// The chips wrapped into rows `inner` wide.
    fn chip_rows(&self, inner: CGFloat) -> Vec<Vec<Retained<NSButton>>> {
        let chips = self.chips.borrow();
        let mut rows: Vec<Vec<Retained<NSButton>>> = vec![Vec::new()];
        let mut used = 0.0;
        for chip in chips.iter() {
            let chip_width = chip.frame().size.width.min(inner);
            if used > 0.0 && used + chip_width > inner {
                rows.push(Vec::new());
                used = 0.0;
            }
            used += chip_width + space::SM;
            if let Some(row) = rows.last_mut() {
                row.push(chip.clone());
            }
        }
        rows
    }

    /// The height of the sources caption and the chips; nothing without citations.
    fn chips_height(&self, inner: CGFloat) -> CGFloat {
        if self.chips.borrow().is_empty() {
            return 0.0;
        }
        let rows = CGFloat::from(u32::try_from(self.chip_rows(inner).len()).unwrap_or(1));
        rows * (CHIP_HEIGHT + space::XS) + self.sources.frame().size.height + space::MD
    }

    /// Places the sources caption and the chips, wrapping, above `bottom`.
    fn lay_out_chips(&self, padding: CGFloat, inner: CGFloat, bottom: CGFloat) {
        let empty = self.chips.borrow().is_empty();
        self.sources.setHidden(empty);
        if empty {
            return;
        }
        let mut y = bottom;
        for row in self.chip_rows(inner).iter().rev() {
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

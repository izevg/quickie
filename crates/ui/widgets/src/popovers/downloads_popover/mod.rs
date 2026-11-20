//! The downloads popover (`⌘J`, `docs/design/mocks/system/downloads/`): an E2 surface at the
//! top right with the newest downloads first. A row shows the file, a bar and the measured
//! speed while it runs, how it ended otherwise, and its buttons: cancel, open, show in Finder,
//! retry. "Clear Finished" takes the ended ones off the list.
//!
//! Opened from the keyboard it appears at once and the first row is selected: `↑` `↓` select,
//! `⏎` opens a finished download (retries an ended one), `⌘⏎` shows it in the Finder, `⎋`
//! closes. The popover only shows; the browser runs every request and answers with
//! [`DownloadsPopover::update`].

mod download_row;
mod line;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSButton, NSImageView, NSTextAlignment, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_domain_library::{Download, DownloadId, DownloadState};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, hint_bar, motion, symbol};

use download_row::{DownloadRow, ROW_HEIGHT};
use line::{Action, Line, Phase, RateMeter, line};

/// How many downloads show; the newest.
pub const MAX_ROWS: usize = 6;
/// The header's height.
const HEADER: CGFloat = 24.0;
/// The empty state's height.
const EMPTY_HEIGHT: CGFloat = 96.0;

/// What the popover asks of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadsRequest {
    /// Open the finished file with its app.
    Open(DownloadId),
    /// Show the file in the Finder.
    Reveal(DownloadId),
    /// Stop a running download.
    Cancel(DownloadId),
    /// Fetch an ended download again.
    Retry(DownloadId),
    /// Take every ended download off the list.
    ClearFinished,
}

type Handler = Rc<dyn Fn(DownloadsRequest)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
    /// The lines on screen, row by row, to tell which download a button is for.
    lines: Rc<RefCell<Vec<Line>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `DownloadsTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieDownloadsPopoverTarget"]
    #[ivars = Ivars]
    pub struct DownloadsTarget;

    unsafe impl NSObjectProtocol for DownloadsTarget {}

    impl DownloadsTarget {
        #[unsafe(method(primary:))]
        fn primary(&self, sender: Option<&NSButton>) {
            self.button(sender, 0);
        }

        #[unsafe(method(secondary:))]
        fn secondary(&self, sender: Option<&NSButton>) {
            self.button(sender, 1);
        }

        #[unsafe(method(clear:))]
        fn clear(&self, _sender: Option<&NSButton>) {
            self.ask(DownloadsRequest::ClearFinished);
        }
    }
);

impl DownloadsTarget {
    fn new(lines: Rc<RefCell<Vec<Line>>>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
            lines,
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    /// A row's button `slot` was clicked.
    fn button(&self, sender: Option<&NSButton>, slot: usize) {
        let Some(row) = sender.and_then(|button| usize::try_from(button.tag()).ok()) else {
            return;
        };
        let request = {
            let lines = self.ivars().lines.borrow();
            lines.get(row).and_then(|line| {
                let action = *line.phase.actions().get(slot)?;
                Some(request(action, line.id))
            })
        };
        if let Some(request) = request {
            self.ask(request);
        }
    }

    fn ask(&self, request: DownloadsRequest) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }
}

pub struct DownloadsPopover {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    glyph: Option<Retained<NSImageView>>,
    title: Retained<NSTextField>,
    clear: Retained<NSButton>,
    empty_glyph: Option<Retained<NSImageView>>,
    empty: Retained<NSTextField>,
    folder: Retained<NSTextField>,
    folder_glyph: Option<Retained<NSImageView>>,
    more: Retained<NSTextField>,
    hints: Retained<NSTextField>,
    rows: Vec<DownloadRow>,
    lines: Rc<RefCell<Vec<Line>>>,
    selected: Cell<Option<usize>>,
    meters: RefCell<HashMap<DownloadId, RateMeter>>,
    epoch: Instant,
    target: Retained<DownloadsTarget>,
    open: Cell<bool>,
}

impl DownloadsPopover {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let lines = Rc::new(RefCell::new(Vec::new()));
        let target = DownloadsTarget::new(Rc::clone(&lines), mtm);
        let target_object: &AnyObject = target.as_ref();
        let title = text("Downloads", TextRole::Headline, &color::label(), mtm);
        let glyph = symbol(
            "arrow.down.circle.fill",
            "",
            TextRole::Headline,
            &color::accent(),
            mtm,
        );
        let empty_glyph = symbol("tray", "", TextRole::Title, &color::tertiary_label(), mtm);
        let folder = text("", TextRole::Caption, &color::secondary_label(), mtm);
        folder.setFont(Some(&TextRole::Caption.mono_font()));
        folder.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
        let folder_glyph = symbol(
            "folder",
            "Downloads folder",
            TextRole::Caption,
            &color::secondary_label(),
            mtm,
        );
        let clear = NSButton::new(mtm);
        clear.setTitle(&NSString::from_str("Clear Finished"));
        clear.setBordered(false);
        clear.setFont(Some(&TextRole::RowDetail.font()));
        clear.setContentTintColor(Some(&color::accent()));
        clear.sizeToFit();
        // SAFETY: the target is weak; the popover keeps it alive for as long as the button.
        unsafe { clear.setTarget(Some(target_object)) };
        // SAFETY: the target implements `clear:`, which takes the sender.
        unsafe { clear.setAction(Some(sel!(clear:))) };
        clear.setRefusesFirstResponder(true);
        let empty = text(
            "Nothing downloaded yet",
            TextRole::RowDetail,
            &color::secondary_label(),
            mtm,
        );
        empty.setAlignment(NSTextAlignment::Center);
        let more = text("", TextRole::Caption, &color::secondary_label(), mtm);
        let hints = hint_bar(
            &[
                ("↑↓", "select"),
                ("⏎", "open"),
                ("⌘⏎", "show in Finder"),
                ("⎋", "close"),
            ],
            mtm,
        );
        let rows: Vec<DownloadRow> = (0..MAX_ROWS)
            .map(|index| DownloadRow::new(index, target_object, mtm))
            .collect();
        let content = surface.content_view();
        for view in [&*title as &NSView, &clear, &empty, &more, &hints, &folder] {
            content.addSubview(view);
        }
        for view in [&glyph, &empty_glyph, &folder_glyph].into_iter().flatten() {
            content.addSubview(view);
        }
        for row in &rows {
            content.addSubview(row.view());
        }
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            glyph,
            title,
            clear,
            empty_glyph,
            empty,
            folder,
            folder_glyph,
            more,
            hints,
            rows,
            lines,
            selected: Cell::new(None),
            meters: RefCell::new(HashMap::new()),
            epoch: Instant::now(),
            target,
            open: Cell::new(false),
        }
    }

    /// Names the folder downloads go to, in the footer: `~/Downloads`.
    pub fn set_folder(&self, folder: &str) {
        self.folder.setStringValue(&NSString::from_str(folder));
        self.folder.sizeToFit();
    }

    /// Runs `handler` with what the user asks.
    pub fn on_request(&self, handler: impl Fn(DownloadsRequest) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens over `host` with `downloads` (in the order they started), the newest selected.
    pub fn show(&self, host: &NSView, downloads: &[Download], trigger: Trigger) {
        self.meters.borrow_mut().clear();
        self.selected.set(None);
        self.fill(downloads);
        self.selected
            .set((!self.lines.borrow().is_empty()).then_some(0));
        self.lay_out(host);
        if !self.is_open() {
            if let Some(scrim) = &self.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(self.surface.view());
            motion::show(self.surface.view(), motion::OVERLAY_ENTER, trigger);
            self.open.set(true);
        }
    }

    /// Shows the downloads as they are now, keeping the selected one selected.
    pub fn update(&self, downloads: &[Download]) {
        if !self.is_open() {
            return;
        }
        let selected = self.selected();
        self.fill(downloads);
        let lines = self.lines.borrow();
        let index = selected
            .and_then(|id| lines.iter().position(|line| line.id == id))
            .or_else(|| (!lines.is_empty()).then_some(0));
        drop(lines);
        self.selected.set(index);
        // SAFETY: a getter; the superview is retained while it is used.
        if let Some(host) = unsafe { self.surface.view().superview() } {
            self.lay_out(&host);
        }
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        motion::hide(
            self.surface.view(),
            motion::OVERLAY_ENTER,
            Trigger::Keyboard,
        );
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.open.set(false);
    }

    /// Moves the selection by `delta` rows, stopping at the ends.
    pub fn move_selection(&self, delta: isize) {
        let count = self.lines.borrow().len();
        let Some(current) = self.selected.get() else {
            return;
        };
        let next = current
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
        if next != current {
            self.selected.set(Some(next));
            self.paint();
        }
    }

    /// The selected download.
    #[must_use]
    pub fn selected(&self) -> Option<DownloadId> {
        let index = self.selected.get()?;
        self.lines.borrow().get(index).map(|line| line.id)
    }

    /// What `⏎` asks for the selected download, or `⌘⏎` when `reveal`: open or show a finished
    /// one, retry one that ended otherwise; nothing for one still running.
    #[must_use]
    pub fn enter(&self, reveal: bool) -> Option<DownloadsRequest> {
        let index = self.selected.get()?;
        let lines = self.lines.borrow();
        let line = lines.get(index)?;
        line::on_enter(line.phase, reveal).map(|action| request(action, line.id))
    }

    /// Clicks button `button` of row `row`, as the pointer does.
    pub fn press(&self, row: usize, button: usize) {
        if let Some(row) = self.rows.get(row) {
            row.press(button);
        }
    }

    /// The popover's surface, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// The details on screen, row by row, for a check.
    #[must_use]
    pub fn details(&self) -> Vec<String> {
        self.lines
            .borrow()
            .iter()
            .map(|line| format!("{}: {}", line.name, line.detail))
            .collect()
    }

    /// The lines of the newest downloads, their speeds measured from the last samples.
    fn fill(&self, downloads: &[Download]) {
        let seconds = self.epoch.elapsed().as_secs_f64();
        let mut meters = self.meters.borrow_mut();
        let lines: Vec<Line> = downloads
            .iter()
            .rev()
            .take(MAX_ROWS)
            .map(|download| {
                let rate = match download.state {
                    DownloadState::InProgress { received, .. } => meters
                        .entry(download.id)
                        .or_default()
                        .sample(received, seconds),
                    _ => None,
                };
                line(download, rate)
            })
            .collect();
        meters.retain(|id, _| lines.iter().any(|line| line.id == *id));
        drop(meters);
        let hidden = downloads.len().saturating_sub(MAX_ROWS);
        self.more
            .setStringValue(&NSString::from_str(&if hidden == 0 {
                String::new()
            } else {
                format!("and {hidden} older")
            }));
        *self.lines.borrow_mut() = lines;
    }

    /// Sizes the surface to its rows and places everything, top down.
    fn lay_out(&self, host: &NSView) {
        let lines = self.lines.borrow();
        let padding = space::PALETTE_PADDING;
        let width = overlay::POPOVER_WIDTH.min(host.bounds().size.width - space::XL * 2.0);
        let inner = width - padding * 2.0;
        let shown = lines.len();
        let list = if shown == 0 {
            EMPTY_HEIGHT
        } else {
            ROW_HEIGHT * as_float(shown) + space::XS * as_float(shown - 1)
        };
        let more = if self.more.stringValue().length() == 0 {
            0.0
        } else {
            self.more.frame().size.height + space::XS
        };
        let hints = self.hints.frame().size.height;
        let folder = if self.folder.stringValue().length() == 0 {
            0.0
        } else {
            self.folder.frame().size.height + space::SM
        };
        let height =
            padding + HEADER + space::SM + list + more + space::MD + folder + hints + padding;
        self.surface
            .place_top_right(host, NSSize::new(width, height));
        let mut top = height - padding;
        let mut title_left = padding;
        if let Some(glyph) = &self.glyph {
            let size = glyph.frame().size;
            glyph.setFrameOrigin(NSPoint::new(
                padding,
                top - HEADER + ((HEADER - size.height) / 2.0).round(),
            ));
            title_left += size.width + space::SM;
        }
        let title = self.title.frame().size;
        self.title.setFrame(NSRect::new(
            NSPoint::new(
                title_left,
                top - HEADER + ((HEADER - title.height) / 2.0).round(),
            ),
            NSSize::new(inner / 2.0, title.height),
        ));
        let clear = self.clear.frame().size;
        self.clear.setFrameOrigin(NSPoint::new(
            width - padding - clear.width,
            top - HEADER + ((HEADER - clear.height) / 2.0).round(),
        ));
        self.clear
            .setHidden(!lines.iter().any(|line| line.phase != Phase::Running));
        top -= HEADER + space::SM;
        self.lay_out_empty(padding, inner, top, shown == 0);
        for (index, row) in self.rows.iter().enumerate() {
            row.view().setHidden(index >= shown);
            if index < shown {
                let y = top - ROW_HEIGHT - as_float(index) * (ROW_HEIGHT + space::XS);
                row.view().setFrameOrigin(NSPoint::new(padding, y));
            }
        }
        top -= list;
        if more > 0.0 {
            let size = self.more.frame().size;
            self.more.setFrame(NSRect::new(
                NSPoint::new(padding + space::SM, top - more),
                NSSize::new(inner, size.height),
            ));
        }
        self.more.setHidden(more == 0.0);
        self.hints.setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(inner, hints),
        ));
        self.lay_out_folder(padding, inner, padding + hints + space::SM);
        drop(lines);
        self.paint();
    }

    /// Nothing downloaded: a tray, centred, and what will show here.
    fn lay_out_empty(&self, padding: CGFloat, inner: CGFloat, top: CGFloat, empty: bool) {
        self.empty.setHidden(!empty);
        if let Some(glyph) = &self.empty_glyph {
            glyph.setHidden(!empty);
        }
        if !empty {
            return;
        }
        let text = self.empty.frame().size;
        let glyph = self
            .empty_glyph
            .as_ref()
            .map_or(NSSize::new(0.0, 0.0), |glyph| glyph.frame().size);
        let stack = glyph.height + space::SM + text.height;
        let mut y = top - ((EMPTY_HEIGHT - stack) / 2.0).round();
        if let Some(view) = &self.empty_glyph {
            y -= glyph.height;
            view.setFrameOrigin(NSPoint::new(
                padding + ((inner - glyph.width) / 2.0).round(),
                y,
            ));
            y -= space::SM;
        }
        self.empty.setFrame(NSRect::new(
            NSPoint::new(padding, y - text.height),
            NSSize::new(inner, text.height),
        ));
    }

    /// The folder downloads go to, above the hints, at `bottom`.
    fn lay_out_folder(&self, padding: CGFloat, inner: CGFloat, bottom: CGFloat) {
        let named = self.folder.stringValue().length() > 0;
        self.folder.setHidden(!named);
        if let Some(glyph) = &self.folder_glyph {
            glyph.setHidden(!named);
        }
        if !named {
            return;
        }
        let height = self.folder.frame().size.height;
        let mut left = padding;
        if let Some(glyph) = &self.folder_glyph {
            let size = glyph.frame().size;
            glyph.setFrameOrigin(NSPoint::new(
                left,
                bottom + ((height - size.height) / 2.0).round(),
            ));
            left += size.width + space::XS;
        }
        self.folder.setFrame(NSRect::new(
            NSPoint::new(left, bottom),
            NSSize::new(padding + inner - left, height),
        ));
    }

    /// Puts each line in its row, the selected one filled.
    fn paint(&self) {
        let lines = self.lines.borrow();
        let width = self.surface.view().frame().size.width - space::PALETTE_PADDING * 2.0;
        for (index, (row, line)) in self.rows.iter().zip(lines.iter()).enumerate() {
            row.show(line, self.selected.get() == Some(index), width);
        }
    }
}

/// The request of `action` on download `id`.
fn request(action: Action, id: DownloadId) -> DownloadsRequest {
    match action {
        Action::Cancel => DownloadsRequest::Cancel(id),
        Action::Reveal => DownloadsRequest::Reveal(id),
        Action::Open => DownloadsRequest::Open(id),
        Action::Retry => DownloadsRequest::Retry(id),
    }
}

fn as_float(count: usize) -> CGFloat {
    CGFloat::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn text(
    value: &str,
    role: TextRole,
    tint: &objc2_app_kit::NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(value), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.sizeToFit();
    if value.is_empty() {
        field.setStringValue(&NSString::from_str(" "));
        field.sizeToFit();
        field.setStringValue(&NSString::new());
    }
    field
}

//! The shields popover (`docs/design/mocks/system/shields/`): an E2 surface at the top right
//! with what Quickie does on the active page's site. A status line in green while shields are
//! up; tiles of measured numbers, how many requests the page had blocked when the engine counts
//! them (`—` when it does not) and how many filter lists are on; the per-site protections,
//! shields, HTTPS-only, and JavaScript (kept for the site; the last two take effect in a later
//! version, and the rows say so),
//! the enabled filter lists with when each was fetched, a button to fetch them again, and what
//! the content blocker made of them ("Rules active: …", "Compiling rules…", "Failed: …").
//! With no list on, the header and the lists say so, and one click enables them.
//!
//! Turning shields off or on changes what the next load blocks, so the popover then offers
//! "Reload to Apply" (`⏎`). Numbers shown are measured; nothing is claimed that is not.
//!
//! The popover only shows; the browser runs every request and answers with
//! [`ShieldsPopover::update`].

mod filter_list_row;
mod stat_tile;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSBox, NSBoxType, NSButton, NSImage, NSImageSymbolConfiguration, NSImageView,
    NSLineBreakMode, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_domain_shields::{FilterListId, SitePolicy, Timestamp};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{
    Elevation, GlassSurface, Scrim, ToggleRow, Trigger, hint_bar, motion, symbol,
};

use filter_list_row::{FilterListRow, ROW_HEIGHT};
use stat_tile::{StatTile, TILE_HEIGHT};

/// The symbol before each switch, as the mock draws them.
const TOGGLE_SYMBOLS: [(SiteToggle, &str); 3] = [
    (SiteToggle::Shields, "eye.slash"),
    (SiteToggle::HttpsOnly, "lock"),
    (SiteToggle::JavaScript, "curlybraces"),
];
/// Room the symbols take left of the switches.
const ICON_COLUMN: CGFloat = 24.0;

/// What toggles on the site's shields are honest about: stored now, enforced later.
const LATER: &str = "Saved; enforced in a later version";

/// What the popover shows about the active page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShieldsSummary {
    /// The site's name (its registrable domain), `None` for a page off the web.
    pub site: Option<String>,
    /// The site's policy, `None` for a page off the web.
    pub policy: Option<SitePolicy>,
    /// Requests the page had blocked, when the engine counts them.
    pub blocked: Option<u32>,
    /// The enabled filter lists and when each was last fetched.
    pub lists: Vec<(FilterListId, Option<Timestamp>)>,
    /// The clock, for how long ago a list was fetched.
    pub now: Timestamp,
    /// Shields changed since the page loaded: a reload applies them.
    pub reload_offered: bool,
    /// What the content blocker made of the lists, as measured; empty for nothing to say.
    pub rules: String,
}

/// A switch on the site's policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteToggle {
    Shields,
    HttpsOnly,
    JavaScript,
}

/// What the popover asks of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShieldsRequest {
    /// Set one switch of the site's policy.
    Set(SiteToggle, bool),
    /// Fetch the enabled lists again.
    RefreshLists,
    /// Turn the filter lists on, which fetches them.
    EnableLists,
    /// Reload the page so the shields apply.
    Reload,
}

type Handler = Rc<dyn Fn(ShieldsRequest)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ShieldsTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieShieldsPopoverTarget"]
    #[ivars = Ivars]
    pub struct ShieldsTarget;

    unsafe impl NSObjectProtocol for ShieldsTarget {}

    impl ShieldsTarget {
        #[unsafe(method(refresh:))]
        fn refresh(&self, _sender: Option<&NSButton>) {
            self.ask(ShieldsRequest::RefreshLists);
        }

        #[unsafe(method(enable:))]
        fn enable(&self, _sender: Option<&NSButton>) {
            self.ask(ShieldsRequest::EnableLists);
        }

        #[unsafe(method(reload:))]
        fn reload(&self, _sender: Option<&NSButton>) {
            self.ask(ShieldsRequest::Reload);
        }
    }
);

impl ShieldsTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn ask(&self, request: ShieldsRequest) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }
}

pub struct ShieldsPopover {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    glyph: Retained<NSImageView>,
    site: Retained<NSTextField>,
    status: Retained<NSTextField>,
    tiles: [StatTile; 2],
    protections: Retained<NSTextField>,
    icons: Vec<Retained<NSImageView>>,
    toggles: [(SiteToggle, ToggleRow); 3],
    separators: [Retained<NSBox>; 2],
    lists_title: Retained<NSTextField>,
    refresh: Retained<NSButton>,
    enable: Retained<NSButton>,
    no_lists: Retained<NSTextField>,
    list_rows: RefCell<Vec<FilterListRow>>,
    rules: Retained<NSTextField>,
    reload: Retained<NSButton>,
    hints: RefCell<Retained<NSView>>,
    target: Retained<ShieldsTarget>,
    summary: RefCell<Option<ShieldsSummary>>,
    open: Cell<bool>,
    mtm: MainThreadMarker,
}

impl ShieldsPopover {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let target = ShieldsTarget::new(mtm);
        let target_object: &AnyObject = target.as_ref();
        let toggles = [
            (
                SiteToggle::Shields,
                ToggleRow::new(
                    "Shields",
                    "Block trackers and ads with the filter lists",
                    mtm,
                ),
            ),
            (
                SiteToggle::HttpsOnly,
                ToggleRow::new("HTTPS only", LATER, mtm),
            ),
            (
                SiteToggle::JavaScript,
                ToggleRow::new("JavaScript", LATER, mtm),
            ),
        ];
        let popover = Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            glyph: NSImageView::new(mtm),
            site: text(TextRole::RowTitle, &color::label(), mtm),
            status: text(TextRole::RowDetail, &color::secondary_label(), mtm),
            tiles: [StatTile::new(mtm), StatTile::new(mtm)],
            protections: text(TextRole::Caption, &color::secondary_label(), mtm),
            icons: TOGGLE_SYMBOLS
                .iter()
                .filter_map(|(_, name)| {
                    symbol(name, "", TextRole::RowTitle, &color::secondary_label(), mtm)
                })
                .collect(),
            toggles,
            separators: [separator(mtm), separator(mtm)],
            lists_title: text(TextRole::Caption, &color::secondary_label(), mtm),
            refresh: button("Refresh", sel!(refresh:), target_object, mtm),
            enable: button("Enable", sel!(enable:), target_object, mtm),
            no_lists: text(TextRole::RowDetail, &color::secondary_label(), mtm),
            list_rows: RefCell::new(Vec::new()),
            rules: text(TextRole::Caption, &color::secondary_label(), mtm),
            reload: button("Reload to Apply", sel!(reload:), target_object, mtm),
            hints: RefCell::new(hint_bar(&[("⎋", "close")], mtm)),
            target,
            summary: RefCell::new(None),
            open: Cell::new(false),
            surface,
            mtm,
        };
        popover.site.setFont(Some(&TextRole::RowTitle.mono_font()));
        popover
            .lists_title
            .setStringValue(&NSString::from_str("FILTER LISTS"));
        popover
            .protections
            .setStringValue(&NSString::from_str("PER-SITE PROTECTIONS"));
        for caption in [&popover.lists_title, &popover.protections] {
            caption.setFont(Some(&TextRole::Caption.mono_font()));
            caption.sizeToFit();
        }
        popover
            .no_lists
            .setStringValue(&NSString::from_str("No filter lists enabled"));
        popover.reload.setBezelStyle(NSBezelStyle::Push);
        popover.reload.setKeyEquivalent(&NSString::new());
        popover.reload.sizeToFit();
        let content = popover.surface.content_view();
        for view in [
            &*popover.glyph as &NSView,
            &popover.site,
            &popover.status,
            &popover.protections,
            popover.tiles[0].view(),
            popover.tiles[1].view(),
            &popover.lists_title,
            &popover.refresh,
            &popover.enable,
            &popover.no_lists,
            &popover.rules,
            &popover.reload,
        ] {
            content.addSubview(view);
        }
        for separator in &popover.separators {
            content.addSubview(separator);
        }
        for icon in &popover.icons {
            content.addSubview(icon);
        }
        for (_, row) in &popover.toggles {
            content.addSubview(row.view());
        }
        content.addSubview(&popover.hints.borrow());
        popover
    }

    /// Runs `handler` with what the user asks.
    pub fn on_request(&self, handler: impl Fn(ShieldsRequest) + 'static) {
        let handler: Handler = Rc::new(handler);
        for (toggle, row) in &self.toggles {
            let handler = Rc::clone(&handler);
            let toggle = *toggle;
            row.on_change(move |on| handler(ShieldsRequest::Set(toggle, on)));
        }
        *self.target.ivars().handler.borrow_mut() = Some(handler);
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens over `host` with `summary`.
    pub fn show(&self, host: &NSView, summary: ShieldsSummary, trigger: Trigger) {
        *self.summary.borrow_mut() = Some(summary);
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

    /// Shows `summary` in place of what is on screen.
    pub fn update(&self, summary: ShieldsSummary) {
        if !self.is_open() {
            return;
        }
        *self.summary.borrow_mut() = Some(summary);
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

    /// Whether `⏎` reloads now.
    #[must_use]
    pub fn reload_offered(&self) -> bool {
        self.summary
            .borrow()
            .as_ref()
            .is_some_and(|summary| summary.reload_offered)
    }

    /// Flips a switch as a click does.
    pub fn flip(&self, which: SiteToggle) {
        if let Some((_, row)) = self.toggles.iter().find(|(toggle, _)| *toggle == which) {
            row.toggle();
        }
    }

    /// Whether a switch shows on.
    #[must_use]
    pub fn is_on(&self, which: SiteToggle) -> bool {
        self.toggles
            .iter()
            .any(|(toggle, row)| *toggle == which && row.is_on())
    }

    /// The popover's surface, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// Sizes the surface to its content and places everything, top down.
    #[expect(
        clippy::too_many_lines,
        reason = "one pass down the popover, part by part, reads better than split helpers"
    )]
    fn lay_out(&self, host: &NSView) {
        let Some(summary) = self.summary.borrow().clone() else {
            return;
        };
        let mtm = self.mtm;
        let padding = space::PALETTE_PADDING;
        let width = overlay::POPOVER_WIDTH.min(host.bounds().size.width - space::XL * 2.0);
        let inner = width - padding * 2.0;

        let (symbol, tint, status) = status(summary.policy, !summary.lists.is_empty());
        self.glyph
            .setImage(image(symbol, status, TextRole::Title).as_deref());
        self.glyph.setContentTintColor(Some(&tint));
        set(&self.site, summary.site.as_deref().unwrap_or("This page"));
        set(&self.status, status);
        self.status.setTextColor(Some(&tint));
        let blocked_tint = if summary.blocked.is_some_and(|count| count > 0) {
            color::success()
        } else {
            color::label()
        };
        self.tiles[0].set(summary.blocked, "Blocked on this page", &blocked_tint);
        self.tiles[1].set(
            u32::try_from(summary.lists.len()).ok(),
            "Filter lists on",
            &color::label(),
        );
        for (toggle, row) in &self.toggles {
            let on = summary.policy.is_some_and(|policy| match toggle {
                SiteToggle::Shields => policy.shields,
                SiteToggle::HttpsOnly => policy.https_only,
                SiteToggle::JavaScript => policy.javascript,
            });
            row.set_on(on);
            row.set_enabled(summary.policy.is_some());
        }
        let rows: Vec<FilterListRow> = summary
            .lists
            .iter()
            .map(|&(list, at)| FilterListRow::new(list, at, summary.now, inner, mtm))
            .collect();
        for old in self.list_rows.borrow().iter() {
            old.view().removeFromSuperview();
        }
        let hints: &[(&str, &str)] = if summary.reload_offered {
            &[("⏎", "reload to apply"), ("⎋", "close")]
        } else {
            &[("⎋", "close")]
        };
        let fresh = hint_bar(hints, mtm);
        self.hints.borrow().removeFromSuperview();
        self.surface.content_view().addSubview(&fresh);
        *self.hints.borrow_mut() = fresh;

        let header = 36.0;
        let lists_height = if rows.is_empty() {
            ROW_HEIGHT
        } else {
            ROW_HEIGHT * as_float(rows.len())
        };
        let reload = if summary.reload_offered {
            self.reload.frame().size.height + space::MD
        } else {
            0.0
        };
        set(&self.rules, &summary.rules);
        self.rules.setHidden(summary.rules.is_empty());
        let rules_height = if summary.rules.is_empty() {
            0.0
        } else {
            self.rules.frame().size.height + space::SM
        };
        let hint_height = self.hints.borrow().frame().size.height;
        let caption_height = self.protections.frame().size.height;
        let height = padding
            + header
            + space::MD
            + TILE_HEIGHT
            + space::MD
            + 1.0
            + space::MD
            + caption_height
            + space::XS
            + ToggleRow::HEIGHT * 3.0
            + space::SM
            + 1.0
            + space::MD
            + ROW_HEIGHT
            + lists_height
            + rules_height
            + space::MD
            + reload
            + hint_height
            + padding;
        self.surface
            .place_top_right(host, NSSize::new(width, height));

        let mut top = height - padding;
        let glyph = 28.0;
        self.glyph.setFrame(NSRect::new(
            NSPoint::new(padding, top - header + (header - glyph) / 2.0),
            NSSize::new(glyph, glyph),
        ));
        let left = padding + glyph + space::SM;
        let site_height = self.site.frame().size.height;
        let status_height = self.status.frame().size.height;
        self.site.setFrame(NSRect::new(
            NSPoint::new(left, top - site_height),
            NSSize::new(inner - glyph - space::SM, site_height),
        ));
        self.status.setFrame(NSRect::new(
            NSPoint::new(left, top - site_height - status_height),
            NSSize::new(inner - glyph - space::SM, status_height),
        ));
        top -= header + space::MD;
        let tile_width = ((inner - space::SM) / 2.0).floor();
        for (index, tile) in self.tiles.iter().enumerate() {
            let x = padding + as_float(index) * (tile_width + space::SM);
            tile.place(NSPoint::new(x, top - TILE_HEIGHT), tile_width);
        }
        top -= TILE_HEIGHT + space::MD;
        self.separators[0].setFrame(NSRect::new(
            NSPoint::new(padding, top - 1.0),
            NSSize::new(inner, 1.0),
        ));
        top -= 1.0 + space::MD;
        self.protections.setFrame(NSRect::new(
            NSPoint::new(padding, top - caption_height),
            NSSize::new(inner, caption_height),
        ));
        top -= caption_height + space::XS;
        for (index, (_, row)) in self.toggles.iter().enumerate() {
            top -= ToggleRow::HEIGHT;
            row.place(
                NSPoint::new(padding + ICON_COLUMN, top),
                inner - ICON_COLUMN,
            );
            if let Some(icon) = self.icons.get(index) {
                let size = icon.frame().size;
                icon.setFrameOrigin(NSPoint::new(
                    padding,
                    top + ((ToggleRow::HEIGHT - size.height) / 2.0).round(),
                ));
            }
        }
        top -= space::SM;
        self.separators[1].setFrame(NSRect::new(
            NSPoint::new(padding, top - 1.0),
            NSSize::new(inner, 1.0),
        ));
        top -= 1.0 + space::MD;
        let caption = self.lists_title.frame().size.height;
        self.lists_title.setFrame(NSRect::new(
            NSPoint::new(padding, top - ROW_HEIGHT + (ROW_HEIGHT - caption) / 2.0),
            NSSize::new(inner / 2.0, caption),
        ));
        let action = if rows.is_empty() {
            &self.enable
        } else {
            &self.refresh
        };
        self.enable.setHidden(!rows.is_empty());
        self.refresh.setHidden(rows.is_empty());
        let action_size = action.frame().size;
        action.setFrameOrigin(NSPoint::new(
            width - padding - action_size.width,
            top - ROW_HEIGHT + ((ROW_HEIGHT - action_size.height) / 2.0).round(),
        ));
        top -= ROW_HEIGHT;
        let no_lists = self.no_lists.frame().size.height;
        self.no_lists.setFrame(NSRect::new(
            NSPoint::new(padding, top - ROW_HEIGHT + (ROW_HEIGHT - no_lists) / 2.0),
            NSSize::new(inner, no_lists),
        ));
        self.no_lists.setHidden(!rows.is_empty());
        for row in &rows {
            top -= ROW_HEIGHT;
            row.view().setFrameOrigin(NSPoint::new(padding, top));
            self.surface.content_view().addSubview(row.view());
        }
        *self.list_rows.borrow_mut() = rows;
        if !summary.rules.is_empty() {
            if self.list_rows.borrow().is_empty() {
                top -= ROW_HEIGHT;
            }
            let rules = self.rules.frame().size.height;
            self.rules.setFrame(NSRect::new(
                NSPoint::new(padding, top - space::SM - rules),
                NSSize::new(inner, rules),
            ));
        }
        self.reload.setHidden(!summary.reload_offered);
        if summary.reload_offered {
            let reload_size = self.reload.frame().size;
            self.reload.setFrameOrigin(NSPoint::new(
                width - padding - reload_size.width,
                padding + hint_height + space::MD,
            ));
        }
        self.hints.borrow().setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(inner, hint_height),
        ));
    }
}

/// The header's symbol, its tint, and what it says, for a site's policy (`None` off the web)
/// and whether any filter list is on: shields up with no list block nothing, and say so.
fn status(
    policy: Option<SitePolicy>,
    lists_on: bool,
) -> (&'static str, Retained<objc2_app_kit::NSColor>, &'static str) {
    match policy {
        Some(policy) if policy.shields && !lists_on => (
            "shield",
            color::secondary_label(),
            "Shields are up, but no filter list is on",
        ),
        Some(policy) if policy.shields => {
            ("checkmark.shield.fill", color::success(), "Shields are up")
        }
        Some(_) => (
            "shield.slash",
            color::secondary_label(),
            "Shields are down on this site",
        ),
        None => (
            "shield",
            color::secondary_label(),
            "Shields apply to web pages only",
        ),
    }
}

fn as_float(count: usize) -> CGFloat {
    CGFloat::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn set(field: &NSTextField, value: &str) {
    field.setStringValue(&NSString::from_str(value));
}

fn text(
    role: TextRole,
    tint: &objc2_app_kit::NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(" "), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

fn separator(mtm: MainThreadMarker) -> Retained<NSBox> {
    let line = NSBox::new(mtm);
    line.setBoxType(NSBoxType::Separator);
    line
}

/// A borderless accent text button sending `action` to `target`.
fn button(
    title: &str,
    action: objc2::runtime::Sel,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setTitle(&NSString::from_str(title));
    button.setBordered(false);
    button.setFont(Some(&TextRole::RowDetail.font()));
    button.setContentTintColor(Some(&color::accent()));
    // SAFETY: the target is weak; the popover keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.setRefusesFirstResponder(true);
    button.sizeToFit();
    button
}

/// The SF Symbol `name` at the size of `role`.
fn image(name: &str, description: &str, role: TextRole) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(description)),
    )?;
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(role.point_size(), 0.0);
    image.imageWithSymbolConfiguration(&configuration)
}

#[cfg(test)]
mod tests {
    use quickie_domain_shields::SitePolicy;

    use super::status;

    #[test]
    fn the_header_says_where_the_shields_stand() {
        let down = SitePolicy {
            shields: false,
            ..SitePolicy::DEFAULT
        };

        assert_eq!(status(Some(SitePolicy::DEFAULT), true).2, "Shields are up");
        assert_eq!(
            status(Some(SitePolicy::DEFAULT), false).2,
            "Shields are up, but no filter list is on"
        );
        assert_eq!(status(Some(down), true).2, "Shields are down on this site");
        assert_eq!(status(None, true).2, "Shields apply to web pages only");
        assert_eq!(status(Some(down), false).0, "shield.slash");
    }
}

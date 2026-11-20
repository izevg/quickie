//! Privacy & Shields: a card that says whether protection is on and where the per-site
//! switches live (on the page, not here); the filter lists, each a row with when it last
//! arrived, a button that fetches it now, and its switch (`docs/design/inventory.md`,
//! `filter_list_row`); and whether stale lists are fetched on their own.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSBezelStyle, NSButton, NSImage, NSImageView, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSInteger, NSPoint, NSRect, NSSize, NSString};
use quickie_domain_shields::FilterListId;
use quickie_ui_shared::format::ago;
use quickie_ui_shared::tokens::{TextRole, color, radius, space};
use quickie_ui_shared::{FormGroup, Plate, ToggleRow};

use crate::settings::SettingsAction;
use crate::settings::chrome::{
    Column, INNER, InfoRow, ScrollPage, SettingsTarget, button, heading, label, paragraph, set,
    wrapped_height,
};

/// One filter list as the pane shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListState {
    pub id: FilterListId,
    pub enabled: bool,
    /// How long ago it last arrived, in milliseconds; `None` before its first fetch.
    pub fetched_ago_ms: Option<u64>,
}

/// What the Shields pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShieldsState {
    /// Every allowed list, in [`FilterListId::ALL`] order.
    pub lists: Vec<ListState>,
    /// Stale lists are fetched on their own.
    pub keep_fresh: bool,
    /// The keys of the shields popover, as keycaps: `⌥⌘S`.
    pub site_keys: String,
    /// How the last fetch went, or what is going on now; empty for nothing to say.
    pub status: String,
}

impl ShieldsState {
    /// Protection is on when a list is on and has arrived: only then does anything block.
    fn protected(&self) -> bool {
        self.lists
            .iter()
            .any(|list| list.enabled && list.fetched_ago_ms.is_some())
    }
}

/// The card on top: a symbol, a title, and a line.
struct Card {
    plate: Retained<Plate>,
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    detail: Retained<NSTextField>,
}

impl Card {
    const ICON: CGFloat = 28.0;

    fn new(mtm: MainThreadMarker) -> Self {
        let plate = Plate::new(None, None, radius::ROW, mtm);
        let icon = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(Self::ICON, Self::ICON)),
        );
        let title = label("", TextRole::Headline, &color::label(), mtm);
        let detail = paragraph("", mtm);
        detail.setTextColor(Some(&color::secondary_label()));
        for view in [&*icon as &NSView, &title, &detail] {
            plate.addSubview(view);
        }
        Self {
            plate,
            icon,
            title,
            detail,
        }
    }

    fn update(&self, state: &ShieldsState) {
        let (symbol_name, tint, title) = if state.protected() {
            (
                "checkmark.shield.fill",
                color::success(),
                "Protection is on",
            )
        } else {
            (
                "shield.slash",
                color::secondary_label(),
                "No filter list is on yet",
            )
        };
        if state.protected() {
            self.plate
                .set_paint(Some(color::success_fill), Some(color::success_line));
        } else {
            self.plate
                .set_paint(Some(color::group_fill), Some(color::separator));
        }
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol_name),
            Some(&NSString::from_str(title)),
        );
        self.icon.setImage(image.as_deref());
        self.icon.setSymbolConfiguration(Some(
            &objc2_app_kit::NSImageSymbolConfiguration::configurationWithPointSize_weight(
                Self::ICON - space::XS,
                0.0,
            ),
        ));
        self.icon.setContentTintColor(Some(&tint));
        set(&self.title, title);
        self.title.sizeToFit();
        set(
            &self.detail,
            &format!(
                "Shields are set per site, on the page: {} shows the site's switches for \
                 blocking, HTTPS-only, and JavaScript.",
                state.site_keys
            ),
        );
    }

    /// Places the card at `origin`, [`INNER`] wide; returns its height.
    fn place(&self, origin: NSPoint) -> CGFloat {
        let left = space::LG + Self::ICON + space::MD;
        let text_width = INNER - left - space::LG;
        let title_height = self.title.frame().size.height;
        let detail_height = wrapped_height(&self.detail, text_width);
        let height = (space::LG * 2.0 + title_height + space::XS + detail_height)
            .max(Self::ICON + space::LG * 2.0);
        self.plate
            .setFrame(NSRect::new(origin, NSSize::new(INNER, height)));
        self.icon.setFrameOrigin(NSPoint::new(
            space::LG,
            ((height - Self::ICON) / 2.0).round(),
        ));
        let top = ((height - title_height - space::XS - detail_height) / 2.0).round();
        self.title.setFrame(NSRect::new(
            NSPoint::new(left, top),
            NSSize::new(text_width, title_height),
        ));
        self.detail.setFrame(NSRect::new(
            NSPoint::new(left, top + title_height + space::XS),
            NSSize::new(text_width, detail_height),
        ));
        height
    }
}

pub struct Shields {
    page: ScrollPage,
    card: Card,
    lists_heading: Retained<NSTextField>,
    lists_group: FormGroup,
    lists: Vec<(FilterListId, ToggleRow, Retained<NSButton>)>,
    updates_heading: Retained<NSTextField>,
    updates_group: FormGroup,
    keep_fresh: ToggleRow,
    refresh: InfoRow,
    status: Retained<NSTextField>,
}

impl Shields {
    pub fn new(target: &Retained<SettingsTarget>, mtm: MainThreadMarker) -> Self {
        let lists_group = FormGroup::new(mtm);
        let lists = FilterListId::ALL
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                let row = ToggleRow::new(id.title(), "", mtm);
                let list_target = target.clone();
                row.on_change(move |on| list_target.ask(SettingsAction::SetList(id, on)));
                let refresh = refresh_button(id, index, target, mtm);
                row.set_accessory(&refresh);
                lists_group.view().addSubview(row.view());
                (id, row, refresh)
            })
            .collect();
        let keep_fresh = ToggleRow::new(
            "Keep lists fresh",
            "Fetch an enabled list again once it is four days old",
            mtm,
        );
        let fresh_target = target.clone();
        keep_fresh.on_change(move |on| fresh_target.ask(SettingsAction::KeepListsFresh(on)));
        let refresh = InfoRow::new(
            None,
            Some(button("Refresh Now", sel!(refreshLists:), target, mtm)),
            mtm,
        );
        set(refresh.text(), "Fetch every enabled list now");
        let updates_group = FormGroup::new(mtm);
        updates_group.view().addSubview(keep_fresh.view());
        refresh.add_to(updates_group.view());
        let status = paragraph("", mtm);
        status.setFont(Some(&TextRole::RowDetail.font()));
        status.setTextColor(Some(&color::secondary_label()));
        let shields = Self {
            page: ScrollPage::new("Privacy & Shields", mtm),
            card: Card::new(mtm),
            lists_heading: heading("Filter lists", mtm),
            lists_group,
            lists,
            updates_heading: heading("Updates", mtm),
            updates_group,
            keep_fresh,
            refresh,
            status,
        };
        let column = shields.page.column();
        for view in [
            &*shields.card.plate as &NSView,
            &shields.lists_heading,
            shields.lists_group.view(),
            &shields.updates_heading,
            shields.updates_group.view(),
            &shields.status,
        ] {
            column.addSubview(view);
        }
        shields
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    /// Shows `state` and lays the pane out.
    pub fn update(&self, state: &ShieldsState) {
        self.card.update(state);
        for (id, row, refresh) in &self.lists {
            let list = state.lists.iter().find(|list| list.id == *id);
            let enabled = list.is_some_and(|list| list.enabled);
            row.set_on(enabled);
            refresh.setEnabled(enabled);
            let detail = match list.and_then(|list| list.fetched_ago_ms) {
                Some(elapsed) => format!("Updated {}", ago(elapsed)),
                None => "Not fetched yet".to_owned(),
            };
            row.set_detail(&detail);
        }
        self.keep_fresh.set_on(state.keep_fresh);
        set(&self.status, &state.status);

        let mut column = Column::new();
        let card = self.card.place(column.origin());
        column.gap(card + space::SECTION);
        column.heading(&self.lists_heading);
        let heights = vec![ToggleRow::HEIGHT; self.lists.len()];
        let (frames, height) = self.lists_group.place(column.origin(), INNER, &heights);
        for ((_, row, _), frame) in self.lists.iter().zip(&frames) {
            row.place(frame.origin, frame.size.width);
        }
        column.gap(height + space::SECTION);
        column.heading(&self.updates_heading);
        let (frames, height) = self.updates_group.place(
            column.origin(),
            INNER,
            &[ToggleRow::HEIGHT, InfoRow::MIN_HEIGHT],
        );
        self.keep_fresh
            .place(frames[0].origin, frames[0].size.width);
        self.refresh.place(frames[1]);
        column.gap(height + space::SM);
        let status_height = if state.status.is_empty() {
            0.0
        } else {
            wrapped_height(&self.status, INNER)
        };
        column.full(&self.status, status_height);
        self.page.set_content_height(column.top);
    }
}

/// A list's refresh button: a borderless `arrow.clockwise` that fetches that list now.
fn refresh_button(
    list: FilterListId,
    index: usize,
    target: &SettingsTarget,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let description = format!("Refresh {} now", list.title());
    let button = NSButton::new(mtm);
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setBordered(false);
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("arrow.clockwise"),
        Some(&NSString::from_str(&description)),
    );
    button.setImage(image.as_deref());
    button.setToolTip(Some(&NSString::from_str(&description)));
    button.setTag(NSInteger::try_from(index).unwrap_or(NSInteger::MAX));
    let target: &AnyObject = target.as_ref();
    // SAFETY: the target is weak; the screen keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `refreshList:`, which takes the sender.
    unsafe { button.setAction(Some(sel!(refreshList:))) };
    button.sizeToFit();
    button
}

//! Shields: where the per-site switches live (on the page, not here), the filter lists with
//! when each was last fetched, the button that fetches them now, and whether stale lists are
//! fetched on their own.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSButton, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::NSPoint;
use quickie_domain_shields::FilterListId;
use quickie_ui_shared::ToggleRow;
use quickie_ui_shared::format::ago;
use quickie_ui_shared::tokens::space;

use super::{
    Column, PaneView, SettingsAction, SettingsTarget, button, heading, paragraph, set,
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

pub struct Shields {
    view: Retained<PaneView>,
    site_heading: Retained<NSTextField>,
    site: Retained<NSTextField>,
    lists_heading: Retained<NSTextField>,
    lists: Vec<(FilterListId, ToggleRow)>,
    refresh: Retained<NSButton>,
    status: Retained<NSTextField>,
    keep_fresh: ToggleRow,
}

impl Shields {
    pub fn new(target: &Retained<SettingsTarget>, mtm: MainThreadMarker) -> Self {
        let lists = FilterListId::ALL
            .into_iter()
            .map(|id| {
                let row = ToggleRow::new(id.title(), "", mtm);
                let target = target.clone();
                row.on_change(move |on| target.ask(SettingsAction::SetList(id, on)));
                (id, row)
            })
            .collect();
        let keep_fresh = ToggleRow::new(
            "Keep lists fresh",
            "Fetch an enabled list again once it is four days old",
            mtm,
        );
        let fresh_target = target.clone();
        keep_fresh.on_change(move |on| fresh_target.ask(SettingsAction::KeepListsFresh(on)));
        let shields = Self {
            view: PaneView::new(mtm),
            site_heading: heading("This site", mtm),
            site: paragraph("", mtm),
            lists_heading: heading("Filter lists", mtm),
            lists,
            refresh: button("Refresh Lists Now", sel!(refreshLists:), target, mtm),
            status: paragraph("", mtm),
            keep_fresh,
        };
        for view in [
            &*shields.site_heading as &NSView,
            &shields.site,
            &shields.lists_heading,
            &shields.refresh,
            &shields.status,
            shields.keep_fresh.view(),
        ] {
            shields.view.addSubview(view);
        }
        for (_, row) in &shields.lists {
            shields.view.addSubview(row.view());
        }
        shields
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Shows `state` and lays the pane out; returns its height.
    pub fn update(&self, state: &ShieldsState) -> CGFloat {
        set(
            &self.site,
            &format!(
                "Shields are set per site, on the page: {} shows the site's switches for \
                 blocking, HTTPS-only, and JavaScript.",
                state.site_keys
            ),
        );
        for (id, row) in &self.lists {
            let list = state.lists.iter().find(|list| list.id == *id);
            row.set_on(list.is_some_and(|list| list.enabled));
            let detail = match list.and_then(|list| list.fetched_ago_ms) {
                Some(elapsed) => format!("Fetched {}", ago(elapsed)),
                None => "Not fetched yet".to_owned(),
            };
            row.set_detail(&detail);
        }
        self.keep_fresh.set_on(state.keep_fresh);
        set(&self.status, &state.status);

        let mut column = Column::new();
        column.full(&self.site_heading, self.site_heading.frame().size.height);
        column.gap(space::XS);
        column.full(&self.site, wrapped_height(&self.site));
        column.gap(space::SECTION);
        column.full(&self.lists_heading, self.lists_heading.frame().size.height);
        column.gap(space::XS);
        for (_, row) in &self.lists {
            row.place(NSPoint::new(space::XL, column.top), Column::INNER);
            column.gap(ToggleRow::HEIGHT);
        }
        column.gap(space::SM);
        column.buttons(&[&self.refresh]);
        column.gap(space::XS);
        column.full(&self.status, wrapped_height(&self.status));
        column.gap(space::LG);
        self.keep_fresh
            .place(NSPoint::new(space::XL, column.top), Column::INNER);
        column.gap(ToggleRow::HEIGHT);
        column.height()
    }
}

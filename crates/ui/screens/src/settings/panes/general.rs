//! General: whether Quickie is the default browser, with the button that asks macOS to make it
//! so; where downloads go, with the button that shows the folder; and the appearance, whose
//! switch puts the page inside the window as a card (*Inset page*). Each is a section with one
//! row.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message, sel};
use objc2_app_kit::{NSLineBreakMode, NSTextField, NSView};
use quickie_ui_shared::tokens::{TextRole, space};
use quickie_ui_shared::{FormGroup, ToggleRow};

use crate::settings::SettingsAction;
use crate::settings::chrome::{
    Column, INNER, InfoRow, ScrollPage, SettingsTarget, button, heading, set,
};

/// What the General pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneralState {
    /// Who opens web links now, in a sentence: "Quickie is your default browser."
    pub default_browser: String,
    /// Whether the button can ask macOS: not when Quickie already is the default, nor from a
    /// bare binary, which macOS cannot hand links to.
    pub can_make_default: bool,
    /// The downloads folder's path.
    pub downloads_folder: String,
    /// *Inset page* is on.
    pub inset_page: bool,
}

pub struct General {
    page: ScrollPage,
    browser_heading: Retained<NSTextField>,
    browser_group: FormGroup,
    browser: InfoRow,
    downloads_heading: Retained<NSTextField>,
    downloads_group: FormGroup,
    downloads: InfoRow,
    appearance_heading: Retained<NSTextField>,
    appearance_group: FormGroup,
    inset_page: ToggleRow,
}

impl General {
    pub fn new(target: &SettingsTarget, mtm: MainThreadMarker) -> Self {
        let make_default = button("Make Default", sel!(makeDefault:), target, mtm);
        let show = button("Show in Finder", sel!(showDownloads:), target, mtm);
        let inset_page = ToggleRow::new(
            "Inset page",
            "Show the page as a card inside the window, framed by the window's own edge",
            mtm,
        );
        let inset_target = target.retain();
        inset_page.on_change(move |on| inset_target.ask(SettingsAction::InsetPage(on)));
        let general = Self {
            page: ScrollPage::new("General", mtm),
            browser_heading: heading("Default browser", mtm),
            browser_group: FormGroup::new(mtm),
            browser: InfoRow::new(Some("globe"), Some(make_default), mtm),
            downloads_heading: heading("Downloads", mtm),
            downloads_group: FormGroup::new(mtm),
            downloads: InfoRow::new(Some("folder"), Some(show), mtm),
            appearance_heading: heading("Appearance", mtm),
            appearance_group: FormGroup::new(mtm),
            inset_page,
        };
        let path = general.downloads.text();
        path.setFont(Some(&TextRole::RowTitle.mono_font()));
        path.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        path.setMaximumNumberOfLines(1);
        general.browser.add_to(general.browser_group.view());
        general.downloads.add_to(general.downloads_group.view());
        general
            .appearance_group
            .view()
            .addSubview(general.inset_page.view());
        let column = general.page.column();
        for view in [
            &*general.browser_heading as &NSView,
            general.browser_group.view(),
            &general.downloads_heading,
            general.downloads_group.view(),
            &general.appearance_heading,
            general.appearance_group.view(),
        ] {
            column.addSubview(view);
        }
        general
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    /// Shows `state` and lays the pane out.
    pub fn update(&self, state: &GeneralState) {
        set(self.browser.text(), &state.default_browser);
        if let Some(make_default) = self.browser.button() {
            make_default.setEnabled(state.can_make_default);
        }
        set(self.downloads.text(), &state.downloads_folder);
        self.downloads
            .text()
            .setToolTip(Some(&objc2_foundation::NSString::from_str(
                &state.downloads_folder,
            )));
        self.inset_page.set_on(state.inset_page);

        let mut column = Column::new();
        column.heading(&self.browser_heading);
        let rows = [self.browser.height(INNER - FormGroup::INSET * 2.0)];
        let (frames, height) = self.browser_group.place(column.origin(), INNER, &rows);
        self.browser.place(frames[0]);
        column.gap(height + space::SECTION);
        column.heading(&self.downloads_heading);
        let (frames, height) =
            self.downloads_group
                .place(column.origin(), INNER, &[InfoRow::MIN_HEIGHT]);
        self.downloads.place(frames[0]);
        column.gap(height + space::SECTION);
        column.heading(&self.appearance_heading);
        let (frames, height) =
            self.appearance_group
                .place(column.origin(), INNER, &[ToggleRow::HEIGHT]);
        self.inset_page
            .place(frames[0].origin, frames[0].size.width);
        column.gap(height);
        self.page.set_content_height(column.top);
    }
}

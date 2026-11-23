//! General: whether Quickie is the default browser, with the button that asks macOS to make it
//! so; where downloads go, with the button that shows the folder; and the appearance, whose
//! switch puts the page inside the window as a card (*Inset page*). Released builds also show
//! software updates.

use std::cell::Cell;

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

/// Software Updates rows for a supported release installation.
pub struct SoftwareUpdates<'a> {
    pub version: &'a str,
    pub automatic: bool,
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
    updates: Updates,
}

struct Updates {
    visible: Cell<bool>,
    heading: Retained<NSTextField>,
    group: FormGroup,
    release: InfoRow,
    automatic: ToggleRow,
    notes: InfoRow,
}

impl Updates {
    fn new(target: &SettingsTarget, mtm: MainThreadMarker) -> Self {
        let release = InfoRow::new(
            Some("arrow.down.circle"),
            Some(button(
                "Check for Updates…",
                sel!(checkAppUpdates:),
                target,
                mtm,
            )),
            mtm,
        );
        let notes = InfoRow::new(
            Some("doc.text"),
            Some(button("View", sel!(whatsNew:), target, mtm)),
            mtm,
        );
        set(notes.text(), "What’s New in Quickie");
        let automatic = ToggleRow::new(
            "Automatic updates",
            "Download new releases and install them when Quickie quits",
            mtm,
        );
        let toggle_target = target.retain();
        automatic.on_change(move |on| toggle_target.ask(SettingsAction::AutomaticAppUpdates(on)));
        let group = FormGroup::new(mtm);
        release.add_to(group.view());
        group.view().addSubview(automatic.view());
        notes.add_to(group.view());
        let heading = heading("Software Updates", mtm);
        heading.setHidden(true);
        group.view().setHidden(true);
        Self {
            visible: Cell::new(false),
            heading,
            group,
            release,
            automatic,
            notes,
        }
    }

    fn update(&self, state: Option<&SoftwareUpdates<'_>>) {
        self.visible.set(state.is_some());
        self.heading.setHidden(state.is_none());
        self.group.view().setHidden(state.is_none());
        if let Some(state) = state {
            set(self.release.text(), &format!("Quickie {}", state.version));
            self.automatic.set_on(state.automatic);
        }
    }

    fn place(&self, column: &mut Column) {
        if !self.visible.get() {
            return;
        }
        column.gap(space::SECTION);
        column.heading(&self.heading);
        let rows = [InfoRow::MIN_HEIGHT, ToggleRow::HEIGHT, InfoRow::MIN_HEIGHT];
        let (frames, height) = self.group.place(column.origin(), INNER, &rows);
        self.release.place(frames[0]);
        self.automatic.place(frames[1].origin, frames[1].size.width);
        self.notes.place(frames[2]);
        column.gap(height);
    }
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
            updates: Updates::new(target, mtm),
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
            &general.updates.heading,
            general.updates.group.view(),
        ] {
            column.addSubview(view);
        }
        general
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    pub fn set_software_updates(&self, state: Option<&SoftwareUpdates<'_>>) {
        self.updates.update(state);
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
        self.updates.place(&mut column);
        self.page.set_content_height(column.top);
    }
}

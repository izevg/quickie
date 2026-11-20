//! General: whether Quickie is the default browser, with the button that asks macOS to make it
//! so, and where downloads go, with the button that shows the folder.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSButton, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use quickie_ui_shared::tokens::{TextRole, color, space};

use super::{
    Column, PaneView, SettingsTarget, button, heading, label, paragraph, set, wrapped_height,
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
}

pub struct General {
    view: Retained<PaneView>,
    browser_heading: Retained<NSTextField>,
    browser: Retained<NSTextField>,
    make_default: Retained<NSButton>,
    downloads_heading: Retained<NSTextField>,
    downloads: Retained<NSTextField>,
    show_downloads: Retained<NSButton>,
}

impl General {
    pub fn new(target: &SettingsTarget, mtm: MainThreadMarker) -> Self {
        let general = Self {
            view: PaneView::new(mtm),
            browser_heading: heading("Default browser", mtm),
            browser: paragraph("", mtm),
            make_default: button(
                "Make Quickie the Default Browser",
                sel!(makeDefault:),
                target,
                mtm,
            ),
            downloads_heading: heading("Downloads", mtm),
            downloads: label("", TextRole::RowTitle, &color::label(), mtm),
            show_downloads: button("Show in Finder", sel!(showDownloads:), target, mtm),
        };
        general
            .downloads
            .setFont(Some(&TextRole::RowTitle.mono_font()));
        for view in [
            &*general.browser_heading as &NSView,
            &general.browser,
            &general.make_default,
            &general.downloads_heading,
            &general.downloads,
            &general.show_downloads,
        ] {
            general.view.addSubview(view);
        }
        general
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Shows `state` and lays the pane out; returns its height.
    pub fn update(&self, state: &GeneralState) -> CGFloat {
        set(&self.browser, &state.default_browser);
        self.make_default.setEnabled(state.can_make_default);
        set(&self.downloads, &state.downloads_folder);
        let mut column = Column::new();
        column.full(
            &self.browser_heading,
            self.browser_heading.frame().size.height,
        );
        column.gap(space::XS);
        column.full(&self.browser, wrapped_height(&self.browser));
        column.gap(space::SM);
        column.buttons(&[&self.make_default]);
        column.gap(space::SECTION);
        column.full(
            &self.downloads_heading,
            self.downloads_heading.frame().size.height,
        );
        column.gap(space::XS);
        let line = (TextRole::RowTitle.point_size() * 1.4).ceil();
        column.full(&self.downloads, line);
        column.gap(space::SM);
        column.buttons(&[&self.show_downloads]);
        column.height()
    }
}

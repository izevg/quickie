//! Extensions: the web extensions of the main profile, each a row with its switch, what it
//! may do, where the user turned it off or on, and a button that removes it; then how to add
//! one (a package or folder from disk, or a Chrome Web Store page pasted into the palette) and
//! how updates come (`docs/design/inventory.md`, Settings).
//!
//! The pane only shows; every switch and button asks the browser through the window's
//! [`SettingsAction`] handler, and [`Extensions::update`] shows what the browser answers.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBezelStyle, NSButton, NSImage, NSTextField, NSView};
use objc2_foundation::{NSInteger, NSString};
use quickie_domain_settings::ExtensionId;
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{FormGroup, ToggleRow};

use crate::settings::SettingsAction;
use crate::settings::chrome::{
    Column, INNER, InfoRow, ScrollPage, SettingsTarget, button, heading, paragraph, set,
    wrapped_height,
};

/// One installed extension as the pane shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionState {
    pub id: ExtensionId,
    pub name: String,
    /// Version and where it came from: `2025.1.0 · Chrome Web Store`.
    pub about: String,
    pub enabled: bool,
    /// What it may do, in plain words, broadest first.
    pub access: Vec<String>,
    /// The sites the user turned it off on, and on.
    pub off_on: Vec<String>,
    pub on_on: Vec<String>,
}

impl ExtensionState {
    /// The row's second line: the sites the user decided come before what it may do, which is
    /// the longest part and the one cut first.
    fn detail(&self) -> String {
        let mut parts = vec![self.about.clone()];
        if !self.off_on.is_empty() {
            parts.push(format!("off on {}", self.off_on.join(", ")));
        }
        if !self.on_on.is_empty() {
            parts.push(format!("on on {}", self.on_on.join(", ")));
        }
        match self.access.first() {
            Some(first) if self.access.len() > 1 => {
                parts.push(format!("{first} and {} more", self.access.len() - 1));
            }
            Some(first) => parts.push(first.clone()),
            None => {}
        }
        parts.join(" · ")
    }
}

/// What the Extensions pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionsState {
    /// Whether this macOS runs web extensions (15.4 and later).
    pub available: bool,
    pub extensions: Vec<ExtensionState>,
    /// Updates come once a day on their own.
    pub auto_update: bool,
    /// The keys of the palette, as keycaps: `⌘P`.
    pub palette_keys: String,
    /// How the last install or update went, or what is going on now; empty for nothing.
    pub status: String,
}

type Ids = RefCell<Vec<ExtensionId>>;

pub struct Ivars {
    ids: Ids,
    settings: Retained<SettingsTarget>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ExtensionsTarget` implements
    // no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsExtensionsTarget"]
    #[ivars = Ivars]
    pub struct ExtensionsTarget;

    unsafe impl NSObjectProtocol for ExtensionsTarget {}

    impl ExtensionsTarget {
        /// A row's remove button: its tag is the row's place in the list.
        #[unsafe(method(remove:))]
        fn remove(&self, sender: Option<&NSButton>) {
            let id = sender
                .and_then(|button| usize::try_from(button.tag()).ok())
                .and_then(|index| self.ivars().ids.borrow().get(index).cloned());
            if let Some(id) = id {
                self.ivars().settings.ask(SettingsAction::RemoveExtension(id));
            }
        }

        #[unsafe(method(choose:))]
        fn choose(&self, _sender: Option<&NSButton>) {
            self.ivars().settings.ask(SettingsAction::ChooseExtensionFile);
        }

        #[unsafe(method(checkUpdates:))]
        fn check_updates(&self, _sender: Option<&NSButton>) {
            self.ivars().settings.ask(SettingsAction::CheckExtensionUpdates);
        }
    }
);

impl ExtensionsTarget {
    fn new(settings: &Retained<SettingsTarget>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            ids: RefCell::new(Vec::new()),
            settings: settings.clone(),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Extensions {
    page: ScrollPage,
    intro: Retained<NSTextField>,
    installed_heading: Retained<NSTextField>,
    installed_group: FormGroup,
    rows: RefCell<Vec<ToggleRow>>,
    empty: InfoRow,
    add_heading: Retained<NSTextField>,
    add_group: FormGroup,
    choose: InfoRow,
    paste: InfoRow,
    auto_update: ToggleRow,
    check: InfoRow,
    status: Retained<NSTextField>,
    target: Retained<ExtensionsTarget>,
    settings: Retained<SettingsTarget>,
    mtm: MainThreadMarker,
}

impl Extensions {
    pub fn new(settings: &Retained<SettingsTarget>, mtm: MainThreadMarker) -> Self {
        let target = ExtensionsTarget::new(settings, mtm);
        let as_settings = |title: &str, action| {
            let made = button(title, action, settings, mtm);
            let target: &AnyObject = target.as_ref();
            // SAFETY: the target is weak; the pane keeps it alive for as long as its buttons.
            unsafe { made.setTarget(Some(target)) };
            made
        };
        let choose = InfoRow::new(
            Some("shippingbox"),
            Some(as_settings("Choose…", sel!(choose:))),
            mtm,
        );
        set(
            choose.text(),
            "Install a .crx or .zip package, or an unpacked folder",
        );
        let paste = InfoRow::new(Some("doc.on.clipboard"), None, mtm);
        let auto_update = ToggleRow::new(
            "Update automatically",
            "Look for updates from the Chrome Web Store once a day",
            mtm,
        );
        let fresh = settings.clone();
        auto_update.on_change(move |on| fresh.ask(SettingsAction::AutoUpdateExtensions(on)));
        let check = InfoRow::new(
            None,
            Some(as_settings("Check Now", sel!(checkUpdates:))),
            mtm,
        );
        set(check.text(), "Look for updates now");
        let add_group = FormGroup::new(mtm);
        choose.add_to(add_group.view());
        paste.add_to(add_group.view());
        add_group.view().addSubview(auto_update.view());
        check.add_to(add_group.view());
        // No symbol: the row hides behind the extensions' rows once there are some.
        let empty = InfoRow::new(None, None, mtm);
        let installed_group = FormGroup::new(mtm);
        empty.add_to(installed_group.view());
        let intro = paragraph(
            "Extensions run in your main profile, never in the agent's profile. Their actions \
             and keys are in the command palette.",
            mtm,
        );
        intro.setTextColor(Some(&color::secondary_label()));
        let status = paragraph("", mtm);
        status.setFont(Some(&TextRole::RowDetail.font()));
        status.setTextColor(Some(&color::secondary_label()));
        let pane = Self {
            page: ScrollPage::new("Extensions", mtm),
            intro,
            installed_heading: heading("Installed", mtm),
            installed_group,
            rows: RefCell::new(Vec::new()),
            empty,
            add_heading: heading("Add and update", mtm),
            add_group,
            choose,
            paste,
            auto_update,
            check,
            status,
            target,
            settings: settings.clone(),
            mtm,
        };
        let column = pane.page.column();
        for view in [
            &*pane.intro as &NSView,
            &pane.installed_heading,
            pane.installed_group.view(),
            &pane.add_heading,
            pane.add_group.view(),
            &pane.status,
        ] {
            column.addSubview(view);
        }
        pane
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    /// Shows `state` and lays the pane out.
    pub fn update(&self, state: &ExtensionsState) {
        self.rebuild(state);
        for (row, extension) in self.rows.borrow().iter().zip(&state.extensions) {
            row.set_on(extension.enabled);
            row.set_detail(&extension.detail());
        }
        let empty = if state.available {
            "No extension yet"
        } else {
            "Extensions need macOS 15.4 or later"
        };
        set(self.empty.text(), empty);
        set(
            self.paste.text(),
            &format!(
                "Or paste a Chrome Web Store page into the palette ({}) to install from the store",
                state.palette_keys
            ),
        );
        self.auto_update.set_on(state.auto_update);
        self.auto_update.set_enabled(state.available);
        for row in [&self.choose, &self.check] {
            if let Some(button) = row.button() {
                button.setEnabled(state.available);
            }
        }
        set(&self.status, &state.status);

        let mut column = Column::new();
        let intro = wrapped_height(&self.intro, INNER);
        column.full(&self.intro, intro);
        column.gap(space::SECTION);
        column.heading(&self.installed_heading);
        let rows = self.rows.borrow();
        let showing_empty = rows.is_empty();
        self.empty.text().setHidden(!showing_empty);
        let heights = if showing_empty {
            vec![InfoRow::MIN_HEIGHT]
        } else {
            vec![ToggleRow::HEIGHT; rows.len()]
        };
        let (frames, height) = self.installed_group.place(column.origin(), INNER, &heights);
        if showing_empty {
            self.empty.place(frames[0]);
        }
        for (row, frame) in rows.iter().zip(&frames) {
            row.place(frame.origin, frame.size.width);
        }
        column.gap(height + space::SECTION);
        column.heading(&self.add_heading);
        let inner = INNER - FormGroup::INSET * 2.0;
        let (frames, height) = self.add_group.place(
            column.origin(),
            INNER,
            &[
                InfoRow::MIN_HEIGHT,
                self.paste.height(inner),
                ToggleRow::HEIGHT,
                InfoRow::MIN_HEIGHT,
            ],
        );
        self.choose.place(frames[0]);
        self.paste.place(frames[1]);
        self.auto_update
            .place(frames[2].origin, frames[2].size.width);
        self.check.place(frames[3]);
        column.gap(height + space::SM);
        let status_height = if state.status.is_empty() {
            0.0
        } else {
            wrapped_height(&self.status, INNER)
        };
        column.full(&self.status, status_height);
        self.page.set_content_height(column.top);
    }

    /// Makes one row per extension when the extensions changed, each wired to its id.
    fn rebuild(&self, state: &ExtensionsState) {
        let ids: Vec<ExtensionId> = state.extensions.iter().map(|e| e.id.clone()).collect();
        if *self.target.ivars().ids.borrow() == ids {
            return;
        }
        for row in self.rows.borrow_mut().drain(..) {
            row.view().removeFromSuperview();
        }
        let mut rows = Vec::with_capacity(ids.len());
        for (index, extension) in state.extensions.iter().enumerate() {
            let row = ToggleRow::new(&extension.name, "", self.mtm);
            let settings = self.settings.clone();
            let id = extension.id.clone();
            row.on_change(move |on| settings.ask(SettingsAction::SetExtension(id.clone(), on)));
            row.set_accessory(&self.remove_button(&extension.name, index));
            self.installed_group.view().addSubview(row.view());
            rows.push(row);
        }
        *self.rows.borrow_mut() = rows;
        *self.target.ivars().ids.borrow_mut() = ids;
    }

    /// A row's remove button: a borderless `trash`.
    fn remove_button(&self, name: &str, index: usize) -> Retained<NSButton> {
        let description = format!("Remove {name}");
        let button = NSButton::new(self.mtm);
        button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
        button.setBordered(false);
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("trash"),
            Some(&NSString::from_str(&description)),
        );
        button.setImage(image.as_deref());
        button.setToolTip(Some(&NSString::from_str(&description)));
        button.setTag(NSInteger::try_from(index).unwrap_or(NSInteger::MAX));
        let target: &AnyObject = self.target.as_ref();
        // SAFETY: the target is weak; the pane keeps it alive for as long as its buttons.
        unsafe { button.setTarget(Some(target)) };
        // SAFETY: the target implements `remove:`, which takes the sender.
        unsafe { button.setAction(Some(sel!(remove:))) };
        button.sizeToFit();
        button
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_settings::ExtensionId;

    use super::ExtensionState;

    fn state(access: &[&str], off: &[&str]) -> ExtensionState {
        ExtensionState {
            id: ExtensionId::from_digest(&[0; 16]),
            name: "Bitwarden".to_owned(),
            about: "2025.1.0 · Chrome Web Store".to_owned(),
            enabled: true,
            access: access.iter().map(|&text| text.to_owned()).collect(),
            off_on: off.iter().map(|&text| text.to_owned()).collect(),
            on_on: Vec::new(),
        }
    }

    #[test]
    fn a_row_says_what_it_may_do_and_where_it_is_off() {
        assert_eq!(
            state(&["Read all sites", "Keep data"], &["a.com"]).detail(),
            "2025.1.0 · Chrome Web Store · off on a.com · Read all sites and 1 more"
        );
        assert_eq!(
            state(&["Keep data"], &[]).detail(),
            "2025.1.0 · Chrome Web Store · Keep data"
        );
        assert_eq!(state(&[], &[]).detail(), "2025.1.0 · Chrome Web Store");
    }
}

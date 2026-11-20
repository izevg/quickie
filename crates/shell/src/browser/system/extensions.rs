//! Web extensions in the browser: routing what they report, the E3 sheet that asks the user
//! before one runs or gets more, its action popup as an E2 popover, installing from the
//! Chrome Web Store or from disk and updating (each on a worker thread, and only on request
//! unless the user turned automatic updates on), the windows and tabs they see, their keys,
//! and the Settings pane. The decisions are the `Extensions` use case's; this only routes.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::thread;

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use quickie_application::{
    AppError, Approval, ExtensionEvent, ExtensionHost, ExtensionPackages, ExtensionTrigger,
    Fetched, PackageError, PackageSource, ReadOutcome, ReplyToken, UpdateCheck, WindowTabs,
    fetch_packages,
};
use quickie_domain_browsing::{Focus, Jar, JarChoice, TabId, WindowId};
use quickie_domain_commands::KeyStroke;
use quickie_domain_settings::{ExtensionId, ExtensionSource, Grants, SiteAccess};
use quickie_engine_webkit::UrlSessionHttp;
use quickie_system::ExtensionFiles;
use quickie_ui_screens::{ExtensionState, ExtensionsState, Pane};
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{ExtensionPopover, ExtensionRequest, ExtensionSheet, Moment, describe};
use url::Url;

use super::super::Browser;
use crate::Addons;
use crate::input::keystroke::{SheetKey, sheet_key};

/// A decision waiting for the user.
#[derive(Clone, Debug)]
enum Decision {
    /// A package to install, or an update that asks for more.
    Approve(Approval),
    /// A running extension asks for more.
    Request {
        token: ReplyToken,
        id: ExtensionId,
        name: String,
        request: Grants,
    },
}

/// The extensions' widgets and what the browser keeps for them.
pub struct ExtensionUi {
    popover: ExtensionPopover,
    sheet: ExtensionSheet,
    /// The decisions waiting, oldest first; the first shows.
    decisions: RefCell<VecDeque<Decision>>,
    /// The window the sheet shows in, while it shows.
    shown_in: RefCell<Option<WindowId>>,
    /// How the last install or update went, for the Settings pane.
    status: RefCell<String>,
    files: ExtensionFiles,
}

impl ExtensionUi {
    pub fn new(files: ExtensionFiles, mtm: MainThreadMarker) -> Self {
        Self {
            popover: ExtensionPopover::new(mtm),
            sheet: ExtensionSheet::new(mtm),
            decisions: RefCell::new(VecDeque::new()),
            shown_in: RefCell::new(None),
            status: RefCell::new(String::new()),
            files,
        }
    }

    /// Whether the popup popover shows.
    pub fn popup_open(&self) -> bool {
        self.popover.is_open()
    }
}

/// What a package arrived as, and from where it is installed.
type Arrival = (
    Result<quickie_application::Unpacked, PackageError>,
    ExtensionSource,
);

impl Browser {
    /// Wakes the browser for what extensions report, and deletes packages nothing uses any
    /// more, on a worker thread.
    pub(in crate::browser) fn wire_extensions(&self) {
        let locations = {
            let mut extensions = self.extensions.borrow_mut();
            let Some(extensions) = extensions.as_mut() else {
                return;
            };
            let weak = self.this.clone();
            extensions.host_mut().set_waker(move || {
                if let Some(browser) = weak.upgrade() {
                    browser.drain();
                }
            });
            extensions.locations()
        };
        let files = self.extension_ui.files.clone();
        let _ = thread::Builder::new()
            .name("quickie-extension-prune".to_owned())
            .spawn(move || files.prune(&locations));
        let due = self
            .extensions
            .borrow()
            .as_ref()
            .map_or_default(Addons::due_update_checks);
        if !due.is_empty() {
            self.fetch_extensions(due, false);
        }
    }

    /// Routes every waiting extension event; part of [`Browser::drain`].
    pub(in crate::browser) fn drain_extensions(&self) {
        let mut events = Vec::new();
        if let Ok(mut extensions) = self.extensions.try_borrow_mut()
            && let Some(extensions) = extensions.as_mut()
        {
            extensions.host_mut().drain(&mut events);
        }
        for event in events {
            self.extension_event(event);
        }
    }

    fn extension_event(&self, event: ExtensionEvent) {
        match event {
            ExtensionEvent::Read { id, info } => {
                let outcome = self.with_extensions(|extensions| extensions.read(&id, info));
                match outcome {
                    Some(Ok(outcome)) => self.read_outcome(outcome),
                    Some(Err(error)) => {
                        self.extension_status(&format!("That didn't work: {error}"));
                    }
                    None => {}
                }
                self.refresh_settings();
            }
            ExtensionEvent::Failed { id, error } => {
                let name = self.extension_name(&id);
                self.extension_status(&format!("{name} could not run: {error}"));
            }
            ExtensionEvent::Asked {
                token,
                id,
                tab: _,
                request,
            } => {
                let name = self.extension_name(&id);
                self.extension_ui
                    .decisions
                    .borrow_mut()
                    .push_back(Decision::Request {
                        token,
                        id,
                        name,
                        request,
                    });
                self.present_decision();
            }
            ExtensionEvent::Popup { id } => self.show_extension_popup(&id),
            ExtensionEvent::OpenTab {
                token,
                id: _,
                url,
                active,
            } => {
                let tab = self.open_for_extension(url, active);
                self.arrange_extensions();
                if let Some(extensions) = self.extensions.borrow_mut().as_mut() {
                    extensions.host_mut().opened(token, tab);
                }
            }
            ExtensionEvent::ActivateTab(tab) => {
                if let Some(window) = self.run(|browsing| browsing.activate(tab)) {
                    self.settle(window);
                }
            }
            ExtensionEvent::CloseTab(tab) => {
                if let Some(window) = self.window_of(tab) {
                    self.close_tab_in(window, tab);
                }
            }
        }
    }

    fn read_outcome(&self, outcome: ReadOutcome) {
        match outcome {
            ReadOutcome::Approve(approval) => {
                self.extension_ui
                    .decisions
                    .borrow_mut()
                    .push_back(Decision::Approve(approval));
                self.present_decision();
            }
            ReadOutcome::Updated { name, version } => {
                self.extension_status(&format!("{name} updated to {version}"));
            }
            ReadOutcome::UpToDate { name } => {
                self.extension_status(&format!("{name} is up to date"));
            }
            ReadOutcome::Unreadable { id, error } => {
                let name = self.extension_name(&id);
                self.extension_status(&format!("{name} could not be read: {error}"));
            }
            ReadOutcome::Loaded | ReadOutcome::Ignored => {}
        }
    }

    /// A tab for a page an extension opens, in the main jar of the current window.
    fn open_for_extension(&self, url: Url, active: bool) -> Option<TabId> {
        let (window, _) = self.current()?;
        let focus = if active {
            Focus::Foreground
        } else {
            Focus::Background
        };
        let tab = self.run(|browsing| browsing.open(window, url, JarChoice::Main, focus))?;
        self.settle(window);
        Some(tab)
    }

    /// Tells extensions the windows and their tabs as they are now: the current window first,
    /// the main jar's tabs only.
    pub(in crate::browser) fn arrange_extensions(&self) {
        let windows = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            let session = browsing.session();
            let current = self.current().map(|(window, _)| window);
            let mut order: Vec<WindowId> = session.windows().collect();
            order.sort_by_key(|&window| Some(window) != current);
            order
                .into_iter()
                .map(|window| {
                    let tabs: Vec<TabId> = session.tabs(window).map_or_default(|tabs| {
                        tabs.filter(|tab| tab.jar == Jar::Main)
                            .map(|tab| tab.id)
                            .collect()
                    });
                    let active = session.active(window).filter(|tab| tabs.contains(tab));
                    WindowTabs {
                        window,
                        tabs,
                        active,
                    }
                })
                .collect::<Vec<_>>()
        };
        if let Ok(mut extensions) = self.extensions.try_borrow_mut()
            && let Some(extensions) = extensions.as_mut()
        {
            extensions.host_mut().arrange(&windows);
        }
    }

    // ---- the sheet ---------------------------------------------------------------------------

    /// Shows the oldest decision in the current window, unless one shows already.
    fn present_decision(&self) {
        if self.extension_ui.shown_in.borrow().is_some() {
            return;
        }
        let Some(decision) = self.extension_ui.decisions.borrow().front().cloned() else {
            return;
        };
        let Some((window, ns_window)) = self.current() else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        // An E3 collapses every E2 under it.
        self.close_overlays();
        self.hide_find();
        self.hide_navigation();
        let request = match &decision {
            Decision::Approve(approval) => ExtensionRequest {
                name: approval.name.clone(),
                moment: if approval.update {
                    Moment::Update(approval.version.to_string())
                } else {
                    Moment::Install(approval.version.to_string())
                },
                permissions: approval.requested.permissions.iter().cloned().collect(),
                patterns: approval.requested.patterns.iter().cloned().collect(),
            },
            Decision::Request { name, request, .. } => ExtensionRequest {
                name: name.clone(),
                moment: Moment::Running,
                permissions: request.permissions.iter().cloned().collect(),
                patterns: request.patterns.iter().cloned().collect(),
            },
        };
        *self.extension_ui.shown_in.borrow_mut() = Some(window);
        let this = self.this.clone();
        self.extension_ui
            .sheet
            .show(&host, &ns_window, &request, move |allowed| {
                if let Some(browser) = this.upgrade() {
                    browser.decide(allowed);
                    browser.drain();
                }
            });
    }

    /// Whether the extension sheet shows in `window`.
    pub(in crate::browser) fn extension_sheet_open(&self, window: WindowId) -> bool {
        *self.extension_ui.shown_in.borrow() == Some(window)
    }

    /// A key while the extension sheet shows in the current window: `1` allows, `⎋` refuses,
    /// every other key is the sheet's too, so nothing reaches the page under it. `None` when
    /// it does not show.
    pub(in crate::browser) fn extension_sheet_key(&self, stroke: KeyStroke) -> Option<bool> {
        let window = self.current().map(|(window, _)| window)?;
        if !self.extension_sheet_open(window) {
            return None;
        }
        match sheet_key(stroke) {
            Some(SheetKey::Number(1)) => self.decide(true),
            Some(SheetKey::Dismiss) => self.decide(false),
            _ => {}
        }
        Some(true)
    }

    /// The user answered the decision on screen.
    fn decide(&self, allowed: bool) {
        let Some(window) = self.extension_ui.shown_in.borrow_mut().take() else {
            return;
        };
        self.extension_ui.sheet.hide();
        let decision = self.extension_ui.decisions.borrow_mut().pop_front();
        let said = match decision {
            Some(Decision::Approve(approval)) if allowed => self
                .with_extensions(|extensions| extensions.approve(&approval.id))
                .map(|done| match done {
                    Ok(_) => format!("{} added", approval.name),
                    Err(error) => format!("That didn't work: {error}"),
                }),
            Some(Decision::Approve(approval)) => {
                self.with_extensions(|extensions| {
                    extensions.decline(&approval.id);
                    Ok::<(), AppError>(())
                });
                Some(format!("{} not added", approval.name))
            }
            Some(Decision::Request {
                token, id, request, ..
            }) => self
                .with_extensions(|extensions| extensions.answer(token, &id, &request, allowed))
                .and_then(Result::err)
                .map(|error| format!("That didn't work: {error}")),
            None => None,
        };
        if let Some(said) = said {
            self.extension_status(&said);
        }
        self.settle(window);
        self.refresh_settings();
        self.present_decision();
    }

    // ---- the popup --------------------------------------------------------------------------

    fn show_extension_popup(&self, id: &ExtensionId) {
        let popup = self.extensions.borrow().as_ref().and_then(|extensions| {
            let _ = id;
            extensions_popup(extensions)
        });
        let (Some(view), Some((_, ns_window))) = (popup, self.current()) else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        self.close_overlays();
        self.extension_ui
            .popover
            .show(&host, &ns_window, &view, Trigger::Keyboard);
    }

    /// Takes the popup away and tells its extension.
    pub(in crate::browser) fn hide_extension_popup(&self) {
        if !self.extension_ui.popover.is_open() {
            return;
        }
        self.extension_ui.popover.hide(Trigger::Keyboard);
        if let Ok(extensions) = self.extensions.try_borrow()
            && let Some(extensions) = extensions.as_ref()
        {
            extensions.host().close_popup();
        }
    }

    // ---- palette and keys -------------------------------------------------------------------

    /// Runs an extension's action on the active page, or one of its commands.
    pub(in crate::browser) fn run_extension(&self, trigger: &ExtensionTrigger) {
        let tab = self.current().and_then(|(window, _)| self.active(window));
        let done = self.with_extensions(|extensions| extensions.trigger(trigger, tab));
        if let Some(Err(error)) = done {
            self.report(&error);
        }
    }

    /// Runs the extension command `stroke` is the key of; false when it is none's.
    pub(in crate::browser) fn extension_key(&self, stroke: KeyStroke) -> bool {
        let trigger = self
            .extensions
            .borrow()
            .as_ref()
            .and_then(|extensions| extensions.trigger_for(stroke));
        let Some(trigger) = trigger else {
            return false;
        };
        self.close_overlays();
        self.run_extension(&trigger);
        true
    }

    // ---- installing and updating ------------------------------------------------------------

    /// "Install Extension from URL…": the active page when it is an extension's page in the
    /// Chrome Web Store, else the palette, to paste one into.
    pub(in crate::browser) fn install_extension_from_url(&self) {
        let page = self
            .current()
            .and_then(|(window, _)| self.active_tab(window))
            .map(|tab| tab.url)
            .filter(|url| ExtensionId::from_store_url(url).is_some());
        if let Some(page) = page {
            self.install_from_store(&page);
        } else {
            self.open_palette("");
            self.toast("Paste an extension's Chrome Web Store page");
        }
    }

    /// Installs the extension a Chrome Web Store page shows: the store is asked, on a worker
    /// thread, only now that the user asked.
    pub(in crate::browser) fn install_from_store(&self, page: &Url) {
        let check = self
            .extensions
            .borrow()
            .as_ref()
            .map(|extensions| extensions.store_install(page));
        match check {
            Some(Ok(check)) => self.fetch_extensions(vec![check], true),
            Some(Err(error)) => self.report(&error),
            None => self.toast("Extensions need macOS 15.4 or later"),
        }
    }

    /// "Check Extension Updates": every extension from the store.
    pub(in crate::browser) fn check_extension_updates(&self) {
        let checks = self
            .extensions
            .borrow()
            .as_ref()
            .map_or_default(Addons::update_checks);
        if checks.is_empty() {
            self.extension_status("No extension from the Chrome Web Store to update");
            return;
        }
        self.fetch_extensions(checks, true);
    }

    /// Asks the store and downloads on a worker thread; the answers come back to the main
    /// thread. `asked` says the user asked, so a failure is told.
    fn fetch_extensions(&self, checks: Vec<UpdateCheck>, asked: bool) {
        let files = self.extension_ui.files.clone();
        let browser = MainThreadBound::new(self.this.clone(), self.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-extension-fetch".to_owned())
            .spawn(move || {
                let fetched = fetch_packages(&UrlSessionHttp::new(), &files, &checks);
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(browser) = browser.into_inner(mtm).upgrade() {
                        browser.extensions_fetched(fetched, asked);
                    }
                });
            });
        let said = match spawned {
            Ok(_) => "Asking the Chrome Web Store…".to_owned(),
            Err(error) => format!("Couldn't ask the Chrome Web Store: {error}"),
        };
        if asked {
            self.extension_status(&said);
        }
    }

    fn extensions_fetched(&self, fetched: Vec<Fetched>, asked: bool) {
        self.with_extensions(Addons::checked);
        let mut said = Vec::new();
        let mut current = 0;
        for Fetched { id, package } in fetched {
            match package {
                Ok(Some(unpacked)) => {
                    let read = self.with_extensions(|extensions| {
                        extensions.unpacked(unpacked, ExtensionSource::ChromeWebStore)
                    });
                    if let Some(Err(error)) = read {
                        said.push(format!("That didn't work: {error}"));
                    }
                }
                Ok(None) => current += 1,
                Err(error) => said.push(format!("{}: {error}", self.extension_name(&id))),
            }
        }
        if current > 0 && said.is_empty() {
            said.push(if current == 1 {
                "Up to date".to_owned()
            } else {
                format!("All {current} up to date")
            });
        }
        if asked || said.iter().any(|line| !line.is_empty()) {
            self.extension_status(&said.join(" · "));
        }
        self.refresh_settings();
    }

    /// "Install Extension from File…": a `.crx`, a `.zip`, or a folder, chosen in the system's
    /// open panel, checked and kept on a worker thread.
    pub(in crate::browser) fn install_extension_file(&self) {
        if self.extensions.borrow().is_none() {
            return self.toast("Extensions need macOS 15.4 or later");
        }
        let panel = NSOpenPanel::openPanel(self.mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setMessage(Some(&objc2_foundation::NSString::from_str(
            "Choose an extension: a .crx or .zip package, or a folder with a manifest.json",
        )));
        panel.setPrompt(Some(&objc2_foundation::NSString::from_str("Install")));
        let chosen = panel.clone();
        let this = self.this.clone();
        let block = RcBlock::new(move |response: objc2_app_kit::NSModalResponse| {
            if response != NSModalResponseOK {
                return;
            }
            let path = chosen
                .URLs()
                .firstObject()
                .and_then(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()));
            if let (Some(path), Some(browser)) = (path, this.upgrade()) {
                browser.unpack_extension(path);
            }
        });
        panel.beginWithCompletionHandler(&block);
    }

    /// Checks and keeps a file or folder on a worker thread, then reads it.
    fn unpack_extension(&self, path: PathBuf) {
        let (source, kind) = if path.is_dir() {
            (PackageSource::Folder(path), ExtensionSource::Folder)
        } else {
            (PackageSource::File(path), ExtensionSource::File)
        };
        let files = self.extension_ui.files.clone();
        let browser = MainThreadBound::new(self.this.clone(), self.mtm);
        let _ = thread::Builder::new()
            .name("quickie-extension-unpack".to_owned())
            .spawn(move || {
                let arrival: Arrival = (files.unpack(&source, None), kind);
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(browser) = browser.into_inner(mtm).upgrade() {
                        browser.extension_arrived(arrival);
                    }
                });
            });
    }

    fn extension_arrived(&self, (unpacked, source): Arrival) {
        let read = match unpacked {
            Ok(unpacked) => {
                self.with_extensions(|extensions| extensions.unpacked(unpacked, source))
            }
            Err(error) => Some(Err(AppError::Package(error))),
        };
        if let Some(Err(error)) = read {
            self.report(&error);
        }
    }

    // ---- per site and settings --------------------------------------------------------------

    /// "Turn Extensions Off/On on This Site": every installed extension, on the active page's
    /// site. On forgets the decision, so each extension's grants decide again.
    pub(in crate::browser) fn extensions_on_site(&self, off: bool) {
        let Some(url) = self
            .current()
            .and_then(|(window, _)| self.active_tab(window))
            .map(|tab| tab.url)
        else {
            return;
        };
        let access = off.then_some(SiteAccess::Blocked);
        let done = self.with_extensions(|extensions| {
            let ids: Vec<ExtensionId> = extensions.list().iter().map(|e| e.id().clone()).collect();
            ids.iter()
                .map(|id| extensions.set_site_access(id, &url, access))
                .collect::<Result<Vec<bool>, AppError>>()
        });
        let host = url.host_str().unwrap_or_default();
        match done {
            Some(Ok(_)) if off => {
                self.toast(&format!("Extensions off on {host} · reload to apply"));
            }
            Some(Ok(_)) => self.toast(&format!("Extensions on on {host} · reload to apply")),
            Some(Err(error)) => self.report(&error),
            None => self.toast("Extensions need macOS 15.4 or later"),
        }
        self.refresh_settings();
    }

    /// "Show Extensions": the Settings window on its Extensions pane.
    pub(in crate::browser) fn show_extensions(&self) {
        self.open_settings(Some(Pane::Extensions));
    }

    /// What the Extensions pane shows.
    pub(in crate::browser) fn extensions_state(&self) -> ExtensionsState {
        let palette_keys = self.keys_of("quickie.palette.goto");
        let status = self.extension_ui.status.borrow().clone();
        let extensions = self.extensions.borrow();
        let Some(extensions) = extensions.as_ref() else {
            return ExtensionsState {
                available: false,
                extensions: Vec::new(),
                auto_update: false,
                palette_keys,
                status,
            };
        };
        let rows = extensions
            .list()
            .iter()
            .map(|extension| {
                let granted = extension.granted();
                let permissions: Vec<String> = granted.permissions.iter().cloned().collect();
                let patterns: Vec<String> = granted.patterns.iter().cloned().collect();
                let sites = |wanted: SiteAccess| {
                    extension
                        .sites()
                        .filter(|(_, access)| *access == wanted)
                        .map(|(site, _)| site.to_string())
                        .collect()
                };
                ExtensionState {
                    id: extension.id().clone(),
                    name: extension.name().to_owned(),
                    about: format!("{} · {}", extension.version(), extension.source().label()),
                    enabled: extension.enabled(),
                    access: describe(&permissions, &patterns),
                    off_on: sites(SiteAccess::Blocked),
                    on_on: sites(SiteAccess::Allowed),
                }
            })
            .collect();
        ExtensionsState {
            available: true,
            extensions: rows,
            auto_update: extensions.list().auto_update(),
            palette_keys,
            status,
        }
    }

    pub(in crate::browser) fn set_extension(&self, id: &ExtensionId, on: bool) {
        if let Some(Err(error)) = self.with_extensions(|extensions| extensions.set_enabled(id, on))
        {
            self.extension_status(&format!("That didn't work: {error}"));
        }
        self.refresh_settings();
    }

    pub(in crate::browser) fn remove_extension(&self, id: &ExtensionId) {
        let said = match self.with_extensions(|extensions| extensions.remove(id)) {
            Some(Ok(name)) => format!("{name} removed"),
            Some(Err(error)) => format!("That didn't work: {error}"),
            None => return,
        };
        self.extension_status(&said);
        self.refresh_settings();
    }

    pub(in crate::browser) fn auto_update_extensions(&self, on: bool) {
        if let Some(Err(error)) = self.with_extensions(|extensions| extensions.set_auto_update(on))
        {
            self.extension_status(&format!("That didn't work: {error}"));
        }
        self.refresh_settings();
    }

    // ---- helpers ----------------------------------------------------------------------------

    /// Runs an extension use case; `None` on a macOS without extensions.
    fn with_extensions<T>(&self, run: impl FnOnce(&mut Addons) -> T) -> Option<T> {
        let mut extensions = self.extensions.borrow_mut();
        extensions.as_mut().map(run)
    }

    fn extension_name(&self, id: &ExtensionId) -> String {
        self.extensions
            .borrow()
            .as_ref()
            .and_then(|extensions| {
                extensions
                    .list()
                    .get(id)
                    .map(|extension| extension.name().to_owned())
                    .or_else(|| extensions.info(id).map(|info| info.name.clone()))
            })
            .unwrap_or_else(|| "An extension".to_owned())
    }

    /// Says `text` in a toast and in the Settings pane.
    fn extension_status(&self, text: &str) {
        self.toast(text);
        text.clone_into(&mut self.extension_ui.status.borrow_mut());
        self.refresh_settings();
    }
}

/// The popup on its way, as the engine holds it.
fn extensions_popup(extensions: &Addons) -> Option<Retained<objc2_app_kit::NSView>> {
    let (_, view) = extensions.host().popup()?;
    Some(Retained::into_super(view))
}

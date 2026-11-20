//! Web extensions in the main profile: installing from the Chrome Web Store or from disk,
//! approving what an extension asks for, running, stopping, and removing it, its access per
//! site, updates, and its actions and commands for the palette and the keys.
//!
//! Installing never happens behind the user's back. A package arrives (the shell fetches it
//! with [`fetch_packages`] on a worker thread, or unpacks a file the user chose), the engine
//! reads it ([`ExtensionHost::read`]), and [`Extensions::read`] decides: an update that asks
//! for nothing new replaces the running version at once; anything else waits for the user's
//! [`Extensions::approve`] or [`Extensions::decline`] in memory, so the saved list holds no
//! half-installed entries.
//!
//! The same transaction rule as the other use cases: a command changes a copy of the list, the
//! copy is saved, and only then does it replace the list and the engine follow, so a failed
//! save changes nothing.
//!
//! - [`store`]: the Chrome Web Store's update protocol and the worker-thread fetch.

#[cfg(test)]
mod fakes;
mod store;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use quickie_domain_browsing::TabId;
use quickie_domain_commands::{KeyStroke, Platform};
use quickie_domain_settings::{
    CorruptExtensions, ExtensionId, ExtensionSource, ExtensionVersion, Grants, Installed,
    InstalledExtension, InstalledExtensions, Package, Site, SiteAccess,
};
use url::Url;

pub use store::{Fetched, UpdateCheck, UpdateOffer, fetch_packages, parse_answer};

use crate::{
    Access, AppError, Clock, ExtensionHost, ExtensionInfo, ExtensionsRepo, ReplyToken, Unpacked,
};

/// The manifest's name for the command that runs the action.
const EXECUTE_ACTION: &str = "_execute_action";

/// What running an extension's palette row or key does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExtensionRun {
    /// Its action: its popup opens, or it hears a click.
    Action,
    /// The keyboard command of this name.
    Command(String),
}

/// An extension's action or command, as the palette lists it and a key runs it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExtensionTrigger {
    pub id: ExtensionId,
    pub run: ExtensionRun,
}

/// A row the palette offers for a running extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionOffer {
    pub trigger: ExtensionTrigger,
    /// `Bitwarden: Open`, `Bitwarden: Autofill`.
    pub title: String,
    /// The key that runs it, as the manifest suggests it.
    pub stroke: Option<KeyStroke>,
}

/// What the user is asked to approve before an extension runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub id: ExtensionId,
    pub name: String,
    pub version: ExtensionVersion,
    /// What it asks for that it was not granted before.
    pub requested: Grants,
    /// An installed extension's new version, not a new extension.
    pub update: bool,
}

/// What reading a package came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// An installed extension's package, read at start: it runs if enabled.
    Loaded,
    /// The user decides first.
    Approve(Approval),
    /// An update that asks for nothing new now runs in place of the old version.
    Updated {
        name: String,
        version: ExtensionVersion,
    },
    /// The package is not newer than the installed version; it was dropped.
    UpToDate { name: String },
    /// The package could not be read; an install was dropped.
    Unreadable { id: ExtensionId, error: String },
    /// Nothing waited for it.
    Ignored,
}

/// A package on its way in, until the user decides.
#[derive(Clone, Debug)]
struct Pending {
    source: ExtensionSource,
    location: String,
    /// Known once the engine read it.
    info: Option<ExtensionInfo>,
}

/// The extension use cases over the main profile's installed extensions.
#[derive(Debug)]
pub struct Extensions<R, H, C> {
    list: InstalledExtensions,
    /// What the engine read of each installed extension's package.
    infos: BTreeMap<ExtensionId, ExtensionInfo>,
    pending: BTreeMap<ExtensionId, Pending>,
    corrupt_on_start: Option<CorruptExtensions>,
    platform: Platform,
    repo: R,
    host: H,
    clock: C,
}

impl<R: ExtensionsRepo, H: ExtensionHost, C: Clock> Extensions<R, H, C> {
    /// Loads the saved extensions and has the engine read each one's package; those enabled
    /// run once read ([`Self::read`]). A corrupt list starts empty and is reported by
    /// [`Self::corrupt_on_start`]. `platform` reads the keys manifests suggest.
    ///
    /// # Errors
    ///
    /// The store could not be read, or the engine cannot run extensions.
    pub fn start(repo: R, mut host: H, clock: C, platform: Platform) -> Result<Self, AppError> {
        let (list, corrupt_on_start) = match InstalledExtensions::restore(repo.load_extensions()?) {
            Ok(list) => (list, None),
            Err(corrupt) => (InstalledExtensions::new(), Some(corrupt)),
        };
        for extension in list.iter() {
            host.read(extension.id(), extension.location())?;
        }
        Ok(Self {
            list,
            infos: BTreeMap::new(),
            pending: BTreeMap::new(),
            corrupt_on_start,
            platform,
            repo,
            host,
            clock,
        })
    }

    /// The saved list that was discarded at start, if it was.
    #[must_use]
    pub fn corrupt_on_start(&self) -> Option<&CorruptExtensions> {
        self.corrupt_on_start.as_ref()
    }

    /// The installed extensions, for reading.
    #[must_use]
    pub fn list(&self) -> &InstalledExtensions {
        &self.list
    }

    /// What the engine read of an installed extension, `None` before it was read.
    #[must_use]
    pub fn info(&self, id: &ExtensionId) -> Option<&ExtensionInfo> {
        self.infos.get(id)
    }

    /// Where the installed extensions' packages are, and those waiting for the user: what
    /// [`crate::ExtensionPackages::prune`] keeps.
    #[must_use]
    pub fn locations(&self) -> Vec<String> {
        self.list
            .iter()
            .map(|extension| extension.location().to_owned())
            .chain(
                self.pending
                    .values()
                    .map(|pending| pending.location.clone()),
            )
            .collect()
    }

    /// The engine's extension host, for the shell to read what it shows.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// The engine's extension host, for the shell to drain and to tell it about windows.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// The check that installs the extension a Chrome Web Store page shows.
    ///
    /// # Errors
    ///
    /// The page is not an extension's page in the store.
    pub fn store_install(&self, page: &Url) -> Result<UpdateCheck, AppError> {
        let id = ExtensionId::from_store_url(page)
            .ok_or_else(|| AppError::NotAStorePage(page.to_string()))?;
        let installed = self.list.get(&id).map(InstalledExtension::version);
        Ok(UpdateCheck::new(id, installed)?)
    }

    /// The checks of an explicit "Check Extension Updates": every extension from the store.
    #[must_use]
    pub fn update_checks(&self) -> Vec<UpdateCheck> {
        self.list
            .from_store()
            .filter_map(|extension| {
                UpdateCheck::new(extension.id().clone(), Some(extension.version())).ok()
            })
            .collect()
    }

    /// The checks due without the user asking: only when they let updates come on their own,
    /// once a day. None on a fresh profile.
    #[must_use]
    pub fn due_update_checks(&self) -> Vec<UpdateCheck> {
        if self.list.update_due(self.clock.now()) {
            self.update_checks()
        } else {
            Vec::new()
        }
    }

    /// Records that updates were looked for now, so the next look on its own is a day away.
    ///
    /// # Errors
    ///
    /// The store failed; nothing changed.
    pub fn checked(&mut self) -> Result<(), AppError> {
        let mut next = self.list.clone();
        next.checked(self.clock.now());
        self.commit(next)
    }

    /// Lets updates come on their own, or only on request; returns whether that changed.
    ///
    /// # Errors
    ///
    /// The store failed; nothing changed.
    pub fn set_auto_update(&mut self, on: bool) -> Result<bool, AppError> {
        let mut next = self.list.clone();
        if !next.set_auto_update(on) {
            return Ok(false);
        }
        self.commit(next)?;
        Ok(true)
    }

    /// A package arrived and was kept ([`crate::ExtensionPackages::unpack`]); the engine reads
    /// it, and [`Self::read`] decides what becomes of it.
    ///
    /// # Errors
    ///
    /// The engine cannot run extensions.
    pub fn unpacked(
        &mut self,
        unpacked: Unpacked,
        source: ExtensionSource,
    ) -> Result<(), AppError> {
        self.host.read(&unpacked.id, &unpacked.location)?;
        self.pending.insert(
            unpacked.id,
            Pending {
                source,
                location: unpacked.location,
                info: None,
            },
        );
        Ok(())
    }

    /// The engine read `id`'s package ([`crate::ExtensionEvent::Read`]).
    ///
    /// # Errors
    ///
    /// The store failed or the engine refused to run an update; it stays pending no more.
    pub fn read(
        &mut self,
        id: &ExtensionId,
        info: Result<ExtensionInfo, String>,
    ) -> Result<ReadOutcome, AppError> {
        let Some(pending) = self.pending.remove(id) else {
            return self.read_installed(id, info);
        };
        let info = match info {
            Ok(info) => info,
            Err(error) => {
                self.host.discard(id);
                return Ok(ReadOutcome::Unreadable {
                    id: id.clone(),
                    error,
                });
            }
        };
        let installed = self.list.get(id);
        if pending.source == ExtensionSource::ChromeWebStore
            && installed.is_some()
            && !self.list.is_newer(id, &info.version)
        {
            self.host.discard(id);
            return Ok(ReadOutcome::UpToDate { name: info.name });
        }
        let granted = installed.map(|extension| extension.granted().clone());
        let requested = info
            .requested
            .beyond(granted.as_ref().unwrap_or(&Grants::new()));
        if let Some(granted) = granted.filter(|_| requested.is_empty()) {
            let (name, version) = (info.name.clone(), info.version.clone());
            self.install(id, pending.source, pending.location, info, granted)?;
            return Ok(ReadOutcome::Updated { name, version });
        }
        let approval = Approval {
            id: id.clone(),
            name: info.name.clone(),
            version: info.version.clone(),
            requested,
            update: installed.is_some(),
        };
        self.pending.insert(
            id.clone(),
            Pending {
                info: Some(info),
                ..pending
            },
        );
        Ok(ReadOutcome::Approve(approval))
    }

    /// The user approved what `id` asks for: it joins the list, or its new version replaces
    /// the old one, and runs.
    ///
    /// # Errors
    ///
    /// Nothing waits for approval under `id`, the package is malformed, the store failed
    /// (nothing changed), or the engine refused to run it (it stays installed).
    pub fn approve(&mut self, id: &ExtensionId) -> Result<Installed, AppError> {
        let Some(Pending {
            source,
            location,
            info: Some(info),
        }) = self.pending.remove(id)
        else {
            return Err(quickie_domain_settings::ExtensionError::NotInstalled(id.clone()).into());
        };
        let mut granted = self
            .list
            .get(id)
            .map_or_default(|extension| extension.granted().clone());
        granted.add(&info.requested);
        self.install(id, source, location, info, granted)
    }

    /// The user declined: the package is dropped, and an installed version stays as it was.
    pub fn decline(&mut self, id: &ExtensionId) {
        if self.pending.remove(id).is_some() {
            self.host.discard(id);
        }
    }

    /// Runs an installed extension or stops it; returns whether that changed anything.
    ///
    /// # Errors
    ///
    /// It is not installed, the store failed (nothing changed), or the engine refused to run
    /// it (it stays enabled).
    pub fn set_enabled(&mut self, id: &ExtensionId, enabled: bool) -> Result<bool, AppError> {
        let mut next = self.list.clone();
        if !next.set_enabled(id, enabled)? {
            return Ok(false);
        }
        self.commit(next)?;
        if enabled {
            self.run(id)?;
        } else {
            self.host.stop(id);
        }
        Ok(true)
    }

    /// Removes an installed extension and its files; returns its name.
    ///
    /// # Errors
    ///
    /// It is not installed, or the store failed (nothing changed).
    pub fn remove(&mut self, id: &ExtensionId) -> Result<String, AppError> {
        let mut next = self.list.clone();
        let removed = next
            .remove(id)
            .ok_or_else(|| quickie_domain_settings::ExtensionError::NotInstalled(id.clone()))?;
        self.commit(next)?;
        self.infos.remove(id);
        self.host.remove(id);
        Ok(removed.name().to_owned())
    }

    /// Allows or blocks `id` on the site of the page at `url`, or forgets the decision;
    /// returns whether that changed anything. A running extension takes it at once.
    ///
    /// # Errors
    ///
    /// The page is off the web, the extension is not installed, the store failed (nothing
    /// changed), or the engine refused the change.
    pub fn set_site_access(
        &mut self,
        id: &ExtensionId,
        url: &Url,
        access: Option<SiteAccess>,
    ) -> Result<bool, AppError> {
        let site = Site::try_from(url)?;
        let mut next = self.list.clone();
        if !next.set_access(id, site, access)? {
            return Ok(false);
        }
        self.commit(next)?;
        self.run(id)?;
        Ok(true)
    }

    /// Answers a running extension's request for more ([`crate::ExtensionEvent::Asked`]):
    /// allowed, the grants are saved and the extension gets them; refused, it gets nothing.
    ///
    /// # Errors
    ///
    /// The store failed: the extension gets nothing and nothing is saved.
    pub fn answer(
        &mut self,
        token: ReplyToken,
        id: &ExtensionId,
        request: &Grants,
        allowed: bool,
    ) -> Result<(), AppError> {
        if !allowed || self.list.get(id).is_none() {
            self.host.answer(token, &Grants::new());
            return Ok(());
        }
        let mut next = self.list.clone();
        next.grant(id, request)?;
        if let Err(error) = self.commit(next) {
            self.host.answer(token, &Grants::new());
            return Err(error);
        }
        self.host.answer(token, request);
        Ok(())
    }

    /// The palette's rows for running extensions: each action, then each command, in install
    /// order. A command that runs the action gives the action its key.
    #[must_use]
    pub fn offers(&self) -> Vec<ExtensionOffer> {
        let mut offers = Vec::new();
        for extension in self.list.iter().filter(|extension| extension.enabled()) {
            let Some(info) = self.infos.get(extension.id()) else {
                continue;
            };
            let id = extension.id();
            if let Some(label) = &info.action {
                let stroke = info
                    .commands
                    .iter()
                    .find(|command| command.name == EXECUTE_ACTION)
                    .and_then(|command| self.stroke(command.shortcut.as_deref()));
                offers.push(ExtensionOffer {
                    trigger: ExtensionTrigger {
                        id: id.clone(),
                        run: ExtensionRun::Action,
                    },
                    title: format!("{}: {label}", info.name),
                    stroke,
                });
            }
            for command in info
                .commands
                .iter()
                .filter(|command| command.name != EXECUTE_ACTION)
            {
                let what = if command.title.is_empty() {
                    &command.name
                } else {
                    &command.title
                };
                offers.push(ExtensionOffer {
                    trigger: ExtensionTrigger {
                        id: id.clone(),
                        run: ExtensionRun::Command(command.name.clone()),
                    },
                    title: format!("{}: {what}", info.name),
                    stroke: self.stroke(command.shortcut.as_deref()),
                });
            }
        }
        offers
    }

    /// What `stroke` runs among running extensions' keys, the first installed first.
    #[must_use]
    pub fn trigger_for(&self, stroke: KeyStroke) -> Option<ExtensionTrigger> {
        self.offers()
            .into_iter()
            .find(|offer| offer.stroke == Some(stroke))
            .map(|offer| offer.trigger)
    }

    /// Runs an extension's action on `tab`'s page, or one of its commands.
    ///
    /// # Errors
    ///
    /// The engine refused: the extension is not running, or has no such action or command.
    pub fn trigger(
        &mut self,
        trigger: &ExtensionTrigger,
        tab: Option<TabId>,
    ) -> Result<(), AppError> {
        match &trigger.run {
            ExtensionRun::Action => self.host.perform_action(&trigger.id, tab)?,
            ExtensionRun::Command(name) => self.host.perform_command(&trigger.id, name)?,
        }
        Ok(())
    }

    /// An installed extension's package as read at start: kept, and run when enabled.
    fn read_installed(
        &mut self,
        id: &ExtensionId,
        info: Result<ExtensionInfo, String>,
    ) -> Result<ReadOutcome, AppError> {
        let Some(enabled) = self.list.get(id).map(InstalledExtension::enabled) else {
            return Ok(ReadOutcome::Ignored);
        };
        let info = match info {
            Ok(info) => info,
            Err(error) => {
                return Ok(ReadOutcome::Unreadable {
                    id: id.clone(),
                    error,
                });
            }
        };
        self.infos.insert(id.clone(), info);
        if enabled {
            self.run(id)?;
        }
        Ok(ReadOutcome::Loaded)
    }

    /// Puts a read package in the list with `granted`, saves, and runs it.
    fn install(
        &mut self,
        id: &ExtensionId,
        source: ExtensionSource,
        location: String,
        info: ExtensionInfo,
        granted: Grants,
    ) -> Result<Installed, AppError> {
        let package = Package {
            id: id.clone(),
            name: info.name.clone(),
            version: info.version.clone(),
            source,
            location,
        };
        let mut next = self.list.clone();
        let installed = match next.install(package, granted) {
            Ok(installed) => installed,
            Err(error) => {
                self.host.discard(id);
                return Err(error.into());
            }
        };
        self.commit(next)?;
        self.infos.insert(id.clone(), info);
        if self.list.get(id).is_some_and(InstalledExtension::enabled) {
            self.run(id)?;
        }
        Ok(installed)
    }

    /// Has the engine run `id` with what the list says it may do.
    fn run(&mut self, id: &ExtensionId) -> Result<(), AppError> {
        let Some(extension) = self.list.get(id) else {
            return Ok(());
        };
        let access = Access {
            granted: extension.granted().clone(),
            sites: extension
                .sites()
                .map(|(site, access)| (site.clone(), access))
                .collect(),
        };
        self.host.start(id, &access)?;
        Ok(())
    }

    fn stroke(&self, shortcut: Option<&str>) -> Option<KeyStroke> {
        KeyStroke::parse(shortcut?, self.platform).ok()
    }

    fn commit(&mut self, next: InstalledExtensions) -> Result<(), AppError> {
        self.repo.save_extensions(next.snapshot())?;
        self.list = next;
        Ok(())
    }
}

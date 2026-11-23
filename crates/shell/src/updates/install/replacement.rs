use objc2::rc::Retained;
use objc2_foundation::{
    NSCocoaErrorDomain, NSError, NSFileManager, NSFileManagerItemReplacementOptions,
    NSFileWriteNoPermissionError, NSPOSIXErrorDomain, NSSearchPathDirectory,
    NSSearchPathDomainMask, NSString, NSURL,
};
use std::fs::{self, File};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::bundle::{BundleIdentity, validated_executable};
use super::privileged::{self, BundleProof};
use super::{APP_NAME, InstallError};

pub(super) struct Replacement {
    installed: PathBuf,
    installed_id: FileIdentity,
    staged: PathBuf,
    staged_id: Option<FileIdentity>,
    directory: StageDirectory,
    transaction: tempfile::TempDir,
    lock: File,
    proof: Option<BundleProof>,
}

#[derive(PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn directory(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
        if !metadata.is_dir() {
            return Err(format!(
                "{} is not an application directory.",
                path.display()
            ));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

struct StageDirectory {
    path: PathBuf,
    preserve: bool,
}

impl Drop for StageDirectory {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

impl Replacement {
    pub(super) fn new(installed: PathBuf) -> Result<Self, String> {
        let installed_id = FileIdentity::directory(&installed)?;
        let lock = File::open(validated_executable(&installed)?).map_err(|error| {
            format!("Could not open the installed executable for locking: {error}")
        })?;
        lock.try_lock()
            .map_err(|error| format!("Could not lock Quickie for an exclusive update: {error}"))?;

        let url = directory_url(&installed)?;
        let stage = NSFileManager::defaultManager()
            .URLForDirectory_inDomain_appropriateForURL_create_error(
                NSSearchPathDirectory::ItemReplacementDirectory,
                NSSearchPathDomainMask::UserDomainMask,
                Some(&url),
                true,
            )
            .map_err(|error| {
                format!("Could not prepare staging on the Quickie installation volume: {error}")
            })?
            .to_file_path()
            .ok_or("The system returned an invalid staging destination.")?;
        let metadata = fs::symlink_metadata(&stage)
            .map_err(|error| format!("Could not inspect the update staging directory: {error}"))?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err("The system staging directory does not provide private storage.".into());
        }
        let directory = StageDirectory {
            path: stage,
            preserve: false,
        };
        if metadata.dev() != installed_id.device {
            return Err("The staging directory is on a different volume than Quickie.".into());
        }
        let transaction = tempfile::Builder::new()
            .prefix("quickie-")
            .tempdir_in(&directory.path)
            .map_err(|error| format!("Could not isolate the staged update: {error}"))?;
        Ok(Self {
            installed,
            installed_id,
            staged: transaction.path().join(APP_NAME),
            staged_id: None,
            directory,
            transaction,
            lock,
            proof: None,
        })
    }

    pub(super) fn installed(&self) -> &Path {
        &self.installed
    }

    pub(super) fn staged(&self) -> &Path {
        &self.staged
    }

    pub(super) fn set_proof(&mut self, proof: BundleProof) {
        self.proof = Some(proof);
    }

    pub(super) fn seal(&mut self) -> Result<(), String> {
        self.staged_id = Some(FileIdentity::directory(&self.staged)?);
        self.recheck()
    }

    pub(super) fn recheck(&self) -> Result<(), String> {
        let staged = FileIdentity::directory(&self.staged)?;
        if !self.original_present()
            || Some(&staged) != self.staged_id.as_ref()
            || staged.device != self.installed_id.device
        {
            return Err("The installed or staged Quickie bundle changed during the update.".into());
        }
        Ok(())
    }

    fn original_present(&self) -> bool {
        FileIdentity::directory(&self.installed)
            .is_ok_and(|installed| installed == self.installed_id)
            && self.lock.metadata().is_ok_and(|locked| {
                validated_executable(&self.installed)
                    .ok()
                    .and_then(|path| fs::symlink_metadata(path).ok())
                    .is_some_and(|current| {
                        locked.dev() == current.dev() && locked.ino() == current.ino()
                    })
            })
    }

    pub(super) fn apply(
        mut self,
        version: &str,
        complete: impl FnOnce(Result<PathBuf, InstallError>) + Send + 'static,
    ) {
        let manager = NSFileManager::defaultManager();
        let backup = match self.backup() {
            Ok(path) => path,
            Err(error) => return complete(Err(InstallError::Failed(error))),
        };
        match replace(&manager, &self.installed, &self.staged, backup.file_name()) {
            Ok(()) => {
                complete(self.finish(&manager, version, &backup));
            }
            Err(ReplacementError::Native(error))
                if no_permission(&error) && self.recheck().is_ok() =>
            {
                let result = self
                    .authorized_replacement(version)
                    .map(|()| self.installed.clone());
                complete(result);
            }
            Err(error) => complete(Err(self.failure(error.message(), &backup))),
        }
    }

    fn backup(&self) -> Result<PathBuf, String> {
        // The private directory has a random name unique to this transaction.
        let token = self
            .transaction
            .path()
            .file_name()
            .ok_or("The staging directory is missing.")?;
        let mut name = std::ffi::OsString::from(".Quickie-update-backup-");
        name.push(token);
        let backup = self.installed.with_file_name(name);
        match fs::symlink_metadata(&backup) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(backup),
            _ => Err("A recovery copy already uses the Quickie update backup path.".into()),
        }
    }

    fn authorized_replacement(&self, version: &str) -> Result<(), InstallError> {
        let proof = self.proof.as_ref().ok_or_else(|| {
            InstallError::Failed(
                "The staged Quickie update has no verified signing identity.".into(),
            )
        })?;
        privileged::replace(&self.installed, &self.staged, version, proof)
    }

    fn finish(
        &mut self,
        manager: &NSFileManager,
        version: &str,
        backup: &Path,
    ) -> Result<PathBuf, InstallError> {
        let installed =
            BundleIdentity::read(&self.installed).and_then(|bundle| bundle.verify_version(version));
        if let Err(error) = installed {
            if FileIdentity::directory(backup).is_ok_and(|identity| identity == self.installed_id) {
                let restored = if self.installed.exists() {
                    replace(manager, &self.installed, backup, None)
                        .map_err(ReplacementError::message)
                } else {
                    fs::rename(backup, &self.installed).map_err(|error| error.to_string())
                };
                if restored.is_ok() {
                    return Err(InstallError::Failed(format!(
                        "The installed update failed verification and Quickie was restored: {error}"
                    )));
                }
            }
            return Err(self.failure(error, backup));
        }
        // Protected installations can leave this copy in place if the user cannot remove it.
        let _ = fs::remove_dir_all(backup);
        Ok(self.installed.clone())
    }

    fn failure(&mut self, reason: String, backup: &Path) -> InstallError {
        if self.original_present() {
            return InstallError::Failed(reason);
        }
        self.directory.preserve = true;
        self.transaction.disable_cleanup(true);
        InstallError::Failed(format!(
            "{reason}. Keep the recovery copy at {} and the staging directory at {}.",
            backup.display(),
            self.directory.path.display(),
        ))
    }
}

enum ReplacementError {
    Path(String),
    Native(Retained<NSError>),
}

impl ReplacementError {
    fn message(self) -> String {
        match self {
            Self::Path(error) => error,
            Self::Native(error) => {
                format!("Could not replace the installed Quickie bundle: {error}")
            }
        }
    }
}

fn directory_url(path: &Path) -> Result<Retained<NSURL>, String> {
    NSURL::from_directory_path(path)
        .ok_or_else(|| format!("Invalid bundle path: {}", path.display()))
}

fn replace(
    manager: &NSFileManager,
    installed: &Path,
    staged: &Path,
    backup: Option<&std::ffi::OsStr>,
) -> Result<(), ReplacementError> {
    let installed = directory_url(installed).map_err(ReplacementError::Path)?;
    let staged = directory_url(staged).map_err(ReplacementError::Path)?;
    let backup = backup
        .map(|backup| {
            backup.to_str().map(NSString::from_str).ok_or_else(|| {
                ReplacementError::Path("The system returned an invalid recovery copy name.".into())
            })
        })
        .transpose()?;
    let mut options = NSFileManagerItemReplacementOptions::UsingNewMetadataOnly;
    if backup.is_some() {
        options |= NSFileManagerItemReplacementOptions::WithoutDeletingBackupItem;
    }
    manager
        .replaceItemAtURL_withItemAtURL_backupItemName_options_resultingItemURL_error(
            &installed,
            &staged,
            backup.as_deref(),
            options,
            None,
        )
        .map_err(ReplacementError::Native)
}

fn no_permission(error: &NSError) -> bool {
    // SAFETY: Foundation exports immutable, process-wide NSError domain strings.
    let cocoa = unsafe { NSCocoaErrorDomain };
    // SAFETY: Foundation exports an immutable, process-wide NSError domain string.
    let posix = unsafe { NSPOSIXErrorDomain };
    let domain = error.domain();
    (&*domain == cocoa && error.code() == NSFileWriteNoPermissionError)
        || (&*domain == posix && matches!(error.code(), 1 | 13))
}

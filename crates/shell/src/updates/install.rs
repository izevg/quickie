//! Staging and replacement of a released macOS application bundle.

mod bundle;
mod disk_image;
mod privileged;
mod replacement;
#[cfg(test)]
mod tests;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;

use bundle::{BundleIdentity, Signature, validated_executable, version};
use disk_image::MountedImage;
use objc2::MainThreadMarker;
use objc2::rc::autoreleasepool;
use privileged::BundleProof;
use quickie_updater::Version;
use replacement::Replacement;

const APP_NAME: &str = "Quickie.app";

#[derive(Debug)]
pub(super) enum InstallError {
    Cancelled,
    Failed(String),
}

pub(super) struct StagedUpdate {
    replacement: Replacement,
    installed_version: String,
    release_version: String,
    signature: Signature,
}

/// Resolve the running package. A source build, an app translocated by Launch Services,
/// and a path outside a matching Quickie bundle have no updater identity.
pub(super) fn installed_bundle(installed_version: &str) -> Result<PathBuf, String> {
    let release = Version::parse(installed_version).map_err(|error| error.to_string())?;
    if (release.major, release.minor, release.patch) == (0, 0, 0) {
        return Err("Development builds do not have an updater identity.".into());
    }
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| format!("Could not locate the running executable: {error}"))?;
    let bundle = executable
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .filter(|path| {
            !path
                .components()
                .any(|component| component.as_os_str() == "AppTranslocation")
        })
        .ok_or("Quickie is not running from an installed application bundle.")?;
    if validated_executable(bundle)? != executable {
        return Err("The running executable is outside the Quickie bundle layout.".into());
    }
    let bundle_version = format!("{}.{}.{}", release.major, release.minor, release.patch);
    BundleIdentity::read(bundle)?.verify_version(&bundle_version)?;
    Ok(bundle.to_owned())
}

/// Inspect the verified download and copy its application to the installation volume.
pub(super) fn stage(
    dmg: &Path,
    installed_version: &str,
    release_version: &str,
) -> Result<StagedUpdate, String> {
    autoreleasepool(|_| {
        let released = Version::parse(release_version).map_err(|error| error.to_string())?;
        let installed = Version::parse(installed_version).map_err(|error| error.to_string())?;
        version(release_version, 3)?;
        if !released.cmp_precedence(&installed).is_gt() {
            return Err("The update must be newer than the installed Quickie version.".into());
        }
        let installed = installed_bundle(installed_version)?;
        let (replacement, signature) = stage_package(dmg, installed, release_version)?;
        Ok(StagedUpdate {
            replacement,
            installed_version: installed_version.into(),
            release_version: release_version.into(),
            signature,
        })
    })
}

fn stage_package(
    dmg: &Path,
    installed: PathBuf,
    release_version: &str,
) -> Result<(Replacement, Signature), String> {
    let mut replacement = Replacement::new(installed)?;
    let signature = Signature::read(replacement.installed())?;
    let original_hash = bundle::code_hash(replacement.installed())?;
    let original_metadata = fs::symlink_metadata(replacement.installed())
        .map_err(|error| format!("Could not inspect the installed Quickie bundle: {error}"))?;
    let image = MountedImage::attach(dmg)?;
    let source = image.volume()?.join(APP_NAME);
    bundle::verify(&source, release_version, &signature)?;
    let candidate_hash = bundle::code_hash(&source)?;
    copy_bundle(&source, replacement.staged())?;
    image.detach()?;
    replacement.seal()?;
    bundle::preserve_quarantine(replacement.staged())?;
    bundle::verify(replacement.staged(), release_version, &signature)?;
    replacement.set_proof(BundleProof {
        original: original_hash,
        candidate: candidate_hash,
        owner: original_metadata.uid(),
        group: original_metadata.gid(),
    });
    Ok((replacement, signature))
}

fn copy_bundle(source: &Path, destination: &Path) -> Result<(), String> {
    run_tool(
        Command::new("/usr/bin/ditto")
            .args(["--rsrc", "--extattr", "--qtn"])
            .arg(source)
            .arg(destination),
        "Could not copy the Quickie bundle on the installation volume",
    )?;
    Ok(())
}

impl StagedUpdate {
    /// Start replacement after the browser has stopped session writes. Signature verification
    /// and replacement use a worker. The caller keeps its termination handshake open.
    pub(super) fn apply(
        self,
        _: MainThreadMarker,
        complete: impl FnOnce(Result<PathBuf, InstallError>) + Send + 'static,
    ) {
        let complete = Arc::new(std::sync::Mutex::new(Some(complete)));
        let worker_complete = complete.clone();
        let worker = thread::Builder::new().name("quickie-update-install".into());
        if let Err(error) = worker.spawn(move || {
            autoreleasepool(|_| {
                let result = installed_bundle(&self.installed_version).and_then(|bundle| {
                    if bundle != self.replacement.installed() {
                        return Err("The installed Quickie location has changed.".into());
                    }
                    self.replacement.recheck()?;
                    bundle::verify(
                        self.replacement.staged(),
                        &self.release_version,
                        &self.signature,
                    )
                });
                if let Err(error) = result {
                    finish(&worker_complete, Err(InstallError::Failed(error)));
                    return;
                }
                self.replacement
                    .apply(&self.release_version, move |result| {
                        finish(&worker_complete, result);
                    });
            });
        }) {
            finish(
                &complete,
                Err(InstallError::Failed(format!(
                    "Could not start update installation: {error}"
                ))),
            );
        }
    }
}

fn finish<F>(complete: &std::sync::Mutex<Option<F>>, result: Result<PathBuf, InstallError>)
where
    F: FnOnce(Result<PathBuf, InstallError>),
{
    if let Some(complete) = complete
        .lock()
        .ok()
        .and_then(|mut complete| complete.take())
    {
        complete(result);
    }
}

fn run_tool(command: &mut Command, operation: &str) -> Result<std::process::Output, String> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{operation}: {error}"))?;
    if output.status.success() {
        return Ok(output);
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    Err(format!("{operation}: {}", detail.trim()))
}

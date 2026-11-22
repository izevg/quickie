use std::env;
use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use quickie_updater::{Distribution, UpdateAsset, Version, fetch_latest};
use rustix::fs::{Mode, OFlags};

use crate::updates::network::{Cancellation, SoupTransport};

use super::super::{
    InstallError, Installation, Target, checked, failure, stable_version, system, trusted_root_path,
};
use super::{Distro, PACKAGED_EXECUTABLE, apply_archive};

#[cfg(test)]
mod tests;

pub(crate) const INSTALL_PACKAGE_ARGUMENT: &str = "--install-verified-package";
const ROOT_STAGE: &str = "/var/tmp";
const CONFIRMATION_DEADLINE: Duration = Duration::from_secs(40);

pub(super) fn request(
    distribution: Distro,
    source: &Path,
    version: &Version,
) -> Result<(), InstallError> {
    trusted_runtime()?;
    checked(&mut authorization_command(distribution, source, version))?;
    Ok(())
}

fn authorization_command(distribution: Distro, source: &Path, version: &Version) -> Command {
    let mut command = system("/usr/bin/pkexec");
    command
        .args([
            "--disable-internal-agent",
            PACKAGED_EXECUTABLE,
            INSTALL_PACKAGE_ARGUMENT,
        ])
        .arg(match distribution {
            Distro::Ubuntu => "deb",
            Distro::Fedora => "rpm",
        })
        .arg(version.to_string())
        .arg(source);
    command
}

pub(crate) fn install_privileged(
    current: &str,
    arguments: impl Iterator<Item = OsString>,
) -> Result<(), InstallError> {
    if !rustix::process::geteuid().is_root() {
        return Err(failure(
            "package installation requires system authorization",
        ));
    }
    let (requested_distro, requested_version, source) = parse_request(arguments)?;
    let invoking_user = env::var("PKEXEC_UID")
        .ok()
        .and_then(|uid| uid.parse::<u32>().ok())
        .filter(|uid| *uid != 0)
        .ok_or_else(|| failure("could not identify the user who authorized the update"))?;
    trusted_runtime()?;
    trusted_parent(invoking_user)?;
    let installation = Installation::detect(current).ok_or_else(|| {
        failure("the installed Quickie version changed. Check for updates again.")
    })?;
    if installation.target != Target::Package(requested_distro)
        || requested_version <= installation.current
    {
        return Err(failure(
            "the update no longer applies to this Quickie installation",
        ));
    }
    let asset = approved_release(
        installation.kind(),
        &installation.current,
        &requested_version,
    )?;
    install_root_archive(
        requested_distro,
        &source,
        &asset,
        &installation.current,
        invoking_user,
    )
}

fn parse_request(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<(Distro, Version, PathBuf), InstallError> {
    let bad_request = || failure("invalid package update request");
    let distribution = match arguments.next().as_deref().and_then(|name| name.to_str()) {
        Some("deb") => Distro::Ubuntu,
        Some("rpm") => Distro::Fedora,
        _ => return Err(bad_request()),
    };
    let version = arguments
        .next()
        .as_deref()
        .and_then(|value| value.to_str())
        .and_then(stable_version)
        .ok_or_else(bad_request)?;
    let source = PathBuf::from(arguments.next().ok_or_else(bad_request)?);
    if !source.is_absolute() || arguments.next().is_some() {
        return Err(bad_request());
    }
    Ok((distribution, version, source))
}

fn trusted_runtime() -> Result<(), InstallError> {
    let executable = Path::new(PACKAGED_EXECUTABLE);
    let libraries = Path::new("/opt/quickie/lib");
    if !trusted_root_path(executable) || !trusted_root_path(libraries) {
        return Err(failure(
            "the installed Quickie runtime is outside the trusted system path",
        ));
    }
    for entry in fs::read_dir(libraries)? {
        let entry = entry?;
        let path = entry.path();
        // Shared library aliases are allowed if they belong to root and resolve within
        // the same trusted library directory.
        let actual = path.canonicalize()?;
        if fs::symlink_metadata(&path)?.uid() != 0
            || !actual.starts_with(libraries)
            || !trusted_root_path(&actual)
        {
            return Err(failure("the installed Quickie libraries are not protected"));
        }
    }
    Ok(())
}

fn trusted_parent(invoking_user: u32) -> Result<(), InstallError> {
    let invalid_parent = || failure("package authorization must come from the installed Quickie");
    let parent = rustix::process::getppid().ok_or_else(invalid_parent)?;
    let process = Path::new("/proc").join(parent.as_raw_pid().to_string());
    let status = fs::read_to_string(process.join("status"))?;
    let owner = status.lines().find_map(|line| {
        line.strip_prefix("Uid:")?
            .split_ascii_whitespace()
            .next()?
            .parse::<u32>()
            .ok()
    });
    let parent = fs::metadata(process.join("exe"))?;
    let installed = fs::metadata(PACKAGED_EXECUTABLE)?;
    if owner != Some(invoking_user) || !same_inode(&parent, &installed) {
        return Err(invalid_parent());
    }
    Ok(())
}

fn approved_release(
    distribution: Distribution,
    installed: &Version,
    requested: &Version,
) -> Result<UpdateAsset, InstallError> {
    let cancellation = Cancellation::new().map_err(InstallError::Failed)?;
    let (finished, wait) = mpsc::channel::<()>();
    let latest = thread::scope(|scope| {
        let deadline = &cancellation;
        let timer = thread::Builder::new()
            .name("quickie-update-confirmation".into())
            .spawn_scoped(scope, move || {
                if wait.recv_timeout(CONFIRMATION_DEADLINE).is_err() {
                    deadline.cancel();
                }
            })?;
        let result = fetch_latest(&SoupTransport(&cancellation), installed, distribution);
        let _ = finished.send(());
        let _ = timer.join();
        Ok::<_, io::Error>(result)
    })?
    .map_err(|error| {
        failure(&format!(
            "could not confirm the approved release ({error}). Check for updates again."
        ))
    })?;
    latest
        .filter(|asset| asset.version() == requested)
        .ok_or_else(|| failure("the approved release changed. Check for updates again."))
}

pub(super) fn install_root_archive(
    distribution: Distro,
    source: &Path,
    asset: &UpdateAsset,
    installed: &Version,
    source_owner: u32,
) -> Result<(), InstallError> {
    if !rustix::process::geteuid().is_root() {
        return Err(failure(
            "package installation requires system authorization",
        ));
    }
    let staging = root_staging(Path::new(ROOT_STAGE))?;
    let archive = stage_archive(source, source_owner, asset, staging.path())?;
    apply_archive(distribution, &archive, asset, installed)
}

fn root_staging(base: &Path) -> Result<tempfile::TempDir, InstallError> {
    let trusted = fs::symlink_metadata(base)?;
    if !base.parent().is_some_and(trusted_root_path)
        || !trusted.is_dir()
        || trusted.uid() != 0
        || (trusted.mode() & 0o022 != 0 && trusted.mode() & 0o1000 == 0)
    {
        return Err(failure("the system temporary directory is not protected"));
    }
    let staging = tempfile::Builder::new()
        .prefix("quickie-update-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(base)?;
    let properties = fs::symlink_metadata(staging.path())?;
    if !properties.is_dir() || properties.uid() != 0 || properties.mode() & 0o077 != 0 {
        return Err(failure(
            "the package staging directory is not private to root",
        ));
    }
    Ok(staging)
}

fn stage_archive(
    source: &Path,
    source_owner: u32,
    asset: &UpdateAsset,
    directory: &Path,
) -> Result<PathBuf, InstallError> {
    let mut source = open_source(source, source_owner)?;
    let previous = source.metadata()?;
    if previous.len() != asset.size() {
        return Err(failure(
            "the staged download has an unexpected size. Download the update again.",
        ));
    }
    let path = directory.join(asset.name());
    let mut destination = File::options()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)?;
    io::copy(&mut (&mut source).take(asset.size() + 1), &mut destination)?;
    let current = source.metadata()?;
    if !private_file(&current, source_owner)
        || !same_inode(&previous, &current)
        || previous.len() != current.len()
        || previous.mtime() != current.mtime()
        || previous.mtime_nsec() != current.mtime_nsec()
        || previous.ctime() != current.ctime()
        || previous.ctime_nsec() != current.ctime_nsec()
    {
        return Err(failure(
            "the staged download changed. Download the update again.",
        ));
    }
    asset
        .verify(File::open(&path)?)
        .map_err(|error| failure(&format!("{error}. Download the update again.")))?;
    destination.sync_all()?;
    Ok(path)
}

fn open_source(path: &Path, owner: u32) -> Result<File, InstallError> {
    // O_PATH snapshots the last component without reading it, including a dangling symlink,
    // FIFO, or special device. Opening the proc descriptor then uses that same inode.
    let reference = File::from(
        rustix::fs::open(
            path,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let expected = reference.metadata()?;
    if !private_file(&expected, owner) {
        return Err(failure(
            "the staged download is not a private regular file owned by the authorizing user",
        ));
    }
    let file = File::from(
        rustix::fs::open(
            format!("/proc/self/fd/{}", reference.as_raw_fd()),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let actual = file.metadata()?;
    if !same_inode(&expected, &actual) || !private_file(&actual, owner) {
        return Err(failure(
            "the staged download changed before it could be read",
        ));
    }
    Ok(file)
}

fn private_file(properties: &Metadata, owner: u32) -> bool {
    properties.is_file()
        && properties.uid() == owner
        && properties.mode() & 0o7077 == 0
        && properties.nlink() == 1
}

fn same_inode(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

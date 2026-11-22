use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, Metadata, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use quickie_updater::UpdateAsset;

use super::{
    InstallError, Target, checked, current_uid, failure, system, trusted_root_path, verify_asset,
};

pub(super) fn appimage_header(mut source: impl Read) -> bool {
    let mut header = [0; 20];
    source.read_exact(&mut header).is_ok()
        && header[..7] == *b"\x7fELF\x02\x01\x01"
        && header[8..11] == *b"AI\x02"
        && header[18..20] == [0x3e, 0]
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn image_uri(path: &Path) -> String {
    use std::fmt::Write as _;

    let mut uri = String::from("admin://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

pub(super) fn detect_appimage(executable: &Path) -> Option<Target> {
    let path = PathBuf::from(env::var_os("APPIMAGE")?);
    let appdir = PathBuf::from(env::var_os("APPDIR")?).canonicalize().ok()?;
    let temp = env::temp_dir().canonicalize().ok()?;
    if !path.is_absolute()
        || path.canonicalize().ok()? != path
        || appdir.parent()? != temp
        || executable != appdir.join("usr/bin/quickie")
        || appdir.join("AppRun").canonicalize().ok()? != executable
        || !appimage_header(File::open(&path).ok()?)
    {
        return None;
    }
    let name = appdir.file_name()?.to_str()?;
    let extracted = if name.starts_with(".mount_") && mounted_appimage(&appdir) {
        false
    } else {
        let digest = name.strip_prefix("appimage_extracted_")?;
        let status = fs::read_to_string("/proc/self/status").ok()?;
        let parent: u32 = status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:"))?
            .trim()
            .parse()
            .ok()?;
        if digest.len() != 32
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || fs::read_link(format!("/proc/{parent}/exe")).ok()? != path
        {
            return None;
        }
        true
    };
    Some(Target::AppImage { path, extracted })
}

fn mounted_appimage(appdir: &Path) -> bool {
    if fs::read_link("/proc/self/fd/1023").as_deref().ok() != Some(appdir) {
        return false;
    }
    fs::read_to_string("/proc/self/mountinfo").is_ok_and(|info| {
        info.lines().any(|line| {
            let Some((before, after)) = line.split_once(" - ") else {
                return false;
            };
            let mut fields = before.split_ascii_whitespace().skip(4);
            let mount_point = fields.next();
            let read_only = fields
                .next()
                .is_some_and(|options| options.split(',').any(|option| option == "ro"));
            mount_point.is_some_and(|value| mount_path(value) == appdir.as_os_str())
                && read_only
                && after.starts_with("fuse.squashfuse ")
        })
    })
}

pub(super) fn mount_path(value: &str) -> OsString {
    use std::os::unix::ffi::OsStringExt as _;

    let mut path = Vec::with_capacity(value.len());
    let mut bytes = value.as_bytes();
    while let Some((&byte, tail)) = bytes.split_first() {
        if let [
            b'\\',
            a @ b'0'..=b'3',
            b @ b'0'..=b'7',
            c @ b'0'..=b'7',
            rest @ ..,
        ] = bytes
        {
            path.push((a - b'0') * 64 + (b - b'0') * 8 + (c - b'0'));
            bytes = rest;
        } else {
            path.push(byte);
            bytes = tail;
        }
    }
    OsString::from_vec(path)
}

pub(super) fn install_appimage(
    target: &Path,
    download: &Path,
    asset: &UpdateAsset,
    current: &str,
) -> Result<(), InstallError> {
    let metadata = fs::symlink_metadata(target)?;
    let uid = current_uid()?;
    if uid == 0
        || !metadata.is_file()
        || metadata.mode() & 0o7000 != 0
        || metadata.mode() & 0o111 == 0
        || !appimage_header(File::open(download)?)
    {
        return Err(failure("the AppImage installation could not be verified"));
    }
    let parent = target
        .parent()
        .ok_or_else(|| failure("the AppImage location is invalid"))?;
    let staging = if metadata.uid() == uid {
        match tempfile::Builder::new()
            .prefix(".quickie-update-")
            .tempdir_in(parent)
        {
            Ok(staging) => Some(staging),
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied && trusted_root_path(parent) =>
            {
                None
            }
            Err(error) => return Err(error.into()),
        }
    } else if trusted_root_path(target) {
        None
    } else {
        return Err(failure(
            "the AppImage owner or installation directory could not be verified",
        ));
    };
    let working = tempfile::Builder::new()
        .prefix("quickie-update-")
        .tempdir()?;
    let candidate = working.path().join("Quickie.AppImage");
    fs::copy(download, &candidate)?;
    verify_asset(&candidate, asset)?;
    verify_image_version(target, working.path(), current)?;
    ensure_same_image(target, &metadata)?;
    fs::set_permissions(&candidate, Permissions::from_mode(0o700))?;
    verify_image_version(&candidate, working.path(), &asset.version().to_string())?;
    if let Some(staging) = staging {
        install_local_image(target, &candidate, &metadata, staging, |path| {
            verify_asset(path, asset)
        })
    } else {
        install_protected_image(target, &candidate, &metadata, asset, working.path())
    }
}

fn verify_image_version(path: &Path, temp: &Path, expected: &str) -> Result<(), InstallError> {
    let version = checked(
        system(path)
            .env("TMPDIR", temp)
            .args(["--appimage-extract-and-run", "--version"]),
    )?;
    if version.trim_end_matches('\n') != expected {
        return Err(failure("the AppImage reports a different Quickie version"));
    }
    Ok(())
}

pub(super) fn install_local_image(
    target: &Path,
    source: &Path,
    metadata: &Metadata,
    staging: tempfile::TempDir,
    verify: impl Fn(&Path) -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    let parent = target
        .parent()
        .ok_or_else(|| failure("the AppImage location is invalid"))?;
    let candidate = staging.path().join("next");
    let backup = staging.path().join("previous");
    fs::copy(source, &candidate)?;
    let file = File::open(&candidate)?;
    file.set_permissions(Permissions::from_mode(metadata.mode() & 0o777))?;
    file.sync_all()?;
    ensure_same_image(target, metadata)?;
    if fs::hard_link(target, &backup).is_err() {
        fs::copy(target, &backup)?;
        File::open(&backup)?.sync_all()?;
    }
    verify(&candidate)?;
    ensure_same_image(target, metadata)?;
    fs::rename(&candidate, target)?;
    let result = verify(target).and_then(|()| {
        File::open(parent)?.sync_all()?;
        Ok(())
    });
    if result.is_err()
        && let Err(error) = fs::rename(&backup, target)
    {
        let _ = staging.keep();
        return Err(rollback_error(&backup, &error));
    }
    result
}

#[cfg(target_os = "linux")]
fn install_protected_image(
    target: &Path,
    source: &Path,
    metadata: &Metadata,
    asset: &UpdateAsset,
    nonce: &Path,
) -> Result<(), InstallError> {
    use super::gio::GioFile;

    let directory = target
        .parent()
        .ok_or_else(|| failure("the AppImage location is invalid"))?
        .join(format!(
            ".{}",
            nonce
                .file_name()
                .ok_or_else(|| failure("the AppImage staging location is invalid"))?
                .to_string_lossy()
        ));
    let candidate = directory.join("next");
    let backup = directory.join("previous");
    let candidate_file = GioFile::admin(&candidate)?;
    let backup_file = GioFile::admin(&backup)?;
    let target_file = GioFile::admin(target)?;
    let directory_file = GioFile::admin(&directory)?;
    directory_file.create_directory()?;
    let mut keep_backup = false;
    let result = (|| {
        if !trusted_root_path(&directory) {
            return Err(failure(
                "the protected AppImage staging directory is unsafe",
            ));
        }
        GioFile::local(source)?.copy_to(&candidate_file, false)?;
        candidate_file.set_uint32(c"unix::gid", metadata.gid())?;
        candidate_file.set_uint32(c"unix::mode", 0o755)?;
        if !trusted_root_path(&candidate) {
            return Err(failure(
                "the staged AppImage permissions could not be verified",
            ));
        }
        verify_asset(&candidate, asset)?;
        ensure_same_image(target, metadata)?;
        target_file.copy_to(&backup_file, true)?;
        ensure_same_image(target, metadata)?;
        candidate_file.replace(&target_file)?;
        let result = (|| {
            verify_asset(target, asset)?;
            target_file.set_uint32(c"unix::mode", metadata.mode() & 0o777)?;
            if metadata.uid() != 0 {
                target_file.set_uint32(c"unix::uid", metadata.uid())?;
            }
            Ok(())
        })();
        if result.is_err()
            && let Err(error) = backup_file.replace(&target_file)
        {
            keep_backup = true;
            return Err(rollback_error(&backup, &error));
        }
        result
    })();
    if !keep_backup {
        let _ = candidate_file.delete();
        let _ = backup_file.delete();
        let _ = directory_file.delete();
    }
    result
}

#[cfg(not(target_os = "linux"))]
fn install_protected_image(
    _: &Path,
    _: &Path,
    _: &Metadata,
    _: &UpdateAsset,
    _: &Path,
) -> Result<(), InstallError> {
    Err(failure("protected AppImage updates require Linux"))
}

fn ensure_same_image(target: &Path, expected: &Metadata) -> Result<(), InstallError> {
    let current = fs::symlink_metadata(target)?;
    if !current.is_file()
        || current.dev() != expected.dev()
        || current.ino() != expected.ino()
        || current.len() != expected.len()
        || current.mtime() != expected.mtime()
        || current.mtime_nsec() != expected.mtime_nsec()
    {
        return Err(failure("the AppImage changed while preparing the update"));
    }
    Ok(())
}

fn rollback_error(backup: &Path, error: &impl fmt::Display) -> InstallError {
    InstallError::Failed(format!(
        "the AppImage could not be restored ({error}). The previous image is at {}",
        backup.display()
    ))
}

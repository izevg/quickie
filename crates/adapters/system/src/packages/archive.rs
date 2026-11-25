//! Extract checked extension archives before handing their directory to `WebKitGTK`.

#[cfg(test)]
pub(super) mod tests;

use std::fs::{self, File};
use std::io::{Cursor, Read, copy};
use std::path::{Component, Path, PathBuf};

use quickie_application::PackageError;
use zip::ZipArchive;

use super::failed;

const MAX_ENTRIES: usize = 16_384;
const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
const MANIFEST: &str = "manifest.json";

pub(super) fn keep_archive(root: &Path, hash: &str, bytes: &[u8]) -> Result<PathBuf, PackageError> {
    let path = root.join(hash);
    if complete(&path) {
        return Ok(path);
    }
    let staging = tempfile::tempdir_in(root).map_err(|error| failed(root, &error))?;
    extract(staging.path(), bytes)?;
    if !complete(staging.path()) {
        return Err(package_error("missing manifest.json"));
    }
    match fs::rename(staging.path(), &path) {
        Ok(()) => Ok(path),
        // Two installs of the same checked bytes can finish together.
        Err(_) if complete(&path) => Ok(path),
        Err(error) => Err(failed(&path, &error)),
    }
}

pub(super) fn remove_package(path: &Path) -> Result<(), PackageError> {
    let managed = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if managed {
        fs::remove_dir_all(path).map_err(|error| failed(path, &error))?;
    }
    Ok(())
}

fn complete(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.is_dir())
        && path
            .join(MANIFEST)
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file())
}

fn extract(root: &Path, bytes: &[u8]) -> Result<(), PackageError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(package_error)?;
    if archive.len() > MAX_ENTRIES {
        return Err(package_error("too many entries"));
    }
    let mut remaining = MAX_UNPACKED;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(package_error)?;
        let name = entry
            .enclosed_name()
            .filter(|name| {
                name.components()
                    .all(|part| matches!(part, Component::Normal(_)))
            })
            .ok_or_else(|| package_error("invalid entry path"))?;
        if entry.is_symlink() || entry.encrypted() {
            return Err(package_error("links and encrypted files are not supported"));
        }
        let path = root.join(name);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(|error| failed(&path, &error))?;
            continue;
        }
        if entry.size() > remaining {
            return Err(package_error("exceeds the unpacked size limit"));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| failed(parent, &error))?;
        }
        let mut destination = File::create_new(&path).map_err(|error| failed(&path, &error))?;
        let copied = copy(&mut entry.take(remaining + 1), &mut destination)
            .map_err(|error| failed(&path, &error))?;
        remaining = remaining
            .checked_sub(copied)
            .ok_or_else(|| package_error("exceeds the unpacked size limit"))?;
    }
    Ok(())
}

fn package_error(reason: impl std::fmt::Display) -> PackageError {
    PackageError(format!("Invalid extension ZIP: {reason}"))
}

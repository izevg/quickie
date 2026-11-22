use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use super::run_tool;

pub(super) struct MountedImage {
    root: Option<TempDir>,
    device: u64,
}

impl MountedImage {
    pub(super) fn attach(image: &Path) -> Result<Self, String> {
        if !image.is_absolute() || !fs::symlink_metadata(image).is_ok_and(|entry| entry.is_file()) {
            return Err("The verified Quickie disk image is not a regular file.".into());
        }
        let root = tempfile::Builder::new()
            .prefix("quickie-image-")
            .tempdir()
            .map_err(|error| {
                format!("Could not create a private disk image mount directory: {error}")
            })?;
        let device = fs::metadata(root.path())
            .map_err(|error| format!("Could not inspect the disk image mount directory: {error}"))?
            .dev();
        let attached = Self {
            root: Some(root),
            device,
        };
        let root = attached
            .root
            .as_ref()
            .ok_or("The mount directory is missing.")?;
        run_tool(
            Command::new("/usr/bin/hdiutil")
                .arg("attach")
                .arg(image)
                .args([
                    "-readonly",
                    "-nobrowse",
                    "-noautoopen",
                    "-noautoopenro",
                    "-noautoopenrw",
                    "-mountroot",
                ])
                .arg(root.path()),
            "Could not mount the Quickie disk image",
        )?;
        Ok(attached)
    }

    fn volumes(&self) -> Result<Vec<PathBuf>, String> {
        let Some(root) = &self.root else {
            return Ok(Vec::new());
        };
        let entries = fs::read_dir(root.path()).map_err(|error| {
            format!("Could not inspect the mounted Quickie disk image: {error}")
        })?;
        let mut mounted = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("Could not locate the disk image volume: {error}"))?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("Could not inspect the disk image volume: {error}"))?;
            if metadata.is_dir() && metadata.dev() != self.device {
                mounted.push(entry.path());
            }
        }
        Ok(mounted)
    }

    pub(super) fn volume(&self) -> Result<PathBuf, String> {
        let mut volumes = self.volumes()?;
        if volumes.len() != 1 {
            return Err("The Quickie disk image must have exactly one mounted volume.".into());
        }
        volumes
            .pop()
            .ok_or_else(|| "The Quickie disk image has no mounted volume.".into())
    }

    pub(super) fn detach(mut self) -> Result<(), String> {
        for volume in self.volumes()? {
            detach(&volume, false)?;
        }
        self.root.take();
        Ok(())
    }
}

impl Drop for MountedImage {
    fn drop(&mut self) {
        let detached = self.volumes().is_ok_and(|volumes| {
            let mut detached = true;
            for volume in volumes {
                detached &= detach(&volume, true).is_ok();
            }
            detached
        });
        // A still-mounted volume belongs to the disk image. Avoid recursively cleaning it.
        if !detached && let Some(root) = self.root.take() {
            let _ = root.keep();
        }
    }
}

fn detach(volume: &Path, force: bool) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/hdiutil");
    command.arg("detach").arg(volume);
    if force {
        command.arg("-force");
    }
    run_tool(&mut command, "Could not detach the Quickie disk image")?;
    Ok(())
}

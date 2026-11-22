//! `--dmg`: the signed app in a compressed disk image, the way a Mac app is handed out. The
//! image holds `Quickie.app` and a link to `/Applications` laid out on a custom Finder background.
//! The release workflow handles notarization with Apple's service.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{quiet, status};

/// Packs `app` into the disk image `dmg`, replacing one already there, and signs the image with
/// `identity` when there is one (an ad hoc build leaves it unsigned: Gatekeeper reads the app's
/// signature, not the image's).
pub fn create(root: &Path, app: &Path, dmg: &Path, identity: Option<&str>) -> Result<(), String> {
    let staging = staging_dir(dmg)?;
    let result = build(root, app, dmg, identity, &staging);
    let cleanup = fs::remove_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()));
    result.and(cleanup)
}

fn staging_dir(dmg: &Path) -> Result<PathBuf, String> {
    let output = Command::new("mktemp")
        .arg("-d")
        .arg(dmg.with_extension("staging.XXXXXX"))
        .output()
        .map_err(|e| format!("mktemp: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "mktemp: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let path = String::from_utf8(output.stdout).map_err(|e| format!("mktemp: {e}"))?;
    Ok(PathBuf::from(path.trim_end_matches('\n')))
}

fn build(
    root: &Path,
    app: &Path,
    dmg: &Path,
    identity: Option<&str>,
    staging: &Path,
) -> Result<(), String> {
    let source = root.join("docs/assets/installer-background.png");
    let standard = staging.join("installer-background.png");
    let retina = staging.join("installer-background@2x.png");
    // dmgbuild combines these into a Retina-aware TIFF using their filename suffixes.
    fs::copy(&source, &retina).map_err(|e| format!("{}: {e}", source.display()))?;
    quiet(
        "sips",
        &[
            "--resampleHeightWidth",
            "512",
            "768",
            &source.to_string_lossy(),
            "--out",
            &standard.to_string_lossy(),
        ],
    )?;
    // dmgbuild writes Finder's metadata headlessly and uses ditto to preserve the app signature.
    let image = staging.join("Quickie.dmg");
    let image_path = image.to_string_lossy();
    status(
        "dmgbuild",
        &[
            "--settings",
            &root
                .join("tools/packaging/macos/installer.py")
                .to_string_lossy(),
            "-D",
            &format!("app={}", app.display()),
            "-D",
            &format!("background={}", standard.display()),
            "Quickie",
            &image_path,
        ],
    )?;
    if let Some(identity) = identity {
        status(
            "codesign",
            &["--sign", identity, "--timestamp", &image_path],
        )?;
        status("codesign", &["--verify", "--strict", &image_path])?;
    }
    fs::rename(&image, dmg).map_err(|e| format!("{}: {e}", dmg.display()))
}

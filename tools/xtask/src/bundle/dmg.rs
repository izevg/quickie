//! `--dmg`: the signed app in a compressed disk image, the way a Mac app is handed out. The
//! image holds `Quickie.app` and a link to `/Applications` to drag it onto. Notarizing the image
//! needs Apple's service and the team's key, so the release workflow does that, not this tool.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use super::{quiet, status};

/// Packs `app` into the disk image `dmg`, replacing one already there, and signs the image with
/// `identity` when there is one (an ad hoc build leaves it unsigned: Gatekeeper reads the app's
/// signature, not the image's).
pub fn create(app: &Path, dmg: &Path, identity: Option<&str>) -> Result<(), String> {
    let staging = dmg.with_extension("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    }
    fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let name = app.file_name().ok_or("the app has no file name")?;
    // `ditto` keeps the signature, extended attributes, and links exactly as they are.
    quiet(
        "ditto",
        &[
            &app.to_string_lossy(),
            &staging.join(name).to_string_lossy(),
        ],
    )?;
    let applications = staging.join("Applications");
    symlink("/Applications", &applications)
        .map_err(|e| format!("{}: {e}", applications.display()))?;
    if dmg.exists() {
        fs::remove_file(dmg).map_err(|e| format!("{}: {e}", dmg.display()))?;
    }
    status(
        "hdiutil",
        &[
            "create",
            "-quiet",
            "-volname",
            "Quickie",
            "-fs",
            "HFS+",
            "-format",
            "UDZO",
            "-srcfolder",
            &staging.to_string_lossy(),
            &dmg.to_string_lossy(),
        ],
    )?;
    fs::remove_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    if let Some(identity) = identity {
        status(
            "codesign",
            &["--sign", identity, "--timestamp", &dmg.to_string_lossy()],
        )?;
    }
    Ok(())
}

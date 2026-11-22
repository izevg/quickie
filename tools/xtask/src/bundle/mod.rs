//! `cargo xtask bundle [--release] [--bundle-id <id>] [--version <X.Y.Z>] [--sign <identity>]
//! [--webkit <dir>] [--dmg]`: the app as a real macOS app, `target/<profile>/Quickie.app`, that
//! `LaunchServices` can register, open links with, and make the default browser; with `--dmg`
//! also `target/<profile>/Quickie-<version>.dmg` ([`dmg`]), what a release hands out.
//!
//! ```text
//! Quickie.app/Contents/
//!   Info.plist           plist.rs
//!   PkgInfo
//!   MacOS/quickie        the shell binary, built with Cargo
//!   Resources/Quickie.icns   from docs/assets/app-icon.png with sips and iconutil
//!   Frameworks/          with --webkit: our WebKit fork (webkit.rs)
//! ```
//!
//! The version is the latest release tag's unless `--version` names it (a release builds before
//! its tag exists). Without `--sign` the bundle is signed ad hoc (`codesign -s -`), for this
//! Mac only. `--sign` takes a Developer ID Application identity from the keychain and signs
//! under the hardened runtime with a secure timestamp and [`plist::entitlements`], ready for
//! notarization.

mod dmg;
mod plist;
mod webkit;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use plist::{
    Bundle, DEFAULT_ID, SYSTEM_MINIMUM, entitlements, info_plist, valid_id, version_from_tag,
};

/// Every image an `.iconset` holds: the pixel size and its file name.
const ICON_SIZES: [(u32, &str); 10] = [
    (16, "icon_16x16"),
    (32, "icon_16x16@2x"),
    (32, "icon_32x32"),
    (64, "icon_32x32@2x"),
    (128, "icon_128x128"),
    (256, "icon_128x128@2x"),
    (256, "icon_256x256"),
    (512, "icon_256x256@2x"),
    (512, "icon_512x512"),
    (1024, "icon_512x512@2x"),
];

/// What `cargo xtask bundle` was asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub release: bool,
    pub id: String,
    /// `MAJOR.MINOR.PATCH`; the latest release tag's when `None`.
    pub version: Option<String>,
    /// A Developer ID Application identity; ad hoc when `None`.
    pub sign: Option<String>,
    /// Our `WebKit` fork's build to carry (`.build/webkit-frameworks`); the system's when `None`.
    pub webkit: Option<PathBuf>,
    pub dmg: bool,
}

/// Reads the arguments after `bundle`.
pub fn options(args: &[&str]) -> Result<Options, String> {
    let mut options = Options {
        release: false,
        id: DEFAULT_ID.to_owned(),
        version: None,
        sign: None,
        webkit: None,
        dmg: false,
    };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--release" => options.release = true,
            "--bundle-id" => {
                let id = args.next().ok_or("--bundle-id needs a value")?;
                if !valid_id(id) {
                    return Err(format!("not a bundle id: {id}"));
                }
                (*id).clone_into(&mut options.id);
            }
            "--version" => {
                let version = args.next().ok_or("--version needs a value")?;
                if version_from_tag(Some(&format!("v{version}"))) != *version {
                    return Err(format!("not a MAJOR.MINOR.PATCH version: {version}"));
                }
                options.version = Some((*version).to_owned());
            }
            "--sign" => {
                let identity = args.next().filter(|identity| !identity.is_empty());
                options.sign = Some((*identity.ok_or("--sign needs an identity")?).to_owned());
            }
            "--webkit" => {
                let dir = args.next().filter(|dir| !dir.is_empty());
                options.webkit = Some(PathBuf::from(dir.ok_or("--webkit needs a directory")?));
            }
            "--dmg" => options.dmg = true,
            other => return Err(format!("unknown bundle option: {other}")),
        }
    }
    Ok(options)
}

/// Builds the shell, lays out and signs `Quickie.app`, packs the disk image when asked, and
/// returns the paths it made.
pub fn run(
    options: &Options,
    root: &Path,
    target: &Path,
    cargo: &str,
) -> Result<Vec<PathBuf>, String> {
    let profile = if options.release { "release" } else { "debug" };
    let mut build = vec!["build", "--package", "quickie"];
    if options.release {
        build.push("--release");
    }
    status(cargo, &build)?;

    let out = target.join(profile);
    let app = out.join("Quickie.app");
    if app.exists() {
        fs::remove_dir_all(&app).map_err(|e| format!("{}: {e}", app.display()))?;
    }
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    copy(&out.join("quickie"), &macos.join("quickie"))?;
    let fork = options.webkit.as_ref().map(|dir| root.join(dir));
    let bundle = Bundle {
        id: options.id.clone(),
        version: options
            .version
            .clone()
            .unwrap_or_else(|| version_from_tag(latest_tag().as_deref())),
        minimum: match &fork {
            Some(dir) => webkit::minimum(dir)?,
            None => SYSTEM_MINIMUM.to_owned(),
        },
    };
    write(&contents.join("Info.plist"), &info_plist(&bundle))?;
    write(&contents.join("PkgInfo"), "APPL????")?;
    icon(
        &root.join("docs/assets/app-icon.png"),
        &out.join("Quickie.iconset"),
        &resources.join("Quickie.icns"),
    )?;
    if let Some(dir) = &fork {
        webkit::embed(dir, &app)?;
        webkit::sign(&app, options.sign.as_deref(), &out)?;
    }
    sign(&app, options.sign.as_deref(), &out)?;
    let mut made = vec![app.clone()];
    if options.dmg {
        let image = out.join(format!("Quickie-{}.dmg", bundle.version));
        dmg::create(&app, &image, options.sign.as_deref())?;
        made.push(image);
    }
    Ok(made)
}

/// Signs `app` ad hoc, or with `identity` under the hardened runtime; `out` holds the
/// entitlements file the signature reads. The code inside it is signed already
/// ([`webkit::sign`]): a deep signature would drop the helpers' own entitlements.
fn sign(app: &Path, identity: Option<&str>, out: &Path) -> Result<(), String> {
    let app = app.to_string_lossy();
    let Some(identity) = identity else {
        return status(
            "codesign",
            &["--sign", "-", "--force", "--timestamp=none", &app],
        );
    };
    let file = out.join("Quickie.entitlements");
    write(&file, &entitlements())?;
    status(
        "codesign",
        &[
            "--sign",
            identity,
            "--force",
            "--options",
            "runtime",
            "--timestamp",
            "--entitlements",
            &file.to_string_lossy(),
            &app,
        ],
    )?;
    status("codesign", &["--verify", "--strict", "--verbose=2", &app])
}

/// The latest release tag reachable from `HEAD`, if any.
fn latest_tag() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `logo` scaled to every size of an iconset, packed into `icns`. The iconset is scratch.
fn icon(logo: &Path, iconset: &Path, icns: &Path) -> Result<(), String> {
    if iconset.exists() {
        fs::remove_dir_all(iconset).map_err(|e| format!("{}: {e}", iconset.display()))?;
    }
    fs::create_dir_all(iconset).map_err(|e| format!("{}: {e}", iconset.display()))?;
    let logo = logo.to_string_lossy();
    for (size, name) in ICON_SIZES {
        let size = size.to_string();
        let png = iconset.join(format!("{name}.png"));
        let png = png.to_string_lossy();
        quiet(
            "sips",
            &[
                "-s", "format", "png", "-z", &size, &size, &logo, "--out", &png,
            ],
        )?;
    }
    let (set, out) = (iconset.to_string_lossy(), icns.to_string_lossy());
    status("iconutil", &["--convert", "icns", "--output", &out, &set])?;
    fs::remove_dir_all(iconset).map_err(|e| format!("{}: {e}", iconset.display()))
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("{} → {}: {e}", from.display(), to.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Runs `program`, its output on the terminal.
fn status(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed: {status}", args.join(" ")))
    }
}

/// Runs `program`, its output shown only when it fails.
fn quiet(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ID, ICON_SIZES, Options, options};

    #[test]
    fn a_plain_bundle_is_a_debug_build_of_the_app() {
        assert_eq!(
            options(&[]),
            Ok(Options {
                release: false,
                id: DEFAULT_ID.to_owned(),
                version: None,
                sign: None,
                webkit: None,
                dmg: false,
            })
        );
    }

    #[test]
    fn a_test_bundle_registers_under_its_own_id() {
        assert_eq!(
            options(&["--bundle-id", "dev.quickie.test", "--release"]),
            Ok(Options {
                release: true,
                id: "dev.quickie.test".to_owned(),
                version: None,
                sign: None,
                webkit: None,
                dmg: false,
            })
        );
    }

    #[test]
    fn a_release_names_its_version_identity_and_image() {
        assert_eq!(
            options(&[
                "--release",
                "--version",
                "0.1.0",
                "--sign",
                "Developer ID Application: Someone (TEAM123456)",
                "--webkit",
                ".build/webkit-frameworks",
                "--dmg",
            ]),
            Ok(Options {
                release: true,
                id: DEFAULT_ID.to_owned(),
                version: Some("0.1.0".to_owned()),
                sign: Some("Developer ID Application: Someone (TEAM123456)".to_owned()),
                webkit: Some(".build/webkit-frameworks".into()),
                dmg: true,
            })
        );
    }

    #[test]
    fn bad_options_are_refused() {
        assert!(options(&["--bundle-id"]).is_err());
        assert!(options(&["--bundle-id", "quickie"]).is_err());
        assert!(options(&["--debug"]).is_err());
        assert!(options(&["--version"]).is_err());
        assert!(options(&["--version", "v0.1.0"]).is_err());
        assert!(options(&["--version", "0.1"]).is_err());
        assert!(options(&["--webkit"]).is_err());
        assert!(options(&["--webkit", ""]).is_err());
        assert!(options(&["--sign"]).is_err());
        assert!(options(&["--sign", ""]).is_err());
    }

    #[test]
    fn the_iconset_has_every_size_once() {
        let mut names: Vec<&str> = ICON_SIZES.iter().map(|&(_, name)| name).collect();
        names.dedup();
        assert_eq!(names.len(), 10);
        for (size, name) in ICON_SIZES {
            let points: u32 = name
                .trim_start_matches("icon_")
                .split('x')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap();
            let scale = if name.ends_with("@2x") { 2 } else { 1 };
            assert_eq!(size, points * scale, "{name}");
        }
    }
}

//! `--webkit <dir>`: the app on our `WebKit` fork instead of the system's. `<dir>` is the
//! fork's build as `engine/webkit/scripts/fetch` unpacks it (`.build/webkit-frameworks`).
//!
//! ```text
//! Quickie.app/Contents/Frameworks/
//!   WebKit.framework/Versions/A/XPCServices/*.xpc   the helper processes, moved in from <dir>
//!   WebCore, JavaScriptCore, WebKitLegacy, WebGPU, WebInspectorUI, _WebKit_SwiftUI .framework
//!   libwebrtc.dylib, libANGLE-shared.dylib
//!   libWebKitSwift.dylib (older builds) or WebKit.framework/Versions/A/Frameworks/libWebKitSwift.dylib
//! ```
//!
//! The build links everything by the system's install names (`/System/Library/...`). Here each
//! becomes `@rpath/...`, and every binary gets the `LC_RPATH` that leads from it to
//! `Contents/Frameworks`: the app's binary, each framework, each helper. The app's binary also
//! carries `DYLD_FRAMEWORK_PATH` in its own `LC_DYLD_ENVIRONMENT` (`crates/shell/build.rs`), for
//! the system frameworks that link `WebKit` by its absolute path. Headers, modules and the
//! `jsc` helper are left out; so is `libWebCoreTestSupport`, which nothing in the app loads.
//!
//! Signing goes from the inside out, each helper with its own entitlements (the content
//! process's `allow-jit` among them) except `get-task-allow`, which notarization refuses.

use std::fs;
use std::path::{Path, PathBuf};

use super::{quiet, status};

/// The frameworks of the fork the app carries.
const FRAMEWORKS: [&str; 7] = [
    "WebKit",
    "WebCore",
    "JavaScriptCore",
    "WebKitLegacy",
    "WebGPU",
    "WebInspectorUI",
    "_WebKit_SwiftUI",
];
/// Its libraries the app carries.
const DYLIBS: [&str; 2] = ["libwebrtc.dylib", "libANGLE-shared.dylib"];
const SWIFT_DYLIB: &str = "libWebKitSwift.dylib";
const NESTED_SWIFT_DYLIB: &str = "WebKit.framework/Versions/A/Frameworks/libWebKitSwift.dylib";
/// What a framework holds for building against it, not for running it.
const LEFT_OUT: [&str; 4] = ["Headers", "PrivateHeaders", "Modules", "Helpers"];
/// From a helper's binary (`<x>.xpc/Contents/MacOS/<x>`) up to `Contents/Frameworks`:
/// `MacOS`, `Contents`, the `.xpc`, `XPCServices`, `A`, `Versions`, `WebKit.framework`.
const HELPER_RPATH: &str = "@loader_path/../../../../../../..";
/// Debugging the helpers; notarization refuses a binary that allows it.
const DEBUGGING: &str = "com.apple.security.get-task-allow";

/// The install name the app's copy of `dependency` has, when it is one of the fork's.
#[must_use]
fn rewritten(dependency: &str, swift_dylib: &str) -> Option<String> {
    if dependency
        .strip_suffix(SWIFT_DYLIB)
        .is_some_and(|head| head.ends_with('/'))
    {
        let name = format!("@rpath/{swift_dylib}");
        return (dependency != name).then_some(name);
    }
    if dependency.starts_with("@rpath/") {
        return None;
    }
    let framework = FRAMEWORKS.iter().find_map(|name| {
        let tail = format!("{name}.framework/Versions/A/{name}");
        dependency
            .strip_suffix(&tail)
            .filter(|head| head.ends_with('/'))
            .map(|_| format!("@rpath/{tail}"))
    });
    framework.or_else(|| {
        DYLIBS
            .iter()
            .find(|lib| {
                dependency
                    .strip_suffix(*lib)
                    .is_some_and(|head| head.ends_with('/'))
            })
            .map(|lib| format!("@rpath/{lib}"))
    })
}

/// The oldest macOS the fork's build in `source` runs on: its `WebKit`'s deployment target.
pub fn minimum(source: &Path) -> Result<String, String> {
    let binary = source.join("WebKit.framework/Versions/A/WebKit");
    let shown = output("vtool", &["-show-build", &binary.to_string_lossy()])?;
    deployment_target(&shown).ok_or_else(|| {
        format!(
            "{}: no minimum macOS in its build version",
            binary.display()
        )
    })
}

/// The `minos` of `vtool -show-build`'s output.
fn deployment_target(shown: &str) -> Option<String> {
    shown.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("minos"))
            .then(|| words.next())
            .flatten()
            .map(str::to_owned)
    })
}

/// Copies the fork's build from `source` into `app`, renamed and with the paths to find itself.
pub fn embed(source: &Path, app: &Path) -> Result<(), String> {
    if !source.join("WebKit.framework").is_dir() {
        return Err(format!(
            "no WebKit.framework in {}; run engine/webkit/scripts/fetch",
            source.display()
        ));
    }
    let frameworks = app.join("Contents/Frameworks");
    fs::create_dir_all(&frameworks).map_err(|e| format!("{}: {e}", frameworks.display()))?;
    for name in FRAMEWORKS {
        let bundle = format!("{name}.framework");
        ditto(&source.join(&bundle), &frameworks.join(&bundle))?;
        leave_out(&frameworks.join(&bundle))?;
    }
    for lib in DYLIBS {
        ditto(&source.join(lib), &frameworks.join(lib))?;
    }
    let swift_dylib = swift_dylib_path(source);
    if swift_dylib == SWIFT_DYLIB {
        ditto(&source.join(swift_dylib), &frameworks.join(swift_dylib))?;
    }
    let services = helpers_dir(app);
    fs::remove_dir_all(&services).map_err(|e| format!("{}: {e}", services.display()))?;
    fs::create_dir_all(&services).map_err(|e| format!("{}: {e}", services.display()))?;
    for helper in helpers(source)? {
        let name = helper.file_name().ok_or("a helper without a name")?;
        ditto(&helper, &services.join(name))?;
    }

    for name in FRAMEWORKS {
        let binary = frameworks.join(format!("{name}.framework/Versions/A/{name}"));
        relink(
            &binary,
            Some(&format!("@rpath/{name}.framework/Versions/A/{name}")),
            swift_dylib,
        )?;
        add_rpath(&binary, "@loader_path/../../..")?;
    }
    for lib in DYLIBS.into_iter().chain([swift_dylib]) {
        let binary = frameworks.join(lib);
        relink(&binary, Some(&format!("@rpath/{lib}")), swift_dylib)?;
        let rpath = if lib == NESTED_SWIFT_DYLIB {
            "@loader_path/../../../.."
        } else {
            "@loader_path"
        };
        add_rpath(&binary, rpath)?;
    }
    for helper in helpers(&services)? {
        for binary in executables(&helper.join("Contents/MacOS"))? {
            relink(&binary, None, swift_dylib)?;
            add_rpath(&binary, HELPER_RPATH)?;
        }
    }
    let main = app.join("Contents/MacOS/quickie");
    relink(&main, None, swift_dylib)?;
    add_rpath(&main, "@executable_path/../Frameworks")
}

/// Signs what [`embed`] put into `app`, from the inside out; `out` holds the helpers'
/// entitlements. Ad hoc without `identity`, else under the hardened runtime.
pub fn sign(app: &Path, identity: Option<&str>, out: &Path) -> Result<(), String> {
    let frameworks = app.join("Contents/Frameworks");
    for helper in helpers(&helpers_dir(app))? {
        let entitlements = helper_entitlements(&helper, identity.is_some(), out)?;
        codesign(&helper, identity, entitlements.as_deref())?;
    }
    for lib in DYLIBS.into_iter().chain([swift_dylib_path(&frameworks)]) {
        codesign(&frameworks.join(lib), identity, None)?;
    }
    for name in FRAMEWORKS {
        codesign(
            &frameworks.join(format!("{name}.framework")),
            identity,
            None,
        )?;
    }
    Ok(())
}

fn swift_dylib_path(frameworks: &Path) -> &'static str {
    if frameworks.join(NESTED_SWIFT_DYLIB).is_file() {
        NESTED_SWIFT_DYLIB
    } else {
        SWIFT_DYLIB
    }
}

fn helpers_dir(app: &Path) -> PathBuf {
    app.join("Contents/Frameworks/WebKit.framework/Versions/A/XPCServices")
}

/// The `.xpc` bundles directly in `dir`, in name order.
fn helpers(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found: Vec<PathBuf> = entries(dir)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "xpc"))
        .collect();
    found.sort();
    Ok(found)
}

fn executables(dir: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|path| path.is_file())
        .collect())
}

fn entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|e| format!("{}: {e}", dir.display()))
        })
        .collect()
}

/// Removes [`LEFT_OUT`] from a framework, its versioned directory and the links to it alike.
fn leave_out(framework: &Path) -> Result<(), String> {
    for dir in [framework.to_path_buf(), framework.join("Versions/A")] {
        for name in LEFT_OUT {
            let path = dir.join(name);
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            let removed = if meta.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            removed.map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Gives `binary` the install name `id` (a library's own), and the app's names for every fork
/// binary it links.
fn relink(binary: &Path, id: Option<&str>, swift_dylib: &str) -> Result<(), String> {
    let path = binary.to_string_lossy();
    let listed = output("otool", &["-L", &path])?;
    let mut args: Vec<String> = Vec::new();
    if let Some(id) = id {
        args.extend(["-id".to_owned(), id.to_owned()]);
    }
    for dependency in listed
        .lines()
        .skip(1)
        .filter_map(|line| line.split(" (").next())
    {
        let dependency = dependency.trim();
        if let Some(new) = rewritten(dependency, swift_dylib) {
            args.extend(["-change".to_owned(), dependency.to_owned(), new]);
        }
    }
    if args.is_empty() {
        return Ok(());
    }
    args.push(path.into_owned());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    quiet("install_name_tool", &args)
}

fn add_rpath(binary: &Path, rpath: &str) -> Result<(), String> {
    quiet(
        "install_name_tool",
        &["-add_rpath", rpath, &binary.to_string_lossy()],
    )
}

/// A helper's own entitlements, written to a file in `out` for its new signature; without
/// `get-task-allow` for a release. `None` when it has none.
fn helper_entitlements(
    helper: &Path,
    release: bool,
    out: &Path,
) -> Result<Option<PathBuf>, String> {
    let xml = output(
        "codesign",
        &[
            "-d",
            "--entitlements",
            "-",
            "--xml",
            &helper.to_string_lossy(),
        ],
    )?;
    if xml.trim().is_empty() {
        return Ok(None);
    }
    let name = helper.file_name().ok_or("a helper without a name")?;
    let file = out.join(format!("{}.entitlements", name.to_string_lossy()));
    fs::write(&file, xml).map_err(|e| format!("{}: {e}", file.display()))?;
    let path = file.to_string_lossy();
    if release && output("plutil", &["-extract", DEBUGGING, "raw", &path]).is_ok() {
        quiet("plutil", &["-remove", DEBUGGING, &path])?;
    }
    Ok(Some(file))
}

fn codesign(
    path: &Path,
    identity: Option<&str>,
    entitlements: Option<&Path>,
) -> Result<(), String> {
    let path = path.to_string_lossy();
    let entitlements = entitlements.map(|file| file.to_string_lossy().into_owned());
    let mut args = vec!["--sign", identity.unwrap_or("-"), "--force"];
    if identity.is_some() {
        args.extend(["--options", "runtime", "--timestamp"]);
    } else {
        args.push("--timestamp=none");
    }
    if let Some(file) = &entitlements {
        args.extend(["--entitlements", file]);
    }
    args.push(&path);
    quiet("codesign", &args)
}

/// Copies a file or a bundle, links kept as links.
fn ditto(from: &Path, to: &Path) -> Result<(), String> {
    status("ditto", &[&from.to_string_lossy(), &to.to_string_lossy()])
}

/// Runs `program` and returns its standard output.
fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
    use super::{NESTED_SWIFT_DYLIB, SWIFT_DYLIB, deployment_target, rewritten};

    #[test]
    fn the_minimum_macos_is_the_builds_deployment_target() {
        let shown = "/x/WebKit (architecture arm64):\nLoad command 9\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform MACOS\n    minos 26.5\n      sdk 26.5\n   ntools 1\n";

        assert_eq!(deployment_target(shown).as_deref(), Some("26.5"));
        assert_eq!(deployment_target("platform MACOS\n sdk 26.5\n"), None);
    }

    #[test]
    fn the_system_names_of_the_forks_frameworks_become_the_bundles() {
        let cases = [
            (
                "/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit",
                "@rpath/WebKit.framework/Versions/A/WebKit",
            ),
            (
                "/System/Library/Frameworks/WebKit.framework/Versions/A/Frameworks/WebCore.framework/Versions/A/WebCore",
                "@rpath/WebCore.framework/Versions/A/WebCore",
            ),
            (
                "/System/Library/PrivateFrameworks/WebGPU.framework/Versions/A/WebGPU",
                "@rpath/WebGPU.framework/Versions/A/WebGPU",
            ),
            (
                "/System/Library/Frameworks/_WebKit_SwiftUI.framework/Versions/A/_WebKit_SwiftUI",
                "@rpath/_WebKit_SwiftUI.framework/Versions/A/_WebKit_SwiftUI",
            ),
        ];
        for (system, ours) in cases {
            assert_eq!(
                rewritten(system, SWIFT_DYLIB).as_deref(),
                Some(ours),
                "{system}"
            );
        }
    }

    #[test]
    fn its_libraries_are_found_by_name() {
        assert_eq!(
            rewritten("@loader_path/../../../libwebrtc.dylib", SWIFT_DYLIB).as_deref(),
            Some("@rpath/libwebrtc.dylib")
        );
        assert_eq!(
            rewritten("/System/Library/Frameworks/WebKit.framework/Versions/A/Frameworks/libWebKitSwift.dylib", SWIFT_DYLIB)
                .as_deref(),
            Some("@rpath/libWebKitSwift.dylib")
        );
        let nested = format!("@rpath/{NESTED_SWIFT_DYLIB}");
        assert_eq!(
            rewritten("@rpath/libWebKitSwift.dylib", NESTED_SWIFT_DYLIB).as_deref(),
            Some(nested.as_str())
        );
        assert_eq!(rewritten(&nested, NESTED_SWIFT_DYLIB), None);
    }

    #[test]
    fn everything_else_keeps_its_name() {
        for other in [
            "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit",
            "/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI",
            "/System/Library/PrivateFrameworks/MyWebKit.framework/Versions/A/MyWebKit",
            "/usr/lib/libmywebrtc.dylib",
            "/usr/lib/libSystem.B.dylib",
            "@rpath/WebKit.framework/Versions/A/WebKit",
            "@rpath/libWebCoreTestSupport.dylib",
        ] {
            assert_eq!(rewritten(other, SWIFT_DYLIB), None, "{other}");
        }
    }
}

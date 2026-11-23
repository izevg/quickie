use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use super::bundle::{
    BundleIdentity, code_hash, preserve_quarantine, validated_executable, version,
};
use super::privileged::{BundleProof, replace_disposable};
use super::replacement::Replacement;
use super::{APP_NAME, run_tool, stage_package};

#[test]
fn stable_release_version_uses_numeric_order() {
    assert!(version("0.10.0", 3).unwrap() > version("0.9.9", 3).unwrap());
    for invalid in ["0.02.0", "v0.2.0", "0.2", "0.2.0-rc.1", "0.2.+1"] {
        assert!(version(invalid, 3).is_err(), "{invalid}");
    }
}

#[test]
fn macos_minimum_version_includes_optional_patch() {
    assert_eq!(version("26.5", 2).unwrap(), [26, 5, 0]);
    assert!(version("26.5.2", 2).unwrap() > version("26.5.1", 2).unwrap());
    assert!(version("26.5.2.1", 2).is_err());
}

#[test]
fn candidate_bundle_cannot_redirect_its_executable() {
    let fixture = tempfile::tempdir().unwrap();
    let bundle = fixture.path().join("Quickie.app");
    let macos = bundle.join("Contents/MacOS");
    fs::create_dir_all(&macos).unwrap();
    let executable = macos.join("quickie");
    let outside = fixture.path().join("outside");
    fs::write(&outside, b"outside").unwrap();
    symlink(&outside, &executable).unwrap();

    assert!(validated_executable(&bundle).is_err());

    fs::remove_file(&executable).unwrap();
    fs::write(&executable, b"inside").unwrap();
    assert_eq!(validated_executable(&bundle).unwrap(), executable);
}

#[test]
fn staging_locks_the_installation_until_dropped() {
    let fixture = tempfile::tempdir().unwrap();
    let installed = fixture.path().join("Quickie.app");
    write_bundle(&installed, "0.2.0");

    let staging = Replacement::new(installed.clone()).unwrap();
    assert!(Replacement::new(installed.clone()).is_err());
    drop(staging);

    assert!(Replacement::new(installed).is_ok());
}

#[test]
fn replacement_checks_that_the_running_bundle_still_occupies_its_path() {
    let fixture = tempfile::tempdir().unwrap();
    let installed = fixture.path().join("Quickie.app");
    write_bundle(&installed, "0.2.0");
    let mut staging = Replacement::new(installed.clone()).unwrap();
    write_bundle(staging.staged(), "0.3.0");
    staging.seal().unwrap();

    fs::rename(&installed, fixture.path().join("Original.app")).unwrap();
    write_bundle(&installed, "0.2.0");

    assert!(staging.recheck().is_err());
}

#[test]
fn replacement_installs_the_verified_version() {
    let fixture = tempfile::tempdir().unwrap();
    let installed = fixture.path().join("Quickie.app");
    write_bundle(&installed, "0.2.0");
    let mut staging = Replacement::new(installed.clone()).unwrap();
    write_bundle(staging.staged(), "0.3.0");
    preserve_quarantine(staging.staged()).unwrap();
    staging.seal().unwrap();
    let (sender, receiver) = mpsc::channel();

    staging.apply("0.3.0", move |result| sender.send(result).unwrap());

    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        installed
    );
    BundleIdentity::read(&installed)
        .unwrap()
        .verify_version("0.3.0")
        .unwrap();
    assert!(
        Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&installed)
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn replacement_rolls_back_when_the_installed_bundle_fails_verification() {
    let fixture = tempfile::tempdir().unwrap();
    let installed = fixture.path().join("Quickie.app");
    write_bundle(&installed, "0.2.0");
    let mut staging = Replacement::new(installed.clone()).unwrap();
    write_bundle(staging.staged(), "0.3.1");
    staging.seal().unwrap();
    let (sender, receiver) = mpsc::channel();

    staging.apply("0.3.0", move |result| sender.send(result).unwrap());

    assert!(
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_err()
    );
    BundleIdentity::read(&installed)
        .unwrap()
        .verify_version("0.2.0")
        .unwrap();
}

#[test]
#[ignore = "requires the macOS disk-image and code-signing tools"]
fn signed_disk_image_replaces_a_disposable_installation() {
    let fixture = tempfile::tempdir().unwrap();
    let installed = fixture.path().join(APP_NAME);
    let payload = fixture.path().join("payload");
    let candidate = payload.join(APP_NAME);
    write_signed_bundle(&installed, "0.2.0");
    write_signed_bundle(&candidate, "0.3.0");
    let image = fixture.path().join("Quickie-0.3.0.dmg");
    run_tool(
        Command::new("/usr/bin/hdiutil")
            .args([
                "create",
                "-format",
                "UDZO",
                "-volname",
                "Quickie",
                "-srcfolder",
            ])
            .arg(&payload)
            .arg(&image),
        "Could not create the test disk image",
    )
    .unwrap();

    let staged = stage_package(&image, installed.clone(), "0.3.0");
    let (replacement, signature) = staged.unwrap();
    let (sender, receiver) = mpsc::channel();
    replacement.apply("0.3.0", move |result| sender.send(result).unwrap());

    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        installed
    );
    super::bundle::verify(&installed, "0.3.0", &signature).unwrap();
}

#[test]
#[ignore = "requires the macOS scripting and code-signing tools"]
fn protected_replacement_program_verifies_and_swaps_disposable_bundles() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let installed = root.join("Quickie.app");
    let staged = root.join("Staged Quickie's.app");
    write_signed_bundle(&installed, "0.2.0");
    write_signed_bundle(&staged, "0.3.0");
    let staged_executable = staged.join("Contents/MacOS/quickie");
    fs::set_permissions(&staged_executable, fs::Permissions::from_mode(0o6777)).unwrap();
    run_tool(
        Command::new("/bin/chmod")
            .args(["+a", "everyone allow write"])
            .arg(&staged_executable),
        "Could not set the test file ACL",
    )
    .unwrap();
    preserve_quarantine(&staged).unwrap();
    let stage_owner = fs::metadata(&root).unwrap().uid();
    let metadata = fs::symlink_metadata(&installed).unwrap();
    let (owner, group) = (metadata.uid(), metadata.gid());
    let original = code_hash(&installed).unwrap();
    let candidate = code_hash(&staged).unwrap();

    let mismatch = BundleProof {
        original: original.clone(),
        candidate: "0".repeat(40),
        owner,
        group,
    };
    assert!(
        replace_disposable(&installed, &staged, &root, stage_owner, "0.3.0", &mismatch).is_err()
    );
    assert_eq!(code_hash(&installed).unwrap(), original);

    let wrong_owner = BundleProof {
        original: original.clone(),
        candidate: candidate.clone(),
        owner: owner ^ 1,
        group,
    };
    assert!(
        replace_disposable(
            &installed,
            &staged,
            &root,
            stage_owner,
            "0.3.0",
            &wrong_owner
        )
        .is_err()
    );
    assert_eq!(code_hash(&installed).unwrap(), original);

    let verified = BundleProof {
        original,
        candidate: candidate.clone(),
        owner,
        group,
    };
    replace_disposable(&installed, &staged, &root, stage_owner, "0.3.0", &verified).unwrap();

    assert_eq!(code_hash(&installed).unwrap(), candidate);
    let metadata = fs::symlink_metadata(&installed).unwrap();
    assert_eq!((metadata.uid(), metadata.gid()), (owner, group));
    let installed_executable = installed.join("Contents/MacOS/quickie");
    assert_eq!(
        fs::symlink_metadata(&installed_executable).unwrap().mode() & 0o7777,
        0o755
    );
    let permissions = run_tool(
        Command::new("/bin/ls")
            .arg("-le")
            .arg(&installed_executable),
        "Could not inspect the installed executable ACL",
    )
    .unwrap();
    assert!(!String::from_utf8_lossy(&permissions.stdout).contains("everyone allow write"));
    BundleIdentity::read(&installed)
        .unwrap()
        .verify_version("0.3.0")
        .unwrap();
    assert!(
        Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&installed)
            .output()
            .unwrap()
            .status
            .success()
    );
}

fn write_signed_bundle(bundle: &Path, version: &str) {
    write_bundle(bundle, version);
    fs::copy("/usr/bin/true", bundle.join("Contents/MacOS/quickie")).unwrap();
    run_tool(
        Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(bundle),
        "Could not ad hoc sign the test application",
    )
    .unwrap();
}

fn write_bundle(bundle: &Path, version: &str) {
    fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    fs::write(bundle.join("Contents/MacOS/quickie"), b"fixture").unwrap();
    fs::write(
        bundle.join("Contents/Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
    <key>CFBundleIdentifier</key><string>dev.quickie.Quickie</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>quickie</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>LSMinimumSystemVersion</key><string>15.0</string>
</dict></plist>"#
        ),
    )
    .unwrap();
}

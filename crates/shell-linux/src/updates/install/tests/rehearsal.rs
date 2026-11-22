//! These ignored tests are called by the package producer on a disposable Linux runner.

use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use quickie_updater::{Distribution, UpdateAsset, Version, select_update};

use super::super::image::{detect_appimage, install_appimage};
use super::super::packages::{
    Distro, PACKAGE_LAUNCHER, distro, install_package, installed_package, rpm_archive_release,
};
use super::super::{InstallError, Target, checked, current_uid, system};

const PREVIOUS: &str = "0.0.0";

#[test]
#[ignore = "requires a disposable Linux package producer with an older Quickie fixture installed"]
fn native_package_upgrade() {
    assert_eq!(current_uid().unwrap(), 0);
    let distro = distro(&fs::read_to_string("/etc/os-release").unwrap()).unwrap();
    let distribution = match distro {
        Distro::Ubuntu => Distribution::Ubuntu24Amd64,
        Distro::Fedora => Distribution::Fedora43X86_64,
    };
    let fixture = Fixture::load(distribution);
    let version = || checked(system(PACKAGE_LAUNCHER).arg("--version")).unwrap();
    let verify_previous = || {
        assert!(installed_package(distro, PREVIOUS, None).unwrap());
        assert_eq!(version().trim(), PREVIOUS);
    };
    verify_previous();
    let installed = Version::parse(PREVIOUS).unwrap();
    fixture.reject_tampering_then_install(
        |path, asset| install_package(distro, path, asset, &installed),
        verify_previous,
    );
    let expected = fixture.asset.version().to_string();
    let release = match distro {
        Distro::Ubuntu => None,
        Distro::Fedora => Some(rpm_archive_release(fixture.asset.name(), &expected).unwrap()),
    };
    assert!(installed_package(distro, &expected, release).unwrap());
    assert_eq!(version().trim(), expected);
}

#[test]
#[ignore = "requires real AppImages supplied by the Ubuntu package producer"]
fn native_appimage_upgrade() {
    assert_ne!(current_uid().unwrap(), 0);
    let fixture = Fixture::load(Distribution::LinuxAppImageX86_64);
    let target = PathBuf::from(env::var_os("QUICKIE_REHEARSAL_APPIMAGE").unwrap());
    let appdir = PathBuf::from(env::var_os("APPDIR").unwrap());
    let executable = appdir.join("usr/bin/quickie").canonicalize().unwrap();
    assert_eq!(
        detect_appimage(&executable),
        Some(Target::AppImage {
            path: target.clone(),
            extracted: true,
        })
    );
    let old_image = File::open(&target).unwrap();
    let original = old_image.metadata().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let version = || {
        checked(
            system(&target)
                .env("TMPDIR", temp.path())
                .args(["--appimage-extract-and-run", "--version"]),
        )
        .unwrap()
    };
    let verify_previous = || {
        let current = fs::metadata(&target).unwrap();
        assert_eq!(current.ino(), original.ino());
        assert_eq!(current.len(), original.len());
        assert_eq!(version().trim(), PREVIOUS);
    };
    verify_previous();
    let stale = install_appimage(&target, fixture.archive.path(), &fixture.asset, "0.0.1");
    assert!(
        matches!(stale, Err(InstallError::Failed(message)) if message.contains("different Quickie version")),
        "the running version must agree with the AppImage still at the installation location"
    );
    verify_previous();
    fixture.reject_tampering_then_install(
        |path, asset| install_appimage(&target, path, asset, PREVIOUS),
        verify_previous,
    );
    let installed = File::open(&target).unwrap();
    let replacement = installed.metadata().unwrap();
    assert_eq!(replacement.dev(), original.dev());
    assert_ne!(replacement.ino(), original.ino());
    assert_eq!(replacement.mode() & 0o777, original.mode() & 0o777);
    assert_eq!(old_image.metadata().unwrap().len(), original.len());
    fixture.asset.verify(installed).unwrap();
    assert_eq!(version().trim(), fixture.asset.version().to_string());
}

struct Fixture {
    asset: UpdateAsset,
    archive: tempfile::NamedTempFile,
}

impl Fixture {
    fn load(distribution: Distribution) -> Self {
        let source = PathBuf::from(env::var_os("QUICKIE_REHEARSAL_ARCHIVE").unwrap());
        let size = source.metadata().unwrap().len();
        let name = source.file_name().unwrap().to_str().unwrap();
        let version = env::var("QUICKIE_REHEARSAL_VERSION").unwrap();
        let digest = env::var("QUICKIE_REHEARSAL_SHA256").unwrap();
        let asset = release_fixture(distribution, &version, name, size, &digest);
        let archive = tempfile::NamedTempFile::new().unwrap();
        fs::copy(source, archive.path()).unwrap();
        archive
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
        Self { asset, archive }
    }

    fn reject_tampering_then_install(
        &self,
        install: impl Fn(&Path, &UpdateAsset) -> Result<(), InstallError>,
        verify_previous: impl FnOnce(),
    ) {
        let mut file = self.archive.reopen().unwrap();
        let mut last_byte = [0];
        file.seek(SeekFrom::End(-1)).unwrap();
        file.read_exact(&mut last_byte).unwrap();
        overwrite_last_byte(&mut file, last_byte[0] ^ 1);
        let result = install(self.archive.path(), &self.asset);
        assert!(
            matches!(result, Err(InstallError::Failed(message)) if message.contains("SHA-256 digest")),
            "tampered bytes did not reach the SHA-256 check"
        );
        verify_previous();
        overwrite_last_byte(&mut file, last_byte[0]);
        install(self.archive.path(), &self.asset).unwrap();
    }
}

fn overwrite_last_byte(file: &mut File, byte: u8) {
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte]).unwrap();
}

fn release_fixture(
    distribution: Distribution,
    version: &str,
    name: &str,
    size: u64,
    digest: &str,
) -> UpdateAsset {
    let response = format!(
        r#"{{
            "draft": false,
            "prerelease": false,
            "tag_name": "v{version}",
            "assets": [{{
                "name": "{name}",
                "browser_download_url": "https://github.com/izevg/quickie/releases/download/v{version}/{name}",
                "size": {size},
                "digest": "sha256:{digest}"
            }}]
        }}"#
    );
    let previous = Version::parse(PREVIOUS).unwrap();
    select_update(response.as_bytes(), &previous, distribution)
        .unwrap()
        .unwrap()
}

#[test]
fn rehearsal_metadata_checks_the_expected_archive_digest() {
    let asset = release_fixture(
        Distribution::Ubuntu24Amd64,
        "1.0.0",
        "quickie_1.0.0_amd64.deb",
        3,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
    assert!(asset.verify(b"abc".as_slice()).is_ok());
    assert!(asset.verify(b"bad".as_slice()).is_err());
}

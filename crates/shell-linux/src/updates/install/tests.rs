use std::ffi::OsStr;
use std::fs;
use std::io::Cursor;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::image::{appimage_header, image_uri, install_local_image, mount_path};
use super::packages::{
    Distro, deb_identity, distro, package_installer, rpm_archive_release, rpm_identity,
};
use super::{failure, stable_version};

mod rehearsal;

#[test]
fn selects_only_supported_package_manager_and_release_pairs() {
    assert_eq!(
        distro("ID=ubuntu\nVERSION_ID=\"24.04\"\n"),
        Some(Distro::Ubuntu)
    );
    assert_eq!(distro("VERSION_ID=43\nID=fedora\n"), Some(Distro::Fedora));
    assert_eq!(distro("ID=ubuntu\nVERSION_ID=26.04\n"), None);
    assert_eq!(distro("ID=fedora\nVERSION_ID=44\n"), None);
    assert_eq!(distro("ID=debian\nVERSION_ID=24.04\n"), None);
}

#[test]
fn rejects_downgrades_and_nonrelease_versions() {
    assert!(stable_version("1.9.10") < stable_version("1.10.0"));
    assert!(stable_version("2.1.0-beta").is_none());
    assert!(stable_version("2.1.0+dev").is_none());
    assert!(stable_version("0.0.0").is_none());
}

#[test]
fn requires_the_fedora_package_name_architecture_epoch_and_release() {
    assert!(rpm_identity(
        "quickie\tx86_64\t0.2.1\t0\t12.fc43\n",
        "0.2.1",
        None
    ));
    for fields in [
        "quickie-tools\tx86_64\t0.2.1\t0\t1.fc43\n",
        "quickie\taarch64\t0.2.1\t0\t1.fc43\n",
        "quickie\tx86_64\t0.2.2\t0\t1.fc43\n",
        "quickie\tx86_64\t0.2.1\t1\t1.fc43\n",
        "quickie\tx86_64\t0.2.1\t0\t1.fc44\n",
        "quickie\tx86_64\t0.2.1\t0\t0.fc43\n",
        "quickie\tx86_64\t0.2.1\t0\t12beta.fc43\n",
    ] {
        assert!(!rpm_identity(fields, "0.2.1", None), "{fields}");
    }
}

#[test]
fn requires_exactly_the_rpm_release_named_by_the_asset() {
    let release = rpm_archive_release("quickie-0.2.1-12.fc43.x86_64.rpm", "0.2.1");
    assert_eq!(release, Some("12.fc43"));
    assert_eq!(
        rpm_archive_release("quickie-0.2.1-0.fc43.x86_64.rpm", "0.2.1"),
        None
    );
    assert_eq!(
        rpm_archive_release("quickie-0.2.0-12.fc43.x86_64.rpm", "0.2.1"),
        None
    );
    assert!(rpm_identity(
        "quickie\tx86_64\t0.2.1\t0\t12.fc43\n",
        "0.2.1",
        release
    ));
    assert!(!rpm_identity(
        "quickie\tx86_64\t0.2.1\t0\t13.fc43\n",
        "0.2.1",
        release
    ));
}

#[test]
fn uses_the_native_package_manager_with_only_the_verified_archive_as_input() {
    let deb = Path::new("/tmp/Quickie update/quickie_0.2.1_amd64.deb");
    let command = package_installer(Distro::Ubuntu, deb);
    assert_eq!(command.get_program(), "/usr/bin/apt-get");
    assert!(command.get_args().eq([
        OsStr::new("--assume-yes"),
        OsStr::new("--no-remove"),
        OsStr::new("--no-install-recommends"),
        OsStr::new("install"),
        OsStr::new("--"),
        deb.as_os_str(),
    ]));

    let rpm = Path::new("/tmp/Quickie update/quickie-0.2.1-12.fc43.x86_64.rpm");
    let command = package_installer(Distro::Fedora, rpm);
    assert_eq!(command.get_program(), "/usr/bin/dnf5");
    assert!(command.get_args().eq([
        OsStr::new("--assumeyes"),
        OsStr::new("upgrade"),
        OsStr::new("--"),
        rpm.as_os_str(),
    ]));
}

#[test]
fn requires_an_x86_64_type_two_appimage() {
    let mut header = [0; 20];
    header[..11].copy_from_slice(b"\x7fELF\x02\x01\x01\x00AI\x02");
    header[18] = 0x3e;
    assert!(appimage_header(Cursor::new(header)));
    header[18] = 0xb7;
    assert!(!appimage_header(Cursor::new(header)));
    header[18] = 0x3e;
    header[10] = 0;
    assert!(!appimage_header(Cursor::new(header)));
    assert!(!appimage_header(Cursor::new([0x7f, b'E', b'L', b'F'])));
}

#[test]
fn encodes_appimage_paths_as_literal_admin_file_locations() {
    assert_eq!(
        image_uri(Path::new("/opt/Quickie #1%.AppImage")),
        "admin:///opt/Quickie%20%231%25.AppImage"
    );
}

#[test]
fn requires_the_debian_package_name_architecture_and_release_version() {
    let fields = "Package: quickie\nVersion: 0.2.1\nArchitecture: amd64\n";
    assert!(deb_identity(fields, "0.2.1"));
    assert!(!deb_identity(fields, "0.2.0"));
    assert!(!deb_identity(&fields.replace("amd64", "arm64"), "0.2.1"));
    assert!(!deb_identity(
        &fields.replace("quickie", "quickie-other"),
        "0.2.1"
    ));
}

#[test]
fn decodes_the_kernel_mountinfo_escape_sequences() {
    assert_eq!(
        mount_path(r"/some\040place/\134.tmp/.mount_QuickiNA9410"),
        r"/some place/\.tmp/.mount_QuickiNA9410"
    );
}

#[test]
fn appimage_replacement_preserves_its_location_and_mode() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("My relocated.AppImage");
    fs::write(&target, b"original").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o751)).unwrap();
    let candidate = root.path().join("download");
    fs::write(&candidate, b"candidate").unwrap();
    let existing = fs::metadata(&target).unwrap();
    let staging = tempfile::tempdir_in(root.path()).unwrap();

    install_local_image(&target, &candidate, &existing, staging, |path| {
        assert_eq!(fs::read(path).unwrap(), b"candidate");
        Ok(())
    })
    .unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"candidate");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o751
    );
}

#[test]
fn appimage_replacement_rolls_back_after_failed_verification() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("My relocated.AppImage");
    fs::write(&target, b"original").unwrap();
    let candidate = root.path().join("download");
    fs::write(&candidate, b"candidate").unwrap();
    let existing = fs::metadata(&target).unwrap();
    let staging = tempfile::tempdir_in(root.path()).unwrap();

    let result = install_local_image(&target, &candidate, &existing, staging, |path| {
        assert_eq!(fs::read(path).unwrap(), b"candidate");
        Err(failure("verification failed"))
    });

    assert!(result.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"original");
}

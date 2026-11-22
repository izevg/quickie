use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;

use quickie_updater::{Distribution, Version, select_update};

use super::{Distro, authorization_command, open_source, parse_request, stage_archive};

#[test]
fn authorizes_the_installed_quickie_only_with_the_requested_package_and_version() {
    let source = Path::new("/tmp/Quickie update/quickie.deb");
    let command = authorization_command(Distro::Ubuntu, source, &Version::new(1, 0, 0));
    assert_eq!(command.get_program(), "/usr/bin/pkexec");
    assert!(command.get_args().eq([
        OsStr::new("--disable-internal-agent"),
        OsStr::new("/opt/quickie/bin/quickie"),
        OsStr::new("--install-verified-package"),
        OsStr::new("deb"),
        OsStr::new("1.0.0"),
        source.as_os_str(),
    ]));
}

#[test]
fn package_request_has_no_arbitrary_command_flags_or_relative_paths() {
    let parse = |arguments: &[&str]| parse_request(arguments.iter().map(OsString::from));
    assert_eq!(
        parse(&["rpm", "1.0.0", "/tmp/archive.rpm"]).unwrap(),
        (
            Distro::Fedora,
            Version::new(1, 0, 0),
            "/tmp/archive.rpm".into()
        )
    );
    for arguments in [
        &["appimage", "1.0.0", "/tmp/archive"][..],
        &["rpm", "1.0.0-tests", "/tmp/archive"],
        &["rpm", "1.0.0", "archive"],
        &["rpm", "1.0.0", "/tmp/archive", "--allowerasing"],
    ] {
        assert!(parse(arguments).is_err());
    }
}

#[test]
fn reads_only_regular_private_archives_owned_by_the_authorizing_user() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("archive.deb");
    fs::write(&source, b"verified archive").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
    let uid = fs::metadata(&source).unwrap().uid();
    assert!(open_source(&source, uid).is_ok());
    assert!(open_source(&source, uid.wrapping_add(1)).is_err());

    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(open_source(&source, uid).is_err());
}

#[test]
fn does_not_follow_the_archive_symlink_or_open_a_fifo_for_reading() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("archive.deb");
    fs::write(&source, b"verified archive").unwrap();
    let uid = fs::metadata(&source).unwrap().uid();
    let link = directory.path().join("link.deb");
    symlink(source, &link).unwrap();
    assert!(open_source(&link, uid).is_err());

    let fifo = directory.path().join("fifo.deb");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .unwrap();
    assert!(open_source(&fifo, uid).is_err());
}

#[test]
fn copied_archive_must_match_the_digest_from_the_authoritative_release() {
    let response = br#"{
        "draft":false,
        "prerelease":false,
        "tag_name":"v1.0.0",
        "assets":[{
            "name":"quickie_1.0.0_amd64.deb",
            "browser_download_url":"https://github.com/izevg/quickie/releases/download/v1.0.0/quickie_1.0.0_amd64.deb",
            "size":3,
            "digest":"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        }]
    }"#;
    let expected = select_update(
        response,
        &Version::new(0, 1, 0),
        Distribution::Ubuntu24Amd64,
    )
    .unwrap()
    .unwrap();
    let source = tempfile::NamedTempFile::new().unwrap();
    let stage = tempfile::tempdir().unwrap();
    let uid = source.as_file().metadata().unwrap().uid();
    fs::write(source.path(), b"bad").unwrap();
    assert!(
        stage_archive(source.path(), uid, &expected, stage.path())
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );

    let stage = tempfile::tempdir().unwrap();
    fs::write(source.path(), b"abc").unwrap();
    let copied = stage_archive(source.path(), uid, &expected, stage.path()).unwrap();
    fs::write(source.path(), b"bad").unwrap();
    assert_eq!(fs::read(&copied).unwrap(), b"abc");
    assert_eq!(fs::metadata(copied).unwrap().mode() & 0o777, 0o600);
}

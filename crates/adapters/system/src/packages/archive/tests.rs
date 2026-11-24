use std::io::{Cursor, Write as _};

use quickie_application::{ExtensionPackages, PackageSource};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::*;
use crate::packages::ExtensionFiles;

pub(in crate::packages) fn test_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for &(name, bytes) in entries {
        archive.start_file(name, options).unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[test]
fn a_traversal_entry_does_not_publish_an_extension_or_write_outside_it() {
    let root = tempfile::tempdir().unwrap();
    let packages = root.path().join("kept");
    let files = ExtensionFiles::new(packages.clone());
    let archive = root.path().join("fixture.zip");
    fs::write(
        &archive,
        test_zip(&[(MANIFEST, b"{}"), ("../outside", b"bad")]),
    )
    .unwrap();

    assert!(files.unpack(&PackageSource::File(archive), None).is_err());
    let written_files = fs::read_dir(packages)
        .unwrap()
        .flat_map(|directory| fs::read_dir(directory.unwrap().path()).unwrap())
        .count();
    assert_eq!(written_files, 0);
}

#[test]
fn pruning_keeps_the_accepted_version_and_removes_old_package_trees() {
    let root = tempfile::tempdir().unwrap();
    let packages = root.path().join("kept");
    let files = ExtensionFiles::new(packages);
    let archive = root.path().join("fixture.zip");
    fs::write(
        &archive,
        test_zip(&[(MANIFEST, b"{}"), ("images/old", b"old")]),
    )
    .unwrap();
    let source = PackageSource::File(archive.clone());
    let old = files.unpack(&source, None).unwrap();
    fs::write(
        &archive,
        test_zip(&[(MANIFEST, b"{}"), ("images/new", b"new")]),
    )
    .unwrap();
    let new = files.unpack(&source, None).unwrap();

    files.prune(std::slice::from_ref(&new.location)).unwrap();

    assert_eq!(old.id, new.id);
    assert_ne!(old.location, new.location);
    assert!(!Path::new(&old.location).exists());
    assert_eq!(
        fs::read(Path::new(&new.location).join("images/new")).unwrap(),
        b"new"
    );
    assert!(archive.exists());
}

#[test]
fn pruning_never_follows_an_extension_id_symlink_outside_its_root() {
    let root = tempfile::tempdir().unwrap();
    let packages = root.path().join("kept");
    let external = root.path().join("external");
    let unrelated = external.join("0".repeat(64));
    fs::create_dir(&packages).unwrap();
    fs::create_dir_all(&unrelated).unwrap();
    std::os::unix::fs::symlink(&external, packages.join("a".repeat(32))).unwrap();

    ExtensionFiles::new(packages).prune(&[]).unwrap();

    assert!(unrelated.is_dir());
}

#[test]
fn the_package_root_must_not_redirect_to_another_directory() {
    let root = tempfile::tempdir().unwrap();
    let packages = root.path().join("kept");
    let external = root.path().join("external");
    let archive = root.path().join("fixture.zip");
    fs::create_dir(&external).unwrap();
    std::os::unix::fs::symlink(&external, &packages).unwrap();
    fs::write(&archive, test_zip(&[(MANIFEST, b"{}")])).unwrap();
    let files = ExtensionFiles::new(packages);

    let install = files.unpack(&PackageSource::File(archive), None);
    let prune = files.prune(&[]);

    assert!(install.is_err());
    assert!(prune.is_err());
    assert_eq!(fs::read_dir(external).unwrap().count(), 0);
}

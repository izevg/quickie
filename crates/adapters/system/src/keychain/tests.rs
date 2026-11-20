//! Against a Keychain file of its own in a temporary directory, so a test never touches the
//! user's login Keychain.

use std::process::Command;

use quickie_application::SecretStore;
use quickie_domain_agents::{ApiKey, BaseUrl, KeyRef};
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};

use super::{KeychainSecrets, SERVICE};

/// A fresh, unlocked Keychain; the directory removes it when dropped.
fn keychain() -> (tempfile::TempDir, KeychainSecrets) {
    let dir = tempfile::tempdir().unwrap();
    let keychain = CreateOptions::new()
        .password("test")
        .create(dir.path().join("test.keychain"))
        .unwrap();
    (dir, KeychainSecrets::in_keychain(keychain))
}

fn reference(base: &str) -> KeyRef {
    KeyRef::for_origin(&BaseUrl::try_from(base).unwrap())
}

fn key(text: &str) -> ApiKey {
    ApiKey::try_from(text).unwrap()
}

#[test]
fn a_key_never_written_is_none() {
    let (_dir, secrets) = keychain();

    let read = secrets.read(&reference("https://api.example.com/v1"));

    assert_eq!(read, Ok(None));
}

#[test]
fn a_written_key_reads_back() {
    let (_dir, secrets) = keychain();
    let origin = reference("https://api.example.com/v1");

    secrets.write(&origin, &key("sk-first")).unwrap();

    assert_eq!(secrets.read(&origin), Ok(Some(key("sk-first"))));
}

#[test]
fn writing_again_replaces_the_key() {
    let (_dir, secrets) = keychain();
    let origin = reference("https://api.example.com/v1");
    secrets.write(&origin, &key("sk-first")).unwrap();

    secrets.write(&origin, &key("sk-second")).unwrap();

    assert_eq!(secrets.read(&origin), Ok(Some(key("sk-second"))));
}

#[test]
fn keys_of_two_origins_are_kept_apart() {
    let (_dir, secrets) = keychain();
    let hosted = reference("https://api.example.com/v1");
    let local = reference("http://127.0.0.1:1234/v1");

    secrets.write(&hosted, &key("sk-hosted")).unwrap();
    secrets.write(&local, &key("sk-local")).unwrap();

    assert_eq!(secrets.read(&hosted), Ok(Some(key("sk-hosted"))));
    assert_eq!(secrets.read(&local), Ok(Some(key("sk-local"))));
}

#[test]
fn a_deleted_key_is_gone() {
    let (_dir, secrets) = keychain();
    let origin = reference("https://api.example.com/v1");
    secrets.write(&origin, &key("sk-first")).unwrap();

    secrets.delete(&origin).unwrap();

    assert_eq!(secrets.read(&origin), Ok(None));
}

#[test]
fn deleting_a_key_that_is_not_there_is_no_error() {
    let (_dir, secrets) = keychain();

    let deleted = secrets.delete(&reference("https://api.example.com/v1"));

    assert_eq!(deleted, Ok(()));
}

#[test]
fn the_secrets_never_show_their_keys_in_debug_output() {
    let (_dir, secrets) = keychain();

    assert_eq!(format!("{secrets:?}"), "KeychainSecrets { default: false }");
    assert_eq!(
        format!("{:?}", KeychainSecrets::default()),
        "KeychainSecrets { default: true }"
    );
}

#[test]
fn an_item_created_before_quickie_is_replaced_not_written_into() {
    let (_dir, secrets) = keychain();
    let origin = reference("https://api.openai.com/v1");
    let keychain = secrets.keychain.clone().unwrap();
    keychain
        .add_generic_password(SERVICE, origin.as_str(), b"squatter")
        .unwrap();
    let (_, mut squatted) = keychain
        .find_generic_password(SERVICE, origin.as_str())
        .unwrap();

    secrets.write(&origin, &key("sk-real")).unwrap();

    let (password, _) = keychain
        .find_generic_password(SERVICE, origin.as_str())
        .unwrap();
    assert_eq!(&*password, b"sk-real");
    assert!(
        squatted.set_password(b"probe").is_err(),
        "the item created before still exists: the key went into it"
    );
}

#[test]
fn a_key_in_another_keychain_is_never_read() {
    let (_dir, ours) = keychain();
    let (_other_dir, other) = keychain();
    let origin = reference("https://api.openai.com/v1");
    other.write(&origin, &key("sk-decoy")).unwrap();

    let read = ours.read(&origin);

    assert_eq!(read, Ok(None));
}

#[test]
#[ignore = "a locked Keychain blocks on securityd without a GUI session (CI); run with --ignored"]
fn deleting_from_a_locked_keychain_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locked.keychain");
    let keychain = CreateOptions::new().password("test").create(&path).unwrap();
    let secrets = KeychainSecrets::in_keychain(keychain);
    let origin = reference("https://api.openai.com/v1");
    secrets.write(&origin, &key("sk-live")).unwrap();
    let locked = Command::new("/usr/bin/security")
        .arg("lock-keychain")
        .arg(&path)
        .status()
        .unwrap();
    assert!(locked.success());
    let _no_prompt = SecKeychain::disable_user_interaction().unwrap();

    let deleted = secrets.delete(&origin);

    assert!(deleted.is_err(), "{deleted:?}");
}

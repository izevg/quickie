mod support;

use quickie_updater::{Distribution, UpdateError, Version, select_update};
use serde_json::json;
use support::{DMG, MACOS, SHA256, asset, installed, release, response};

const DMG_URL: &str = "https://github.com/izevg/quickie/releases/download/v0.2.0/Quickie-0.2.0.dmg";

#[test]
fn selects_the_package_for_each_distribution() {
    let mut fixture = release();
    fixture["assets"] = json!([
        asset(DMG),
        asset("quickie_0.2.0_amd64.deb"),
        asset("quickie-0.2.0-2.fc43.x86_64.rpm"),
        asset("Quickie-0.2.0-x86_64.AppImage"),
        { "name": "Quickie-0.2.0.dmg.sha256" },
        { "name": "quickie-0.2.0-1.fc43.aarch64.rpm" },
    ]);

    for (distribution, name, url) in [
        (MACOS, DMG, DMG_URL),
        (
            Distribution::Ubuntu24Amd64,
            "quickie_0.2.0_amd64.deb",
            "https://github.com/izevg/quickie/releases/download/v0.2.0/quickie_0.2.0_amd64.deb",
        ),
        (
            Distribution::Fedora43X86_64,
            "quickie-0.2.0-2.fc43.x86_64.rpm",
            "https://github.com/izevg/quickie/releases/download/v0.2.0/quickie-0.2.0-2.fc43.x86_64.rpm",
        ),
        (
            Distribution::LinuxAppImageX86_64,
            "Quickie-0.2.0-x86_64.AppImage",
            "https://github.com/izevg/quickie/releases/download/v0.2.0/Quickie-0.2.0-x86_64.AppImage",
        ),
    ] {
        let update = select_update(&response(&fixture), &installed(), distribution)
            .expect("stable release")
            .expect("matched package");
        assert_eq!(update.name(), name);
        assert_eq!(update.version(), &Version::new(0, 2, 0));
        assert_eq!(update.distribution(), distribution);
        assert_eq!(update.url().as_str(), url);
        assert_eq!(update.size(), 5);
    }
}

#[test]
fn rejects_releases_marked_draft_or_prerelease() {
    for field in ["draft", "prerelease"] {
        let mut fixture = release();
        fixture[field] = json!(true);
        assert!(matches!(
            select_update(&response(&fixture), &installed(), MACOS),
            Err(UpdateError::UnstableRelease)
        ));
    }
}

#[test]
fn rejects_tags_outside_canonical_prefixed_stable_semver() {
    for tag in [
        "0.2.0",
        "V0.2.0",
        "v00.2.0",
        "v0.2.0-rc.1",
        "v0.2.0+build.1",
    ] {
        let mut fixture = release();
        fixture["tag_name"] = json!(tag);
        assert!(matches!(
            select_update(&response(&fixture), &installed(), MACOS),
            Err(UpdateError::InvalidVersion)
        ));
    }
}

#[test]
fn current_or_older_releases_need_no_package_for_this_installation() {
    let mut fixture = release();
    fixture["assets"] = json!([]);
    for version in ["0.2.0", "0.2.0+local", "0.3.0", "0.10.0"] {
        assert!(
            select_update(
                &response(&fixture),
                &Version::parse(version).expect("installed version"),
                MACOS,
            )
            .expect("stable release")
            .is_none()
        );
    }
}

#[test]
fn stable_release_updates_a_local_release_candidate() {
    let rc = Version::parse("0.2.0-rc.1").expect("installed release candidate");
    assert!(
        select_update(&response(&release()), &rc, MACOS)
            .expect("stable release")
            .is_some()
    );
}

#[test]
fn requires_an_asset_for_the_installed_distribution() {
    let mut fixture = release();
    fixture["assets"] = json!([
        asset(DMG),
        asset("quickie-0.2.0-0.fc43.x86_64.rpm"),
        asset("quickie-0.2.0-+1.fc43.x86_64.rpm"),
        asset("quickie-0.2.0-2.1.fc43.x86_64.rpm"),
        asset("quickie-0.2.0-1.fc42.x86_64.rpm"),
        asset("quickie-0.2.0-1.fc43.aarch64.rpm"),
    ]);
    assert!(matches!(
        select_update(
            &response(&fixture),
            &installed(),
            Distribution::Fedora43X86_64
        ),
        Err(UpdateError::MissingAsset)
    ));
}

#[test]
fn rejects_ambiguous_assets_before_validating_their_metadata() {
    let mut fixture = release();
    fixture["assets"] = json!([asset(DMG), { "name": DMG }]);
    assert!(matches!(
        select_update(&response(&fixture), &installed(), MACOS),
        Err(UpdateError::AmbiguousAsset)
    ));

    fixture["assets"] = json!([
        asset("quickie-0.2.0-1.fc43.x86_64.rpm"),
        asset("quickie-0.2.0-2.fc43.x86_64.rpm"),
    ]);
    assert!(matches!(
        select_update(
            &response(&fixture),
            &installed(),
            Distribution::Fedora43X86_64
        ),
        Err(UpdateError::AmbiguousAsset)
    ));
}

#[test]
fn rejects_noncanonical_initial_download_urls() {
    for bad_url in [
        "https://example.com/izevg/quickie/releases/download/v0.2.0/Quickie-0.2.0.dmg".into(),
        DMG_URL.replace("/izevg/", "/other/"),
        DMG_URL.replace("/v0.2.0/", "/v0.3.0/"),
        DMG_URL.replace("Quickie-0.2.0.dmg", "other.dmg"),
        DMG_URL.replace("github.com", "github.com:443"),
        DMG_URL.replace("github.com", "x:secret@github.com"),
        format!("{DMG_URL}?"),
        format!("{DMG_URL}#"),
    ] {
        let mut fixture = release();
        fixture["assets"][0]["browser_download_url"] = json!(bad_url);
        assert!(matches!(
            select_update(&response(&fixture), &installed(), MACOS),
            Err(UpdateError::InvalidAssetUrl)
        ));
    }
}

#[test]
fn rejects_digests_outside_the_exact_sha256_format() {
    for digest in [
        json!(null),
        json!(SHA256.replace("sha256:", "sha512:")),
        json!(&SHA256[..SHA256.len() - 1]),
        json!(format!("{SHA256}0")),
        json!(SHA256.replace("2c", "+c")),
        json!(SHA256.replace("2c", "zc")),
    ] {
        let mut fixture = release();
        fixture["assets"][0]["digest"] = digest;
        assert!(matches!(
            select_update(&response(&fixture), &installed(), MACOS),
            Err(UpdateError::InvalidAssetDigest)
        ));
    }
}

#[test]
fn rejects_asset_sizes_outside_the_allowed_byte_range() {
    let mut fixture = release();
    for size in [
        json!(null),
        json!(0),
        json!(-1),
        json!(5.0),
        json!(1_073_741_825_u64),
    ] {
        fixture["assets"][0]["size"] = size;
        assert!(matches!(
            select_update(&response(&fixture), &installed(), MACOS),
            Err(UpdateError::InvalidAssetSize)
        ));
    }
    for size in [1, 1_073_741_824_u64] {
        fixture["assets"][0]["size"] = json!(size);
        let update = select_update(&response(&fixture), &installed(), MACOS)
            .expect("stable release")
            .expect("matched package");
        assert_eq!(update.size(), size);
    }
}

#[test]
fn bounds_metadata_before_parsing_even_if_the_json_is_valid() {
    let mut encoded = response(&release());
    encoded.resize(1_048_576, b' ');
    assert!(
        select_update(&encoded, &installed(), MACOS)
            .expect("metadata at the size limit")
            .is_some()
    );
    encoded.push(b' ');
    assert!(matches!(
        select_update(&encoded, &installed(), MACOS),
        Err(UpdateError::MetadataTooLarge)
    ));
}

#[test]
fn rejects_malformed_release_metadata() {
    for input in [
        b"[]".as_slice(),
        b"{",
        br#"{"draft":false,"prerelease":false,"tag_name":"v0.2.0"}"#,
    ] {
        assert!(matches!(
            select_update(input, &installed(), MACOS),
            Err(UpdateError::InvalidMetadata)
        ));
    }
}

#[test]
fn keeps_the_original_notes_and_defaults_null_or_missing_notes_to_empty() {
    let mut fixture = release();
    let update = select_update(&response(&fixture), &installed(), MACOS)
        .expect("stable release")
        .expect("matched package");
    assert_eq!(update.notes(), "# Highlights\n\n- A new release.\n");

    fixture["body"] = json!(null);
    let update = select_update(&response(&fixture), &installed(), MACOS)
        .expect("stable release")
        .expect("matched package");
    assert_eq!(update.notes(), "");

    fixture
        .as_object_mut()
        .expect("valid release fixture")
        .remove("body");
    let update = select_update(&response(&fixture), &installed(), MACOS)
        .expect("stable release")
        .expect("matched package");
    assert_eq!(update.notes(), "");
}

#[test]
fn truncates_notes_without_splitting_utf8() {
    let mut fixture = release();
    let prefix = "a".repeat(65_535);
    fixture["body"] = json!(format!("{prefix}é."));
    let update = select_update(&response(&fixture), &installed(), MACOS)
        .expect("stable release")
        .expect("matched package");
    assert_eq!(update.notes(), prefix);
}

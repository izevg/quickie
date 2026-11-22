use quickie_updater::{UpdateError, release_asset_redirect};
use url::Url;

const INITIAL: &str = "https://github.com/izevg/quickie/releases/download/v0.2.0/Quickie-0.2.0.dmg";

#[test]
fn follows_the_approved_github_asset_hosts_and_preserves_the_signature_query() {
    let github = Url::parse(INITIAL).expect("initial asset URL");
    let cdn = release_asset_redirect(
        &github,
        "https://release-assets.githubusercontent.com:443/object?sig=abc%2Bdef",
    )
    .expect("release-asset CDN redirect");
    assert_eq!(
        cdn.as_str(),
        "https://release-assets.githubusercontent.com/object?sig=abc%2Bdef"
    );

    let other = release_asset_redirect(&cdn, "//objects.githubusercontent.com/object?sig=second")
        .expect("redirect between approved CDNs");
    assert_eq!(
        other.as_str(),
        "https://objects.githubusercontent.com/object?sig=second"
    );

    let relative = release_asset_redirect(&other, "/other?sig=third").expect("relative CDN hop");
    assert_eq!(
        relative.as_str(),
        "https://objects.githubusercontent.com/other?sig=third"
    );
}

#[test]
fn refuses_redirects_outside_the_allowed_https_asset_authority() {
    let github = Url::parse(INITIAL).expect("initial asset URL");
    for location in [
        "https://release-assets.githubusercontent.com.example.com/file",
        "https://release-assets.githubusercontent.com./file",
        "https://release-assets.githubusercontent.com@example.com/file",
        "https://user:fixture-secret@release-assets.githubusercontent.com/file",
        "https://@release-assets.githubusercontent.com/file",
        "http://release-assets.githubusercontent.com/file",
        "https://release-assets.githubusercontent.com:8443/file",
        "https://release-assets.githubusercontent.com/file#fragment",
        "https://release-assets.githubusercontent.com/?sig=fixture-secret",
        "https://release-assets.githubusercontent.com",
        "/izevg/quickie/releases/download/v0.2.0/Quickie-0.2.0.dmg",
    ] {
        let error = release_asset_redirect(&github, location).expect_err("redirect must fail");
        assert!(matches!(error, UpdateError::InvalidRedirect), "{location}");
        assert!(!error.to_string().contains("fixture-secret"));
    }
}

#[test]
fn only_accepts_sources_in_the_quickie_download_path_or_on_an_approved_cdn() {
    let target = "https://objects.githubusercontent.com/file";
    for source in [
        "https://api.github.com/repos/izevg/quickie/releases/latest",
        "https://github.com/other/quickie/releases/download/v0.2.0/Quickie-0.2.0.dmg",
        "https://github.com/izevg/quickie/releases/download/",
        "https://github.com/izevg/quickie/releases/downloads/v0.2.0/Quickie-0.2.0.dmg",
        "https://release-assets.githubusercontent.com/",
        "https://example.com/file",
        "http://release-assets.githubusercontent.com/file",
        "https://release-assets.githubusercontent.com:8443/file",
        "https://user@release-assets.githubusercontent.com/file",
        "https://release-assets.githubusercontent.com/file#fragment",
    ] {
        assert!(
            matches!(
                release_asset_redirect(&Url::parse(source).expect("source URL"), target),
                Err(UpdateError::InvalidRedirect)
            ),
            "{source}"
        );
    }
}

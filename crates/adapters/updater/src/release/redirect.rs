use url::Url;

use crate::{DOWNLOAD_PATH, GITHUB_HOST, UpdateError};

/// Resolve a release asset redirect to one of the approved GitHub asset CDN hosts.
///
/// The caller limits the number of redirect hops.
///
/// # Errors
///
/// Returns an error if either URL violates the release asset redirect policy.
pub fn release_asset_redirect(current: &Url, location: &str) -> Result<Url, UpdateError> {
    if !is_plain_https(current)
        || !is_asset_source(current)
        || location.is_empty()
        || location
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\')
        || has_invalid_authority(location)
    {
        return Err(UpdateError::InvalidRedirect);
    }

    let target = current
        .join(location)
        .map_err(|_| UpdateError::InvalidRedirect)?;
    if !is_plain_https(&target) || !is_asset_cdn(&target) {
        return Err(UpdateError::InvalidRedirect);
    }
    Ok(target)
}

fn is_plain_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn is_asset_source(url: &Url) -> bool {
    if url.host_str() != Some(GITHUB_HOST) {
        return is_asset_cdn(url);
    }

    url.path()
        .strip_prefix(DOWNLOAD_PATH)
        .and_then(|path| path.split_once('/'))
        .is_some_and(|(tag, name)| {
            !tag.is_empty() && !name.is_empty() && !name.contains('/') && url.query().is_none()
        })
}

fn is_asset_cdn(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
    ) && url.path() != "/"
}

fn has_invalid_authority(location: &str) -> bool {
    // The URL parser normalizes away empty userinfo and repeated authority slashes.
    let authority = location.strip_prefix("//").or_else(|| {
        let (scheme, rest) = location.split_once("://")?;
        scheme.eq_ignore_ascii_case("https").then_some(rest)
    });
    authority
        .and_then(|authority| authority.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.is_empty() || authority.contains('@'))
}

mod redirect;

use std::io::{self, Read};

pub use redirect::release_asset_redirect;
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{DOWNLOAD_PATH, Distribution, GITHUB_HOST, MAX_METADATA_BYTES, UpdateError, Version};

const MAX_ASSET_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_NOTES_BYTES: usize = 64 * 1024;

/// The selected package and its verification metadata from the Quickie release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateAsset {
    version: Version,
    distribution: Distribution,
    name: String,
    url: Url,
    size: u64,
    digest: [u8; 32],
    notes: String,
}

impl UpdateAsset {
    #[must_use]
    pub fn version(&self) -> &Version {
        &self.version
    }

    #[must_use]
    pub fn distribution(&self) -> Distribution {
        self.distribution
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Untrusted release notes, bounded to 64 KiB of UTF-8.
    #[must_use]
    pub fn notes(&self) -> &str {
        &self.notes
    }

    /// Verify the exact byte count and SHA-256 of the complete package.
    ///
    /// # Errors
    ///
    /// Returns an error when the stream fails or its length or digest differs from the metadata.
    pub fn verify(&self, source: impl Read) -> Result<(), UpdateError> {
        let mut digest = Sha256::new();
        // Read one extra byte at most so trailing data cannot pass verification.
        let count = io::copy(&mut source.take(self.size + 1), &mut digest)
            .map_err(|_| UpdateError::ReadAsset)?;
        if count != self.size {
            return Err(UpdateError::AssetSizeMismatch);
        }

        let actual: [u8; 32] = digest.finalize().into();
        if actual != self.digest {
            return Err(UpdateError::AssetDigestMismatch);
        }
        Ok(())
    }
}

/// Select the package for this installation if the latest stable release is newer.
///
/// # Errors
///
/// Returns an error for invalid metadata, an unstable release, or a newer release without a
/// unique, valid package for the requested distribution.
pub fn select_update(
    response: &[u8],
    installed: &Version,
    distribution: Distribution,
) -> Result<Option<UpdateAsset>, UpdateError> {
    if response.len() > MAX_METADATA_BYTES {
        return Err(UpdateError::MetadataTooLarge);
    }
    let metadata: Value =
        serde_json::from_slice(response).map_err(|_| UpdateError::InvalidMetadata)?;
    match (
        metadata["draft"].as_bool(),
        metadata["prerelease"].as_bool(),
    ) {
        (Some(false), Some(false)) => {}
        (Some(true), _) | (_, Some(true)) => return Err(UpdateError::UnstableRelease),
        _ => return Err(UpdateError::InvalidMetadata),
    }

    let tag = metadata["tag_name"]
        .as_str()
        .ok_or(UpdateError::InvalidVersion)?;
    let version = tag
        .strip_prefix('v')
        .and_then(|version| Version::parse(version).ok())
        .filter(|version| version.pre.is_empty() && version.build.is_empty())
        .ok_or(UpdateError::InvalidVersion)?;
    if !version.cmp_precedence(installed).is_gt() {
        return Ok(None);
    }
    let assets = metadata["assets"]
        .as_array()
        .ok_or(UpdateError::InvalidMetadata)?;
    let (name, asset) = matching_asset(assets, &version, distribution)?;

    let expected = format!("https://{GITHUB_HOST}{DOWNLOAD_PATH}{tag}/{name}");
    if asset["browser_download_url"].as_str() != Some(expected.as_str()) {
        return Err(UpdateError::InvalidAssetUrl);
    }
    let url = Url::parse(&expected).map_err(|_| UpdateError::InvalidAssetUrl)?;
    let size = asset["size"]
        .as_u64()
        .filter(|size| (1..=MAX_ASSET_BYTES).contains(size))
        .ok_or(UpdateError::InvalidAssetSize)?;
    let digest = asset["digest"]
        .as_str()
        .and_then(decode_sha256)
        .ok_or(UpdateError::InvalidAssetDigest)?;
    let notes = metadata["body"].as_str().unwrap_or_default();
    let notes = notes[..notes.floor_char_boundary(MAX_NOTES_BYTES)].to_owned();

    Ok(Some(UpdateAsset {
        version,
        distribution,
        name: name.to_owned(),
        url,
        size,
        digest,
        notes,
    }))
}

fn matching_asset<'a>(
    assets: &'a [Value],
    version: &Version,
    distribution: Distribution,
) -> Result<(&'a str, &'a Value), UpdateError> {
    let expected = match distribution {
        Distribution::MacosArm64 => format!("Quickie-{version}.dmg"),
        Distribution::Ubuntu24Amd64 => format!("quickie_{version}_amd64.deb"),
        Distribution::Fedora43X86_64 => format!("quickie-{version}-"),
        Distribution::LinuxAppImageX86_64 => format!("Quickie-{version}-x86_64.AppImage"),
    };
    let mut matching = assets.iter().filter_map(|asset| {
        let name = asset["name"].as_str()?;
        let matches = if distribution == Distribution::Fedora43X86_64 {
            matches_fedora_rpm(name, &expected)
        } else {
            name == expected
        };
        matches.then_some((name, asset))
    });
    let asset = matching.next().ok_or(UpdateError::MissingAsset)?;
    if matching.next().is_some() {
        return Err(UpdateError::AmbiguousAsset);
    }
    Ok(asset)
}

fn matches_fedora_rpm(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(".fc43.x86_64.rpm"))
        .is_some_and(|revision| {
            revision.bytes().all(|byte| byte.is_ascii_digit())
                && !revision.trim_start_matches('0').is_empty()
        })
}

fn decode_sha256(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.strip_prefix("sha256:")?;
    let mut decoded = [0; 32];
    if hex.len() != decoded.len() * 2 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    for (index, byte) in decoded.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&hex[start..start + 2], 16).ok()?;
    }
    Some(decoded)
}

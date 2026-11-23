//! Release checks shared by Quickie's platform installers.

mod preferences;
mod release;
mod transfer;

pub use preferences::Preferences;
pub use release::{UpdateAsset, release_asset_redirect, select_update};
pub use semver::Version;
use thiserror::Error;
pub use transfer::{Response, Transport, download, fetch_latest};

/// GitHub's public endpoint for the latest stable Quickie release.
pub const RELEASES_URL: &str = "https://api.github.com/repos/izevg/quickie/releases/latest";
/// Maximum accepted release metadata response size, in bytes.
pub const MAX_METADATA_BYTES: usize = 1024 * 1024;

const GITHUB_HOST: &str = "github.com";
const DOWNLOAD_PATH: &str = "/izevg/quickie/releases/download/";

/// The packaging and architecture of an existing Quickie installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distribution {
    MacosArm64,
    Ubuntu24Amd64,
    Fedora43X86_64,
    LinuxAppImageX86_64,
}

/// Failures while selecting, following, or verifying a release asset.
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("release metadata exceeds the size limit")]
    MetadataTooLarge,
    #[error("invalid release metadata")]
    InvalidMetadata,
    #[error("the release is a draft or prerelease")]
    UnstableRelease,
    #[error("invalid stable release version")]
    InvalidVersion,
    #[error("the release has no package for this installation")]
    MissingAsset,
    #[error("the release has multiple packages for this installation")]
    AmbiguousAsset,
    #[error("invalid release asset URL")]
    InvalidAssetUrl,
    #[error("invalid release asset size")]
    InvalidAssetSize,
    #[error("invalid release asset SHA-256 digest")]
    InvalidAssetDigest,
    #[error("release asset redirect is not allowed")]
    InvalidRedirect,
    #[error("could not read the release asset")]
    ReadAsset,
    #[error("release asset size does not match its metadata")]
    AssetSizeMismatch,
    #[error("release asset SHA-256 does not match its metadata")]
    AssetDigestMismatch,
}

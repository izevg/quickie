//! Bounded, credential-free transfers of release metadata and verified artifacts.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

use tempfile::TempDir;
use url::Url;

use crate::{
    Distribution, MAX_METADATA_BYTES, RELEASES_URL, UpdateAsset, Version, release_asset_redirect,
    select_update,
};

const MAX_REDIRECTS: usize = 3;

/// A response received without following a redirect.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
}

/// The host engine's ephemeral, non-redirecting transport.
pub trait Transport {
    /// Delivers bytes only for a successful HTTP 200 response. An error from the
    /// sink stops the transfer. Transport errors must omit URL query strings.
    ///
    /// # Errors
    /// Returns a transport or sink error.
    fn get(
        &self,
        url: &Url,
        accept: &str,
        body: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<Response, String>;
}

/// Retrieves and selects the latest compatible stable release.
///
/// # Errors
/// Returns an error if the request fails or release metadata is invalid.
pub fn fetch_latest(
    transport: &impl Transport,
    installed: &Version,
    distribution: Distribution,
) -> Result<Option<UpdateAsset>, String> {
    let url = Url::parse(RELEASES_URL).map_err(|error| error.to_string())?;
    let mut buffer = Vec::new();
    let response = transport.get(&url, "application/vnd.github+json", &mut |bytes| {
        if bytes.len() > MAX_METADATA_BYTES - buffer.len() {
            return Err(io::Error::other(
                "GitHub release metadata exceeded the size limit",
            ));
        }
        buffer.extend_from_slice(bytes);
        Ok(())
    })?;
    if response.status != 200 {
        return Err(format!(
            "GitHub release check returned HTTP {}",
            response.status
        ));
    }
    select_update(&buffer, installed, distribution).map_err(|error| error.to_string())
}

/// Writes a release asset into a private directory and verifies its exact bytes.
///
/// The artifact is at `directory.path().join(asset.name())`. The directory
/// cleans itself up when the caller no longer needs the verified artifact.
///
/// # Errors
/// Returns an error if a redirect is refused, the transfer fails, or verification fails.
pub fn download(transport: &impl Transport, asset: &UpdateAsset) -> Result<TempDir, String> {
    let directory = tempfile::Builder::new()
        .prefix("quickie-update-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let path = directory.path().join(asset.name());
    let mut url = asset.url().clone();
    let mut options = File::options();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    for hop in 0..=MAX_REDIRECTS {
        let mut remaining = asset.size();
        let response = transport.get(&url, "application/octet-stream", &mut |bytes| {
            let count = u64::try_from(bytes.len()).map_err(io::Error::other)?;
            remaining = remaining.checked_sub(count).ok_or_else(|| {
                io::Error::other("the release download exceeded its declared size")
            })?;
            file.write_all(bytes)
        })?;
        match response.status {
            200 => {
                file.seek(SeekFrom::Start(0))
                    .map_err(|error| error.to_string())?;
                asset.verify(&mut file).map_err(|error| error.to_string())?;
                return Ok(directory);
            }
            301 | 302 | 303 | 307 | 308 if hop < MAX_REDIRECTS => {
                let location = response
                    .location
                    .ok_or("the release redirect has no location")?;
                url = release_asset_redirect(&url, &location).map_err(|error| error.to_string())?;
            }
            301 | 302 | 303 | 307 | 308 => return Err("too many release download redirects".into()),
            status => return Err(format!("GitHub release download returned HTTP {status}")),
        }
    }
    Err("too many release download redirects".into())
}

#[cfg(test)]
mod tests;

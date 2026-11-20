//! The Chrome Web Store's update protocol, which installs as well as it updates: ask for an
//! extension at a version (`0` for one not installed yet), and the answer names the newer
//! package's version, where to download it, and its SHA-256. Asked only on the user's request,
//! or daily when they let updates come on their own (`docs/privacy.md`).
//!
//! Every request goes through the engine's [`HttpPort`]; the package's own signature, checked
//! by [`ExtensionPackages`], is what makes it trusted, so a download host is accepted only to
//! keep requests on Google's update hosts.

use quickie_domain_settings::{ExtensionId, ExtensionVersion};
use url::Url;

use crate::{Expected, ExtensionPackages, HttpPort, PackageError, PackageSource, Unpacked};

/// Where the store answers update checks.
const UPDATE_SERVICE: &str = "https://clients2.google.com/service/update2/crx";

/// The browser version the store is told; it serves an extension only to a browser at least
/// as new as the extension's `minimum_chrome_version`.
const PRODUCT_VERSION: &str = "140.0.0.0";

/// The hosts a store answer may send a download to, each with its subdomains.
const DOWNLOAD_HOSTS: [&str; 3] = ["googleusercontent.com", "google.com", "gvt1.com"];

/// One extension to ask the store about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateCheck {
    pub id: ExtensionId,
    /// The update check's URL, which names the version installed.
    pub url: Url,
}

impl UpdateCheck {
    /// The check for `id` at `installed`, or for an extension not installed yet.
    ///
    /// # Errors
    ///
    /// Never in practice: the service's address is a constant.
    pub fn new(
        id: ExtensionId,
        installed: Option<&ExtensionVersion>,
    ) -> Result<Self, PackageError> {
        let mut url =
            Url::parse(UPDATE_SERVICE).map_err(|error| PackageError(error.to_string()))?;
        let version = installed.map_or("0", ExtensionVersion::as_str);
        url.query_pairs_mut()
            .append_pair("response", "updatecheck")
            .append_pair("acceptformat", "crx3")
            .append_pair("prodversion", PRODUCT_VERSION)
            .append_pair(
                "x",
                &format!("id={id}&v={version}&installsource=ondemand&uc"),
            );
        Ok(Self { id, url })
    }
}

/// A newer package the store offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateOffer {
    pub version: ExtensionVersion,
    pub download: Url,
    /// Its SHA-256, in lowercase hexadecimal, when the store gave one.
    pub sha256: Option<String>,
}

/// What asking the store about one extension came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub id: ExtensionId,
    /// The newer package, checked and kept; `None` when there is none.
    pub package: Result<Option<Unpacked>, PackageError>,
}

/// Asks the store about each extension and, for each that has a newer package, downloads it
/// and hands it to `packages`. Blocks on the network: call it on a worker thread.
pub fn fetch_packages<H: HttpPort, P: ExtensionPackages>(
    http: &H,
    packages: &P,
    checks: &[UpdateCheck],
) -> Vec<Fetched> {
    checks
        .iter()
        .map(|check| Fetched {
            id: check.id.clone(),
            package: fetch_one(http, packages, check),
        })
        .collect()
}

fn fetch_one<H: HttpPort, P: ExtensionPackages>(
    http: &H,
    packages: &P,
    check: &UpdateCheck,
) -> Result<Option<Unpacked>, PackageError> {
    let answer = http
        .get(&check.url)
        .map_err(|error| PackageError(format!("the store did not answer: {error}")))?;
    let answer = String::from_utf8_lossy(&answer);
    let Some(offer) = parse_answer(&answer, &check.id)? else {
        return Ok(None);
    };
    let crx = http
        .get(&offer.download)
        .map_err(|error| PackageError(format!("the download failed: {error}")))?;
    let expected = Expected {
        id: check.id.clone(),
        sha256: offer.sha256,
    };
    packages
        .unpack(&PackageSource::Crx(crx), Some(&expected))
        .map(Some)
}

/// What the store's answer (`gupdate` XML) says of `id`: a newer package, or `None` when there
/// is none.
///
/// # Errors
///
/// The answer does not mention `id`, the store refused it, or the offer lacks a version or a
/// download on one of the store's hosts.
pub fn parse_answer(answer: &str, id: &ExtensionId) -> Result<Option<UpdateOffer>, PackageError> {
    let fail = |why: &str| PackageError(format!("the store's answer for {id} {why}"));
    let app = element(answer, "app", |app| {
        attribute(app, "appid").as_deref() == Some(id.as_str())
    })
    .ok_or_else(|| fail("is missing"))?;
    let rest = answer.get(app..).unwrap_or_default();
    let check = element(rest, "updatecheck", |_| true)
        .and_then(|at| rest.get(at..))
        .and_then(|text| text.split('>').next())
        .ok_or_else(|| fail("has no update check"))?;
    match attribute(check, "status").as_deref() {
        Some("noupdate") => return Ok(None),
        Some("ok") => {}
        status => return Err(fail(&format!("is status {}", status.unwrap_or("none")))),
    }
    let version = attribute(check, "version")
        .and_then(|version| ExtensionVersion::try_from(version.as_str()).ok())
        .ok_or_else(|| fail("has no version"))?;
    let download = attribute(check, "codebase")
        .and_then(|codebase| Url::parse(&codebase).ok())
        .filter(on_store_host)
        .ok_or_else(|| fail("has no download on the store's hosts"))?;
    let sha256 = attribute(check, "hash_sha256").filter(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    Ok(Some(UpdateOffer {
        version,
        download,
        sha256,
    }))
}

/// Whether a download is over HTTPS on one of the store's hosts.
fn on_store_host(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            DOWNLOAD_HOSTS.iter().any(|allowed| {
                host == *allowed
                    || host
                        .strip_suffix(allowed)
                        .is_some_and(|sub| sub.ends_with('.'))
            })
        })
}

/// Where the first `<name …>` start tag for which `wanted` holds of its text begins.
fn element(text: &str, name: &str, wanted: impl Fn(&str) -> bool) -> Option<usize> {
    let open = format!("<{name} ");
    text.match_indices(&open)
        .map(|(start, _)| start)
        .find(|&start| {
            text.get(start..)
                .and_then(|tag| tag.split('>').next())
                .is_some_and(&wanted)
        })
}

/// The value of attribute `name` in a start tag, its `&amp;` read as `&`.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let value = tag.get(start..)?.split('"').next()?;
    Some(value.replace("&amp;", "&"))
}

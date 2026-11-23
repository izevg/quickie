use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use objc2_foundation::{
    NSBundle, NSData, NSDictionary, NSOperatingSystemVersion, NSProcessInfo,
    NSPropertyListMutabilityOptions, NSPropertyListSerialization, NSString, NSURL,
    NSURLQuarantinePropertiesKey, ns_string,
};

use super::run_tool;

const BUNDLE_ID: &str = "dev.quickie.Quickie";
const CPU_TYPE_ARM64: i32 = 0x0100_000c;
const MAX_PLIST_BYTES: u64 = 1024 * 1024;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    #[link_name = "kLSQuarantineTypeKey"]
    static QUARANTINE_TYPE: &'static NSString;
    #[link_name = "kLSQuarantineTypeWebDownload"]
    static QUARANTINE_WEB_DOWNLOAD: &'static NSString;
    #[link_name = "kLSQuarantineAgentNameKey"]
    static QUARANTINE_AGENT_NAME: &'static NSString;
}

pub(super) fn preserve_quarantine(bundle: &Path) -> Result<(), String> {
    let url = NSURL::from_directory_path(bundle).ok_or("The staged bundle path is invalid.")?;
    // SAFETY: Foundation exports this immutable resource key.
    let property = unsafe { NSURLQuarantinePropertiesKey };
    let mut existing = None;
    // SAFETY: the output is a mutable slot for the returned Objective-C object.
    unsafe { url.getResourceValue_forKey_error(&mut existing, property) }
        .map_err(|error| format!("Could not read the update's quarantine metadata: {error}"))?;
    if existing.is_some() {
        return Ok(());
    }
    // SAFETY: Launch Services exports these immutable, toll-free bridged CF strings.
    let quarantine_type = unsafe { QUARANTINE_TYPE };
    // SAFETY: Launch Services exports this immutable, toll-free bridged CF string.
    let agent_name = unsafe { QUARANTINE_AGENT_NAME };
    // SAFETY: Launch Services exports this immutable, toll-free bridged CF string.
    let web_download = unsafe { QUARANTINE_WEB_DOWNLOAD };
    let quarantine = NSDictionary::from_slices(
        &[quarantine_type, agent_name],
        &[web_download, ns_string!("Quickie")],
    );
    // SAFETY: the quarantine key accepts a quarantine properties dictionary.
    unsafe { url.setResourceValue_forKey_error(Some(quarantine.as_ref()), property) }
        .map_err(|error| format!("Could not preserve Gatekeeper checks for the update: {error}"))
}

pub(super) struct BundleIdentity {
    version: String,
    minimum: Option<String>,
}

impl BundleIdentity {
    pub(super) fn read(bundle: &Path) -> Result<Self, String> {
        validated_executable(bundle)?;
        let metadata = bundle.join("Contents/Info.plist");
        let file = fs::symlink_metadata(&metadata)
            .map_err(|error| format!("Could not inspect Quickie's bundle metadata: {error}"))?;
        if !file.is_file() || file.len() > MAX_PLIST_BYTES {
            return Err("Quickie's bundle metadata is not a supported property list.".into());
        }
        let mut bytes = Vec::new();
        File::open(metadata)
            .and_then(|file| file.take(MAX_PLIST_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| format!("Could not read Quickie's bundle metadata: {error}"))?;
        if bytes.len() as u64 > MAX_PLIST_BYTES {
            return Err("Quickie's bundle metadata is too large.".into());
        }
        let data = NSData::with_bytes(&bytes);
        // SAFETY: the immutable data stays alive, and a null format pointer skips that output.
        let plist = unsafe {
            NSPropertyListSerialization::propertyListWithData_options_format_error(
                &data,
                NSPropertyListMutabilityOptions::Immutable,
                ptr::null_mut(),
            )
        }
        .map_err(|error| format!("Could not parse Quickie's bundle metadata: {error}"))?
        .downcast::<NSDictionary>()
        .map_err(|_| "Quickie's bundle metadata is not a dictionary.")?;

        let text = |key: &NSString| {
            plist
                .objectForKey(key.as_ref())
                .and_then(|value| value.downcast::<NSString>().ok())
                .map(|value| value.to_string())
        };
        if text(ns_string!("CFBundleIdentifier")).as_deref() != Some(BUNDLE_ID)
            || text(ns_string!("CFBundlePackageType")).as_deref() != Some("APPL")
            || text(ns_string!("CFBundleExecutable")).as_deref() != Some("quickie")
        {
            return Err("The update bundle does not identify as the Quickie application.".into());
        }
        let version = text(ns_string!("CFBundleShortVersionString"))
            .ok_or("Quickie's bundle is missing its release version.")?;
        if text(ns_string!("CFBundleVersion")).as_deref() != Some(version.as_str()) {
            return Err("Quickie's bundle version fields disagree.".into());
        }

        let minimum =
            match plist.objectForKey(ns_string!("LSMinimumSystemVersionByArchitecture").as_ref()) {
                None => text(ns_string!("LSMinimumSystemVersion")),
                Some(overrides) => {
                    let overrides = overrides
                        .downcast::<NSDictionary>()
                        .map_err(|_| "Quickie's architecture-specific macOS minimum is invalid.")?;
                    match overrides.objectForKey(ns_string!("arm64").as_ref()) {
                        None => text(ns_string!("LSMinimumSystemVersion")),
                        Some(minimum) => Some(
                            minimum
                                .downcast::<NSString>()
                                .map_err(|_| "Quickie's Apple silicon macOS minimum is invalid.")?
                                .to_string(),
                        ),
                    }
                }
            };
        Ok(Self { version, minimum })
    }

    pub(super) fn verify_version(&self, expected: &str) -> Result<(), String> {
        if self.version != expected {
            return Err(format!(
                "Expected Quickie {expected}, but the bundle contains {}.",
                self.version
            ));
        }
        Ok(())
    }

    fn verify_compatibility(&self, bundle: &Path) -> Result<(), String> {
        let url =
            NSURL::from_directory_path(bundle).ok_or("The Quickie bundle path is invalid.")?;
        let compatible = NSBundle::bundleWithURL(&url)
            .and_then(|bundle| bundle.executableArchitectures())
            .is_some_and(|architectures| {
                (0..architectures.count())
                    .any(|index| architectures.objectAtIndex(index).intValue() == CPU_TYPE_ARM64)
            });
        if !compatible || !cfg!(target_arch = "aarch64") {
            return Err("This Quickie update requires an Apple silicon executable and Mac.".into());
        }
        let minimum = self
            .minimum
            .as_deref()
            .ok_or("Quickie's minimum macOS version is missing.")?;
        let [major, minor, patch] = version(minimum, 2)?;
        let minimum_version = NSOperatingSystemVersion {
            majorVersion: major,
            minorVersion: minor,
            patchVersion: patch,
        };
        if !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(minimum_version) {
            return Err(format!(
                "This Quickie update requires macOS {minimum} or later."
            ));
        }
        Ok(())
    }
}

pub(super) enum Signature {
    AdHoc,
    Team(String),
}

impl Signature {
    pub(super) fn read(bundle: &Path) -> Result<Self, String> {
        let output = run_tool(
            Command::new("/usr/bin/codesign")
                .args(["-d", "--verbose=4"])
                .arg(bundle),
            "Could not inspect the installed Quickie code signature",
        )?;
        let details = String::from_utf8_lossy(&output.stderr);
        let team = details
            .lines()
            .find_map(|line| line.strip_prefix("TeamIdentifier="))
            .ok_or("Quickie's signature has no signing information.")?;
        if team == "not set" && details.lines().any(|line| line == "Signature=adhoc") {
            return Ok(Self::AdHoc);
        }
        if team.len() == 10
            && team
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Ok(Self::Team(team.into()));
        }
        Err("Quickie's signature has an unsupported signing identity.".into())
    }

    fn verify(&self, bundle: &Path) -> Result<(), String> {
        let mut command = Command::new("/usr/bin/codesign");
        command.args(["--verify", "--deep", "--strict", "--all-architectures"]);
        if let Self::Team(team) = self {
            command.arg("-R").arg(format!(
                "anchor apple generic and certificate leaf[subject.OU] = \"{team}\""
            ));
        }
        run_tool(
            command.arg(bundle),
            "Quickie's update code signature could not be verified",
        )?;
        Ok(())
    }
}

pub(super) fn verify(bundle: &Path, expected: &str, signature: &Signature) -> Result<(), String> {
    let identity = BundleIdentity::read(bundle)?;
    identity.verify_version(expected)?;
    identity.verify_compatibility(bundle)?;
    signature.verify(bundle)
}

pub(super) fn validated_executable(bundle: &Path) -> Result<PathBuf, String> {
    for directory in [
        bundle.to_owned(),
        bundle.join("Contents"),
        bundle.join("Contents/MacOS"),
    ] {
        if !fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.is_dir()) {
            return Err("The Quickie bundle is missing or its directories are invalid.".into());
        }
    }
    let executable = bundle.join("Contents/MacOS/quickie");
    if !fs::symlink_metadata(&executable).is_ok_and(|metadata| metadata.is_file()) {
        return Err("The Quickie bundle does not contain the expected executable.".into());
    }
    Ok(executable)
}

pub(super) fn code_hash(bundle: &Path) -> Result<String, String> {
    let output = run_tool(
        Command::new("/usr/bin/codesign")
            .args(["--display", "--arch", "arm64", "--verbose=4"])
            .arg(bundle),
        "Could not read Quickie's Apple silicon code hash",
    )?;
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.strip_prefix("CDHash="))
        .filter(|hash| {
            hash.len() == 40
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .map(str::to_owned)
        .ok_or_else(|| "Quickie's code directory hash is missing or invalid.".into())
}

pub(super) fn version(text: &str, required: usize) -> Result<[isize; 3], String> {
    let invalid = || format!("Quickie received an invalid version: {text}");
    let mut parts = text.split('.');
    let mut version = [0; 3];
    for (index, number) in version.iter_mut().enumerate() {
        let Some(part) = parts.next() else {
            if index < required {
                return Err(invalid());
            }
            break;
        };
        if part.starts_with('0') && part.len() > 1
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid());
        }
        *number = part.parse().map_err(|_| invalid())?;
    }
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok(version)
}

use std::path::Path;
use std::process::Command;

use quickie_updater::{UpdateAsset, Version};

#[cfg(target_os = "linux")]
mod privileged;
#[cfg(target_os = "linux")]
pub(crate) use privileged::{INSTALL_PACKAGE_ARGUMENT, install_privileged};

#[cfg(target_os = "linux")]
use super::current_uid;
use super::{InstallError, checked, failure, system};

pub(super) const PACKAGED_EXECUTABLE: &str = "/opt/quickie/bin/quickie";
pub(super) const PACKAGE_LAUNCHER: &str = "/usr/bin/quickie";
const RPM_FORMAT: &str = "%{NAME}\\t%{ARCH}\\t%{VERSION}\\t%{EPOCHNUM}\\t%{RELEASE}\\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Distro {
    Ubuntu,
    Fedora,
}

pub(super) fn distro(release: &str) -> Option<Distro> {
    let field = |key| {
        release.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name == key).then(|| value.trim_matches(['"', '\'']))
        })
    };
    match (field("ID")?, field("VERSION_ID")?) {
        ("ubuntu", "24.04") => Some(Distro::Ubuntu),
        ("fedora", "43") => Some(Distro::Fedora),
        _ => None,
    }
}

fn valid_fedora_release(release: &str) -> bool {
    release.strip_suffix(".fc43").is_some_and(|revision| {
        revision.bytes().all(|byte| byte.is_ascii_digit())
            && !revision.trim_start_matches('0').is_empty()
    })
}

pub(super) fn rpm_archive_release<'a>(name: &'a str, version: &str) -> Option<&'a str> {
    let release = name
        .strip_prefix(&format!("quickie-{version}-"))?
        .strip_suffix(".x86_64.rpm")?;
    valid_fedora_release(release).then_some(release)
}

pub(super) fn rpm_identity(
    fields: &str,
    expected_version: &str,
    expected_release: Option<&str>,
) -> bool {
    let mut fields = fields.trim_end_matches('\n').split('\t');
    fields
        .by_ref()
        .take(4)
        .eq(["quickie", "x86_64", expected_version, "0"])
        && fields.next().is_some_and(|release| {
            valid_fedora_release(release)
                && expected_release.is_none_or(|expected| release == expected)
        })
        && fields.next().is_none()
}

pub(super) fn installed_package(
    distribution: Distro,
    version: &str,
    fedora_release: Option<&str>,
) -> Result<bool, InstallError> {
    match distribution {
        Distro::Ubuntu => {
            let owner = checked(system("/usr/bin/dpkg-query").args([
                "--search",
                "--",
                PACKAGED_EXECUTABLE,
            ]))?;
            if owner.trim_end_matches('\n') != "quickie: /opt/quickie/bin/quickie" {
                return Ok(false);
            }
            let metadata = checked(system("/usr/bin/dpkg-query").args([
                "--show",
                "--showformat=${Package}\\t${Architecture}\\t${Version}\\t${Status}\\n",
                "--",
                "quickie",
            ]))?;
            Ok(metadata.trim_end_matches('\n').split('\t').eq([
                "quickie",
                "amd64",
                version,
                "install ok installed",
            ]))
        }
        Distro::Fedora => {
            let metadata = checked(system("/usr/bin/rpm").args([
                "--noplugins",
                "--query",
                "--file",
                "--queryformat",
                RPM_FORMAT,
                "--",
                PACKAGED_EXECUTABLE,
            ]))?;
            Ok(rpm_identity(&metadata, version, fedora_release))
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn install_package(
    distribution: Distro,
    path: &Path,
    asset: &UpdateAsset,
    installed: &Version,
) -> Result<(), InstallError> {
    let uid = current_uid()?;
    if uid != 0 {
        return privileged::request(distribution, path, asset.version());
    }
    privileged::install_root_archive(distribution, path, asset, installed, uid)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn install_package(
    _: Distro,
    _: &Path,
    _: &UpdateAsset,
    _: &Version,
) -> Result<(), InstallError> {
    Err(failure("native package updates require Linux"))
}

#[cfg(target_os = "linux")]
fn apply_archive(
    distribution: Distro,
    archive: &Path,
    asset: &UpdateAsset,
    installed: &Version,
) -> Result<(), InstallError> {
    let version = asset.version().to_string();
    let expected_rpm_release = match distribution {
        Distro::Ubuntu => {
            let metadata = checked(
                system("/usr/bin/dpkg-deb")
                    .args(["--field", "--"])
                    .arg(archive)
                    .args(["Package", "Version", "Architecture"]),
            )?;
            if !deb_identity(&metadata, &version) {
                return Err(failure(
                    "the downloaded Debian package identity does not match",
                ));
            }
            None
        }
        Distro::Fedora => {
            let release = rpm_archive_release(asset.name(), &version)
                .ok_or_else(|| failure("the update RPM file name has an invalid release"))?;
            let metadata = checked(
                system("/usr/bin/rpm")
                    .args([
                        "--noplugins",
                        "--query",
                        "--package",
                        "--queryformat",
                        RPM_FORMAT,
                        "--",
                    ])
                    .arg(archive),
            )?;
            if !rpm_identity(&metadata, &version, Some(release)) {
                return Err(failure(
                    "the downloaded RPM package identity does not match",
                ));
            }
            Some(release)
        }
    };
    if *asset.version() <= *installed
        || !installed_package(distribution, &installed.to_string(), None)?
    {
        return Err(failure(
            "the installed Quickie version changed. Check for updates again.",
        ));
    }
    checked(&mut package_installer(distribution, archive))?;
    if !installed_package(distribution, &version, expected_rpm_release)? {
        return Err(failure(
            "the package manager did not install the expected Quickie release",
        ));
    }
    Ok(())
}

pub(super) fn deb_identity(fields: &str, expected_version: &str) -> bool {
    let mut lines: Vec<_> = fields.lines().collect();
    lines.sort_unstable();
    lines
        == [
            "Architecture: amd64",
            "Package: quickie",
            &format!("Version: {expected_version}"),
        ]
}

pub(super) fn package_installer(distribution: Distro, archive: &Path) -> Command {
    // Quickie has already verified this local release archive. The approved native transaction
    // resolves OS dependencies using the system's repository and signature policy.
    let (program, arguments): (&str, &[&str]) = match distribution {
        Distro::Ubuntu => (
            "/usr/bin/apt-get",
            &[
                "--assume-yes",
                "--no-remove",
                "--no-install-recommends",
                "install",
                "--",
            ],
        ),
        Distro::Fedora => ("/usr/bin/dnf5", &["--assumeyes", "upgrade", "--"]),
    };
    let mut command = system(program);
    command.args(arguments).arg(archive);
    command
}

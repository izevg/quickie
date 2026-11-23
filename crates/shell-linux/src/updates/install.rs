use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use quickie_updater::{Distribution, UpdateAsset, UpdateError, Version};

#[cfg(target_os = "linux")]
mod gio;
mod image;
mod packages;
#[cfg(test)]
mod tests;

use image::{detect_appimage, install_appimage};
use packages::{
    Distro, PACKAGE_LAUNCHER, PACKAGED_EXECUTABLE, distro, install_package, installed_package,
};
#[cfg(target_os = "linux")]
pub(crate) use packages::{INSTALL_PACKAGE_ARGUMENT, install_privileged};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Package(Distro),
    AppImage { path: PathBuf, extracted: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installation {
    current: Version,
    target: Target,
}

#[derive(Debug)]
pub enum InstallError {
    Cancelled,
    Failed(String),
    Verification(UpdateError),
}

impl fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("update authorization was cancelled"),
            Self::Failed(message) => formatter.write_str(message),
            Self::Verification(error) => write!(formatter, "{error}. Download the update again."),
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

impl Installation {
    #[must_use]
    pub fn detect(current_version: &str) -> Option<Self> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return None;
        }
        let current = stable_version(current_version)?;
        let executable = env::current_exe().ok()?.canonicalize().ok()?;
        let target = if executable == Path::new(PACKAGED_EXECUTABLE) {
            let distribution = distro(&fs::read_to_string("/etc/os-release").ok()?)?;
            if !trusted_root_path(&executable)
                || Path::new(PACKAGE_LAUNCHER).canonicalize().ok()? != executable
                || !installed_package(distribution, current_version, None).ok()?
            {
                return None;
            }
            Target::Package(distribution)
        } else {
            detect_appimage(&executable)?
        };
        Some(Self { current, target })
    }

    #[must_use]
    pub fn kind(&self) -> Distribution {
        match self.target {
            Target::Package(Distro::Ubuntu) => Distribution::Ubuntu24Amd64,
            Target::Package(Distro::Fedora) => Distribution::Fedora43X86_64,
            Target::AppImage { .. } => Distribution::LinuxAppImageX86_64,
        }
    }

    /// Rechecks the installed and downloaded packages before applying a verified release.
    ///
    /// # Errors
    ///
    /// Returns an error if verification, authorization, or installation fails.
    pub fn install(&self, artifact: &Path, asset: &UpdateAsset) -> Result<(), InstallError> {
        let current_version = self.current.to_string();
        if asset.distribution() != self.kind()
            || asset.version() <= &self.current
            || Self::detect(&current_version).as_ref() != Some(self)
        {
            return Err(failure(
                "the current installation does not match this update",
            ));
        }
        let staged = fs::symlink_metadata(artifact)?;
        if !staged.is_file() || staged.uid() != current_uid()? || staged.mode() & 0o077 != 0 {
            return Err(failure(
                "the downloaded update is not a private regular file",
            ));
        }
        let artifact = artifact.canonicalize()?;
        verify_asset(&artifact, asset)?;
        match &self.target {
            Target::Package(distribution) => {
                install_package(*distribution, &artifact, asset, &self.current)
            }
            Target::AppImage { path, .. } => {
                install_appimage(path, &artifact, asset, &current_version)
            }
        }
    }

    /// # Errors
    ///
    /// Returns an error if the installed executable cannot be launched.
    pub fn relaunch(&self) -> Result<(), InstallError> {
        let mut command = match &self.target {
            Target::Package(_) => Command::new(PACKAGE_LAUNCHER),
            Target::AppImage { path, extracted } => {
                let mut command = Command::new(path);
                if *extracted {
                    command.arg("--appimage-extract-and-run");
                }
                command
            }
        };
        command
            .env_remove("TARGET_APPIMAGE")
            .env_remove("APPIMAGE")
            .env_remove("APPDIR")
            .env_remove("ARGV0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }
}

fn stable_version(value: &str) -> Option<Version> {
    let version = Version::parse(value).ok()?;
    (version.pre.is_empty() && version.build.is_empty() && version != Version::new(0, 0, 0))
        .then_some(version)
}

fn trusted_root_path(path: &Path) -> bool {
    path.is_absolute()
        && path.ancestors().all(|ancestor| {
            fs::symlink_metadata(ancestor).is_ok_and(|metadata| {
                metadata.uid() == 0 && metadata.mode() & 0o022 == 0 && !metadata.is_symlink()
            })
        })
}

fn current_uid() -> Result<u32, InstallError> {
    checked(system("/usr/bin/id").arg("-u"))?
        .trim()
        .parse()
        .map_err(|_| failure("could not identify the user performing the update"))
}

fn system(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", "/")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    command
}

fn checked(command: &mut Command) -> Result<String, InstallError> {
    let output = command.output()?;
    command_result(command.get_program(), &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn command_result(program: &OsStr, output: &Output) -> Result<(), InstallError> {
    if output.status.success() {
        return Ok(());
    }
    if program == "/usr/bin/pkexec" && output.status.code() == Some(126) {
        return Err(InstallError::Cancelled);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail: String = stderr
        .trim()
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(512)
        .collect();
    Err(InstallError::Failed(if detail.is_empty() {
        format!("{} returned {}", program.to_string_lossy(), output.status)
    } else {
        detail
    }))
}

fn failure(message: &str) -> InstallError {
    InstallError::Failed(message.into())
}

fn verify_asset(path: &Path, asset: &UpdateAsset) -> Result<(), InstallError> {
    asset
        .verify(File::open(path)?)
        .map_err(InstallError::Verification)
}

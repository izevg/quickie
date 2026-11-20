//! The application's `ExtensionPackages`: web extension packages checked and kept in the
//! profile, where the engine reads them.
//!
//! A CRX from the Chrome Web Store, or one the user chose, is kept only once its signatures
//! hold and prove the expected id ([`crx`]); a ZIP the user chose has no signature and takes
//! the id of where it was found, as Chrome's unpacked extensions do. What is kept is the ZIP
//! archive itself, which `WKWebExtension` opens directly, at
//! `<root>/<id>/<first 16 hex digits of its SHA-256>.zip`: a new version never overwrites the
//! one running. A folder the user chose is used where it is and never copied or deleted.
//!
//! - [`crx`]: the CRX3 format and its signature check.
//! - [`crypto`]: SHA-256 and signature checks from the system.

mod crx;
mod crypto;
#[cfg(test)]
mod tests;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use quickie_application::{Expected, ExtensionPackages, PackageError, PackageSource, Unpacked};
use quickie_domain_settings::ExtensionId;

/// What a ZIP archive starts with.
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";

/// What a CRX starts with.
const CRX_MAGIC: &[u8; 4] = b"Cr24";

/// The packages of one profile, under `root`.
#[derive(Clone, Debug)]
pub struct ExtensionFiles {
    root: PathBuf,
}

impl ExtensionFiles {
    /// Packages kept under `root` (`…/Quickie/Extensions`), created when the first arrives.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn keep_crx(&self, crx: &[u8], expected: Option<&Expected>) -> Result<Unpacked, PackageError> {
        if let Some(wanted) = expected.and_then(|expected| expected.sha256.as_deref())
            && hex(&crypto::sha256(&[crx])) != wanted
        {
            return Err(PackageError(
                "the package is not the one the store announced".to_owned(),
            ));
        }
        let verified = crx::verify(crx).map_err(|error| PackageError(error.to_string()))?;
        if let Some(expected) = expected
            && expected.id != verified.id
        {
            return Err(PackageError(format!(
                "the package is signed as {}, not as {}",
                verified.id, expected.id
            )));
        }
        self.keep(&verified.id, verified.archive)
    }

    /// Writes `archive` as `id`'s, unless the same archive is already there.
    fn keep(&self, id: &ExtensionId, archive: &[u8]) -> Result<Unpacked, PackageError> {
        let directory = self.root.join(id.as_str());
        fs::create_dir_all(&directory).map_err(|error| failed(&directory, &error))?;
        let name = hex(&crypto::sha256(&[archive]));
        let path = directory.join(format!("{}.zip", &name[..16]));
        if !path.is_file() {
            let partial = directory.join(format!(".{}.partial", &name[..16]));
            fs::write(&partial, archive).map_err(|error| failed(&partial, &error))?;
            fs::rename(&partial, &path).map_err(|error| failed(&path, &error))?;
        }
        Ok(Unpacked {
            id: id.clone(),
            location: location(&path)?,
        })
    }
}

impl ExtensionPackages for ExtensionFiles {
    fn unpack(
        &self,
        source: &PackageSource,
        expected: Option<&Expected>,
    ) -> Result<Unpacked, PackageError> {
        match source {
            PackageSource::Crx(crx) => self.keep_crx(crx, expected),
            PackageSource::File(path) => {
                let bytes = fs::read(path).map_err(|error| failed(path, &error))?;
                if bytes.starts_with(CRX_MAGIC) {
                    self.keep_crx(&bytes, expected)
                } else if bytes.starts_with(ZIP_MAGIC) {
                    let id = id_of(&canonical(path)?);
                    match expected {
                        Some(expected) if expected.id != id => Err(PackageError(format!(
                            "{} is not the extension {}",
                            path.display(),
                            expected.id
                        ))),
                        _ => self.keep(&id, &bytes),
                    }
                } else {
                    Err(PackageError(format!(
                        "{} is neither a .crx nor a .zip package",
                        path.display()
                    )))
                }
            }
            PackageSource::Folder(path) => {
                let folder = canonical(path)?;
                if !folder.join("manifest.json").is_file() {
                    return Err(PackageError(format!(
                        "{} has no manifest.json",
                        path.display()
                    )));
                }
                Ok(Unpacked {
                    id: id_of(&folder),
                    location: location(&folder)?,
                })
            }
        }
    }

    fn prune(&self, keep: &[String]) -> Result<(), PackageError> {
        let Ok(directories) = fs::read_dir(&self.root) else {
            return Ok(());
        };
        for directory in directories.flatten().map(|entry| entry.path()) {
            if !directory.is_dir() {
                continue;
            }
            let files = fs::read_dir(&directory).map_err(|error| failed(&directory, &error))?;
            for file in files.flatten().map(|entry| entry.path()) {
                let kept = file
                    .to_str()
                    .is_some_and(|file| keep.iter().any(|k| k == file));
                if !kept {
                    fs::remove_file(&file).map_err(|error| failed(&file, &error))?;
                }
            }
            // Only an empty folder goes; one still holding a kept package stays.
            let _ = fs::remove_dir(&directory);
        }
        Ok(())
    }
}

/// The id of a package found at `path`: Chrome's for an unpacked extension.
fn id_of(path: &Path) -> ExtensionId {
    let digest = crypto::sha256(&[path.as_os_str().as_bytes()]);
    let mut head = [0_u8; 16];
    head.copy_from_slice(&digest[..16]);
    ExtensionId::from_digest(&head)
}

fn canonical(path: &Path) -> Result<PathBuf, PackageError> {
    path.canonicalize().map_err(|error| failed(path, &error))
}

fn location(path: &Path) -> Result<String, PackageError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| PackageError(format!("{} is not UTF-8", path.display())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn failed(path: &Path, error: &std::io::Error) -> PackageError {
    PackageError(format!("{}: {error}", path.display()))
}

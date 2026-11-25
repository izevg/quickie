//! Trusted per-user runtime path resolution and strict endpoint validation for Unix sockets.

use std::fs::{self, DirBuilder, Metadata};
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

use crate::McpError;

/// Default Quickie directory name created inside the trusted per-user runtime root.
pub const DEFAULT_SOCKET_DIR_NAME: &str = "quickie";

/// Default Unix domain socket file name.
pub const DEFAULT_SOCKET_FILE_NAME: &str = "mcp.sock";

#[cfg(target_os = "macos")]
const MAX_UNIX_SOCKET_PATH_LEN: usize = 103;

#[cfg(not(target_os = "macos"))]
const MAX_UNIX_SOCKET_PATH_LEN: usize = 107;

/// Returns the effective user ID of the current process.
#[must_use]
pub(crate) fn current_euid() -> u32 {
    // SAFETY: `geteuid` is always safe to call and has no preconditions.
    unsafe { libc::geteuid() }
}

/// Resolves the OS-trusted per-user runtime directory without creating any directories or files.
///
/// - On macOS: queries `confstr(_CS_DARWIN_USER_TEMP_DIR)`.
/// - On Linux: uses `XDG_RUNTIME_DIR` when set and valid, otherwise a private per-UID subdirectory
///   under `std::env::temp_dir()`.
///
/// # Errors
///
/// Returns [`McpError::InvalidEndpoint`] if the platform runtime directory cannot be determined or
/// fails ownership/mode validation.
pub fn default_runtime_dir() -> Result<PathBuf, McpError> {
    #[cfg(target_os = "macos")]
    {
        let root = darwin_user_temp_dir()?;
        validate_trusted_root_if_exists(&root, true)?;
        Ok(root)
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR")
            && !xdg.is_empty()
        {
            let path = PathBuf::from(xdg);
            validate_trusted_root_if_exists(&path, true)?;
            return Ok(path);
        }
        let euid = current_euid();
        let fallback = std::env::temp_dir().join(format!("quickie-runtime-{euid}"));
        validate_trusted_root_if_exists(&fallback, false)?;
        Ok(fallback)
    }
}

/// Returns the default Quickie MCP Unix socket path without creating any directories or files.
///
/// # Errors
///
/// Returns [`McpError::InvalidEndpoint`] if the trusted runtime directory is unavailable or invalid.
pub fn default_socket_path() -> Result<PathBuf, McpError> {
    Ok(default_runtime_dir()?
        .join(DEFAULT_SOCKET_DIR_NAME)
        .join(DEFAULT_SOCKET_FILE_NAME))
}

#[cfg(target_os = "macos")]
fn darwin_user_temp_dir() -> Result<PathBuf, McpError> {
    use std::ffi::OsStr;

    // SAFETY: Calling `confstr` with a null buffer and zero length queries the required buffer size.
    let needed = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
    if needed == 0 {
        return Err(McpError::InvalidEndpoint(
            "confstr(_CS_DARWIN_USER_TEMP_DIR) unavailable".to_owned(),
        ));
    }

    let mut buf = vec![0_u8; needed];
    // SAFETY: `buf` is a valid writable buffer of `buf.len()` bytes.
    let written = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast::<libc::c_char>(),
            buf.len(),
        )
    };
    if written == 0 || written > buf.len() {
        return Err(McpError::InvalidEndpoint(
            "confstr(_CS_DARWIN_USER_TEMP_DIR) failed".to_owned(),
        ));
    }

    let bytes = buf[..written].strip_suffix(&[0]).unwrap_or(&buf[..written]);
    if bytes.is_empty() {
        return Err(McpError::InvalidEndpoint(
            "confstr(_CS_DARWIN_USER_TEMP_DIR) returned an empty path".to_owned(),
        ));
    }
    let path = PathBuf::from(OsStr::from_bytes(bytes));
    Ok(strip_trailing_slashes(&path))
}

fn strip_trailing_slashes(path: &Path) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    let trimmed = match bytes.iter().rposition(|&b| b != b'/') {
        Some(idx) => &bytes[..=idx],
        None => bytes,
    };
    PathBuf::from(std::ffi::OsStr::from_bytes(trimmed))
}

/// A validated Unix domain socket endpoint located inside a trusted per-user runtime directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketEndpoint {
    trusted_root: PathBuf,
    relative_dirs: Vec<PathBuf>,
    socket_path: PathBuf,
}

impl SocketEndpoint {
    /// Creates the default Quickie MCP socket endpoint without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::InvalidEndpoint`] if the trusted runtime directory or path is invalid.
    pub fn default_endpoint() -> Result<Self, McpError> {
        let root = default_runtime_dir()?;
        let path = root
            .join(DEFAULT_SOCKET_DIR_NAME)
            .join(DEFAULT_SOCKET_FILE_NAME);
        Self::in_trusted_root(root, path)
    }

    /// Validates an explicit socket path against the OS-trusted per-user runtime directory.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::InvalidEndpoint`] if `socket_path` is not an absolute path inside a
    /// subdirectory of the trusted runtime directory or contains traversal/symlink components.
    pub fn from_path(socket_path: impl Into<PathBuf>) -> Result<Self, McpError> {
        let root = default_runtime_dir()?;
        Self::in_trusted_root(root, socket_path)
    }

    /// Validates `socket_path` under an explicit `trusted_root` per-user runtime directory.
    ///
    /// Does not create any files or directories.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::InvalidEndpoint`] if either path is relative, `trusted_root` fails
    /// ownership/mode checks, `socket_path` escapes `trusted_root`, or `socket_path` is too long
    /// for a Unix domain socket address.
    pub fn in_trusted_root(
        trusted_root: impl Into<PathBuf>,
        socket_path: impl Into<PathBuf>,
    ) -> Result<Self, McpError> {
        let trusted_root = strip_trailing_slashes(&trusted_root.into());
        let socket_path = socket_path.into();

        if !trusted_root.is_absolute() {
            return Err(McpError::InvalidEndpoint(
                "trusted runtime root must be an absolute path".to_owned(),
            ));
        }
        if !socket_path.is_absolute() {
            return Err(McpError::InvalidEndpoint(
                "socket path must be an absolute path".to_owned(),
            ));
        }
        if socket_path.as_os_str().as_bytes().len() > MAX_UNIX_SOCKET_PATH_LEN {
            return Err(McpError::InvalidEndpoint(format!(
                "socket path exceeds {MAX_UNIX_SOCKET_PATH_LEN} bytes"
            )));
        }

        validate_trusted_root_if_exists(&trusted_root, false)?;

        let rel = strip_trusted_prefix(&trusted_root, &socket_path)?;
        let mut components = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(part) => components.push(PathBuf::from(part)),
                Component::ParentDir
                | Component::CurDir
                | Component::RootDir
                | Component::Prefix(_) => {
                    return Err(McpError::InvalidEndpoint(
                        "socket path contains disallowed path component".to_owned(),
                    ));
                }
            }
        }
        if components.len() < 2 {
            return Err(McpError::InvalidEndpoint(
                "socket must live inside a private subdirectory of the runtime root".to_owned(),
            ));
        }

        // Remove the socket file name, leaving the subdirectory chain under `trusted_root`.
        components.pop();
        let canonical_base =
            fs::canonicalize(&trusted_root).unwrap_or_else(|_| trusted_root.clone());
        let mut normalized_socket = canonical_base.clone();
        for part in &components {
            normalized_socket.push(part);
        }
        if let Some(file_name) = socket_path.file_name() {
            normalized_socket.push(file_name);
        }

        if normalized_socket.as_os_str().as_bytes().len() > MAX_UNIX_SOCKET_PATH_LEN {
            return Err(McpError::InvalidEndpoint(format!(
                "normalized socket path exceeds {MAX_UNIX_SOCKET_PATH_LEN} bytes"
            )));
        }

        Ok(Self {
            trusted_root: canonical_base,
            relative_dirs: components,
            socket_path: normalized_socket,
        })
    }

    /// Returns the validated socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Returns the validated parent directory containing the socket and its lock file.
    #[must_use]
    pub fn parent_dir(&self) -> &Path {
        self.socket_path
            .parent()
            .unwrap_or(self.trusted_root.as_path())
    }

    /// Returns the lock file path adjacent to the socket inside the `0700` parent directory.
    #[must_use]
    pub(crate) fn lock_path(&self) -> PathBuf {
        let mut name = self
            .socket_path
            .file_name()
            .map_or_else(|| "mcp.sock".into(), ToOwned::to_owned);
        name.push(".lock");
        self.parent_dir().join(name)
    }

    /// Validates the existing directory chain and socket for a client/relay connect without
    /// creating any files or directories.
    pub(crate) fn validate_for_connect(&self) -> Result<(), McpError> {
        let euid = current_euid();
        match fs::symlink_metadata(&self.trusted_root) {
            Ok(meta) => validate_private_dir_meta(&self.trusted_root, &meta, euid)?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(McpError::EndpointMissing(self.socket_path.clone()));
            }
            Err(err) => return Err(McpError::Io(err)),
        }

        let mut cur = self.trusted_root.clone();
        for part in &self.relative_dirs {
            cur.push(part);
            match fs::symlink_metadata(&cur) {
                Ok(meta) => validate_private_dir_meta(&cur, &meta, euid)?,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    return Err(McpError::EndpointMissing(self.socket_path.clone()));
                }
                Err(err) => return Err(McpError::Io(err)),
            }
        }

        match fs::symlink_metadata(&self.socket_path) {
            Ok(meta) => validate_socket_meta(&self.socket_path, &meta, euid),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(McpError::EndpointMissing(self.socket_path.clone()))
            }
            Err(err) => Err(McpError::Io(err)),
        }
    }

    /// Atomically creates and validates the private `0700` directory chain before binding.
    pub(crate) fn prepare_for_bind(&self) -> Result<(), McpError> {
        let euid = current_euid();
        ensure_private_dir(&self.trusted_root, euid)?;

        let mut cur = self.trusted_root.clone();
        for part in &self.relative_dirs {
            cur.push(part);
            ensure_private_dir(&cur, euid)?;
        }

        match fs::symlink_metadata(&self.socket_path) {
            Ok(meta) => validate_socket_meta(&self.socket_path, &meta, euid),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(McpError::Io(err)),
        }
    }
}

fn strip_trusted_prefix(trusted_root: &Path, socket_path: &Path) -> Result<PathBuf, McpError> {
    if let Ok(rel) = socket_path.strip_prefix(trusted_root) {
        return Ok(rel.to_path_buf());
    }
    if let Ok(canonical_root) = fs::canonicalize(trusted_root)
        && let Ok(rel) = socket_path.strip_prefix(&canonical_root)
    {
        return Ok(rel.to_path_buf());
    }
    Err(McpError::InvalidEndpoint(format!(
        "socket path {} is outside trusted runtime root {}",
        socket_path.display(),
        trusted_root.display()
    )))
}

fn validate_trusted_root_if_exists(root: &Path, must_exist: bool) -> Result<(), McpError> {
    let euid = current_euid();
    match fs::symlink_metadata(root) {
        Ok(meta) => validate_private_dir_meta(root, &meta, euid),
        Err(err) if err.kind() == io::ErrorKind::NotFound && !must_exist => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(McpError::InvalidEndpoint(
            format!("trusted runtime root does not exist: {}", root.display()),
        )),
        Err(err) => Err(McpError::Io(err)),
    }
}

fn ensure_private_dir(path: &Path, euid: u32) -> Result<(), McpError> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(McpError::Io(err)),
    }

    // Open with O_NOFOLLOW | O_DIRECTORY and fstat the opened descriptor to eliminate TOCTOU.
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        McpError::InvalidEndpoint("directory path contains interior NUL".to_owned())
    })?;
    // SAFETY: `c_path` is a valid NUL-terminated path and `open` takes no extra args without O_CREAT.
    let raw_fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw_fd < 0 {
        let err = io::Error::last_os_error();
        if matches!(
            err.raw_os_error(),
            Some(libc::ELOOP | libc::ENOTDIR | libc::EMLINK)
        ) {
            return Err(McpError::InvalidEndpoint(format!(
                "runtime directory is a symlink or not a directory: {}",
                path.display()
            )));
        }
        return Err(McpError::Io(err));
    }
    // SAFETY: `raw_fd` is a newly opened, valid file descriptor owned exclusively here.
    let owned_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let file = fs::File::from(owned_fd);
    let meta = file.metadata().map_err(McpError::Io)?;
    validate_private_dir_meta(path, &meta, euid)
}

pub(crate) fn validate_private_dir_meta(
    path: &Path,
    meta: &Metadata,
    euid: u32,
) -> Result<(), McpError> {
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(McpError::InvalidEndpoint(format!(
            "runtime directory must be a real directory, not a symlink: {}",
            path.display()
        )));
    }
    if meta.uid() != euid {
        return Err(McpError::InvalidEndpoint(format!(
            "runtime directory {} is owned by uid {}, expected {euid}",
            path.display(),
            meta.uid()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(McpError::InvalidEndpoint(format!(
            "runtime directory {} has unsafe permissions {:04o} (expected 0700)",
            path.display(),
            meta.mode() & 0o7777
        )));
    }
    Ok(())
}

pub(crate) fn validate_socket_meta(
    path: &Path,
    meta: &Metadata,
    euid: u32,
) -> Result<(), McpError> {
    if meta.file_type().is_symlink() || !meta.file_type().is_socket() {
        return Err(McpError::InvalidEndpoint(format!(
            "endpoint must be a Unix socket, not a symlink or regular file: {}",
            path.display()
        )));
    }
    if meta.uid() != euid {
        return Err(McpError::InvalidEndpoint(format!(
            "endpoint {} is owned by uid {}, expected {euid}",
            path.display(),
            meta.uid()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(McpError::InvalidEndpoint(format!(
            "endpoint {} has unsafe permissions {:04o} (expected 0600)",
            path.display(),
            meta.mode() & 0o7777
        )));
    }
    Ok(())
}

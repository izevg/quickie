//! Dedicated `flock`-backed lifetime guard for Unix socket bind, stale reclaim, and inode-safe
//! cleanup.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use super::endpoint::{SocketEndpoint, current_euid, validate_socket_meta};
use crate::McpError;

/// Holds the exclusive lock file and records the bound socket's `(dev, ino)` identity.
///
/// Dropping this guard unlinks the socket path **only** if the path still refers to the exact
/// socket inode we bound, so cleanup can never remove a replaced path.
pub(crate) struct SocketGuard {
    socket_path: PathBuf,
    bound_identity: (u64, u64),
    _lock_file: File,
}

impl SocketGuard {
    /// Prepares the private directory, acquires the exclusive lock, performs controlled stale
    /// reclaim if needed, binds the Unix listener, sets mode `0600`, and records the bound inode.
    pub(crate) fn bind(endpoint: &SocketEndpoint) -> Result<(UnixListener, Self), McpError> {
        endpoint.prepare_for_bind()?;

        let lock_file = acquire_lock(&endpoint.lock_path())?;
        let socket_path = endpoint.socket_path().to_path_buf();
        let euid = current_euid();

        match fs::symlink_metadata(&socket_path) {
            Ok(meta) => {
                validate_socket_meta(&socket_path, &meta, euid)?;
                match UnixStream::connect(&socket_path) {
                    Ok(_) => return Err(McpError::EndpointInUse(socket_path)),
                    Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => {
                        fs::remove_file(&socket_path).map_err(McpError::Io)?;
                    }
                    Err(err) => return Err(McpError::Io(err)),
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(McpError::Io(err)),
        }

        let listener = UnixListener::bind(&socket_path).map_err(McpError::Io)?;
        fs::set_permissions(&socket_path, Permissions::from_mode(0o600)).map_err(McpError::Io)?;
        listener.set_nonblocking(true).map_err(McpError::Io)?;

        let meta = fs::symlink_metadata(&socket_path).map_err(McpError::Io)?;
        let bound_identity = (meta.dev(), meta.ino());

        Ok((
            listener,
            Self {
                socket_path,
                bound_identity,
                _lock_file: lock_file,
            },
        ))
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Ok(meta) = fs::symlink_metadata(&self.socket_path)
            && !meta.file_type().is_symlink()
            && meta.file_type().is_socket()
            && (meta.dev(), meta.ino()) == self.bound_identity
        {
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

fn acquire_lock(lock_path: &std::path::Path) -> Result<File, McpError> {
    let euid = current_euid();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(lock_path)
        .map_err(|err| {
            if matches!(err.raw_os_error(), Some(libc::ELOOP | libc::EMLINK)) {
                McpError::InvalidEndpoint(format!(
                    "lock path is a symlink: {}",
                    lock_path.display()
                ))
            } else {
                McpError::Io(err)
            }
        })?;

    let meta = file.metadata().map_err(McpError::Io)?;
    if !meta.is_file() || meta.uid() != euid || (meta.mode() & 0o077 != 0) {
        return Err(McpError::InvalidEndpoint(format!(
            "lock file {} has invalid type, owner, or mode",
            lock_path.display()
        )));
    }

    // SAFETY: `file.as_raw_fd()` is an open file descriptor owned by `file`.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock {
            return Err(McpError::EndpointInUse(lock_path.to_path_buf()));
        }
        return Err(McpError::Io(err));
    }

    Ok(file)
}

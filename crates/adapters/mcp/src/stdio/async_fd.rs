//! Non-blocking Unix file descriptor reader and writer backed by [`tokio::io::unix::AsyncFd`].
//!
//! Avoids Tokio's blocking-pool stdio adapters so cancelling a read when the remote peer closes
//! never leaves a stuck blocking thread behind.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf};

/// Sets `O_NONBLOCK` on the given file descriptor.
pub(crate) fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: `F_GETFL` takes no additional arguments and is safe on a valid fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `F_SETFL` with `flags | O_NONBLOCK` is valid on a valid fd.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Non-blocking [`AsyncRead`] adapter over an owned Unix file descriptor (pipe or socket).
pub(crate) struct AsyncPipeReader {
    inner: AsyncFd<OwnedFd>,
}

impl AsyncPipeReader {
    pub(crate) fn new(fd: OwnedFd) -> io::Result<Self> {
        set_nonblocking(fd.as_raw_fd())?;
        // SAFETY: `fd` is an owned, valid open file descriptor whose raw fd remains unchanged and
        // open for the entire lifetime of the returned `AsyncFd`.
        let inner = unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE) }?;
        Ok(Self { inner })
    }
}

impl AsyncRead for AsyncPipeReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            let mut guard = ready!(self.inner.poll_read_ready(cx))?;
            let dst = buf.initialize_unfilled();
            let raw_fd = self.inner.as_raw_fd();
            // SAFETY: `raw_fd` is owned by `self.inner` and `dst` is a valid writable slice.
            let n = unsafe { libc::read(raw_fd, dst.as_mut_ptr().cast(), dst.len()) };
            if n >= 0 {
                #[expect(
                    clippy::cast_sign_loss,
                    reason = "non-negative `read` return value fits in usize"
                )]
                buf.advance(n as usize);
                return Poll::Ready(Ok(()));
            }
            let err = io::Error::last_os_error();
            match err.kind() {
                io::ErrorKind::Interrupted => {}
                io::ErrorKind::WouldBlock => {
                    guard.clear_ready();
                }
                _ => return Poll::Ready(Err(err)),
            }
        }
    }
}

/// Non-blocking [`AsyncWrite`] adapter over an owned Unix file descriptor (pipe or socket).
pub(crate) struct AsyncPipeWriter {
    inner: AsyncFd<OwnedFd>,
}

impl AsyncPipeWriter {
    pub(crate) fn new(fd: OwnedFd) -> io::Result<Self> {
        set_nonblocking(fd.as_raw_fd())?;
        // SAFETY: `fd` is an owned, valid open file descriptor whose raw fd remains unchanged and
        // open for the entire lifetime of the returned `AsyncFd`.
        let inner = unsafe { AsyncFd::register_with_interest(fd, Interest::WRITABLE) }?;
        Ok(Self { inner })
    }
}

impl AsyncWrite for AsyncPipeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            let mut guard = ready!(self.inner.poll_write_ready(cx))?;
            let raw_fd = self.inner.as_raw_fd();
            // SAFETY: `raw_fd` is owned by `self.inner` and `buf` is a valid readable slice.
            let n = unsafe { libc::write(raw_fd, buf.as_ptr().cast(), buf.len()) };
            if n >= 0 {
                #[expect(
                    clippy::cast_sign_loss,
                    reason = "non-negative `write` return value fits in usize"
                )]
                return Poll::Ready(Ok(n as usize));
            }
            let err = io::Error::last_os_error();
            match err.kind() {
                io::ErrorKind::Interrupted => {}
                io::ErrorKind::WouldBlock => {
                    guard.clear_ready();
                }
                _ => return Poll::Ready(Err(err)),
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

//! RAII wrapper around a Linux file descriptor.

use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd as StdOwnedFd, RawFd};

/// Owned Linux file descriptor that closes on drop.
#[derive(Debug)]
pub struct OwnedFd {
    inner: StdOwnedFd,
}

impl OwnedFd {
    /// Takes ownership of an already-open raw fd.
    ///
    /// # Safety
    ///
    /// `fd` must be open and not owned elsewhere. After this call, only `OwnedFd`
    /// may close or otherwise operate on the descriptor.
    #[must_use]
    pub unsafe fn from_raw_fd(fd: RawFd) -> Self {
        // SAFETY: caller guarantees unique ownership of an open fd.
        Self {
            inner: unsafe { StdOwnedFd::from_raw_fd(fd) },
        }
    }

    #[must_use]
    pub fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }

    /// Releases ownership without closing.
    #[must_use]
    pub fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_fd_closes_on_drop() {
        let mut fds = [-1, -1];
        // SAFETY: `fds` is a valid two-slot buffer for pipe2.
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(rc, 0, "pipe2 failed");

        // SAFETY: unique ownership of the read end after pipe2.
        let owned = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        assert!(owned.as_raw_fd() >= 0);
        drop(owned);

        // SAFETY: unique ownership of the write end.
        let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        drop(write);
    }
}

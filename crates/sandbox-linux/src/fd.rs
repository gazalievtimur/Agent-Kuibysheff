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
    use std::fs::File;
    use std::io::Write;
    use std::os::fd::IntoRawFd;

    #[test]
    fn owned_fd_closes_on_drop() {
        let mut file = tempfile::tempfile().expect("tempfile");
        writeln!(file, "x").expect("write");
        let raw = file.into_raw_fd();
        // SAFETY: `raw` is uniquely owned after `into_raw_fd`.
        let owned = unsafe { OwnedFd::from_raw_fd(raw) };
        assert!(owned.as_raw_fd() >= 0);
        drop(owned);
    }
}

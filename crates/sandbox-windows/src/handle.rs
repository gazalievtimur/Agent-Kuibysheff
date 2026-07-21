//! RAII wrapper around a Windows `HANDLE`.

use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

/// Owned Windows handle that closes on drop.
///
/// Not `Send`/`Sync` yet: later stages will document which Job/process handles may move threads.
#[derive(Debug)]
pub struct OwnedHandle {
    handle: HANDLE,
}

impl OwnedHandle {
    /// Takes ownership of an already-open handle.
    ///
    /// # Safety
    ///
    /// `handle` must be a valid, open Win32 handle not owned elsewhere. After this call,
    /// only `OwnedHandle` may close it.
    #[must_use]
    pub unsafe fn from_raw(handle: HANDLE) -> Self {
        Self { handle }
    }

    #[must_use]
    pub fn as_raw(&self) -> HANDLE {
        self.handle
    }

    /// Releases ownership without closing.
    #[must_use]
    pub fn into_raw(self) -> HANDLE {
        let handle = self.handle;
        std::mem::forget(self);
        handle
    }

    #[must_use]
    pub fn is_invalid(&self) -> bool {
        is_invalid_handle(self.handle)
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !is_invalid_handle(self.handle) {
            // SAFETY: we uniquely own `handle` and CloseHandle is the matching destructor.
            let _ = unsafe { CloseHandle(self.handle) };
            self.handle = ptr::null_mut();
        }
    }
}

fn is_invalid_handle(handle: HANDLE) -> bool {
    handle.is_null() || handle == INVALID_HANDLE_VALUE
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::JobObjects::CreateJobObjectW;

    #[test]
    fn owned_handle_closes_on_drop() {
        // SAFETY: CreateJobObjectW with null args creates an unnamed job; we take ownership.
        let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        assert!(!is_invalid_handle(raw));
        // SAFETY: `raw` is uniquely owned after CreateJobObjectW success.
        let owned = unsafe { OwnedHandle::from_raw(raw) };
        assert!(!owned.is_invalid());
        drop(owned);
    }
}

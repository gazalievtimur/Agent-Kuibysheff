//! Anonymous pipes for stdout/stderr capture.

use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::util::setup_last;
use crate::OwnedHandle;

/// Parent-held read end + child-inherited write end.
pub struct PipePair {
    pub reader: OwnedHandle,
    pub writer: OwnedHandle,
}

impl PipePair {
    /// Creates an inheritable pipe; parent keeps reader, child gets writer.
    pub fn create_inheritable() -> Result<Self, SandboxWindowsError> {
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: TRUE,
        };
        let mut read_raw: HANDLE = ptr::null_mut();
        let mut write_raw: HANDLE = ptr::null_mut();
        // SAFETY: CreatePipe fills two new handles on success.
        let ok = unsafe { CreatePipe(&mut read_raw, &mut write_raw, &mut sa, 0) };
        if ok == 0 || read_raw.is_null() || write_raw.is_null() {
            return Err(setup_last(SandboxStage::Pipes, "CreatePipe"));
        }
        if read_raw == INVALID_HANDLE_VALUE || write_raw == INVALID_HANDLE_VALUE {
            return Err(SandboxWindowsError::setup(
                SandboxStage::Pipes,
                "CreatePipe returned INVALID_HANDLE_VALUE",
            ));
        }

        // Parent reader must not be inherited by the child.
        // SAFETY: clear HANDLE_FLAG_INHERIT on the parent read end.
        let clear = unsafe { SetHandleInformation(read_raw, HANDLE_FLAG_INHERIT, 0) };
        if clear == 0 {
            // SAFETY: close both ends on failure.
            unsafe {
                CloseHandle(read_raw);
                CloseHandle(write_raw);
            }
            return Err(setup_last(SandboxStage::Pipes, "SetHandleInformation"));
        }

        // SAFETY: both handles are uniquely owned after CreatePipe.
        Ok(Self {
            reader: unsafe { OwnedHandle::from_raw(read_raw) },
            writer: unsafe { OwnedHandle::from_raw(write_raw) },
        })
    }
}

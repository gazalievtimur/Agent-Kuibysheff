//! Resolve the per-profile AppContainer folder (already accessible to the package SID).

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;
use std::slice;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::Isolation::GetAppContainerFolderPath;
use windows_sys::Win32::Security::PSID;

use crate::error::{SandboxStage, SandboxWindowsError};

/// Returns the AppContainer profile directory for `package_sid`.
pub fn appcontainer_folder(package_sid: PSID) -> Result<PathBuf, SandboxWindowsError> {
    let mut sid_string: *mut u16 = ptr::null_mut();
    // SAFETY: ConvertSidToStringSidW allocates a LocalAlloc string on success.
    let ok = unsafe { ConvertSidToStringSidW(package_sid, &mut sid_string) };
    if ok == 0 || sid_string.is_null() {
        return Err(SandboxWindowsError::setup(
            SandboxStage::Profile,
            "ConvertSidToStringSidW failed",
        ));
    }

    let mut folder: *mut u16 = ptr::null_mut();
    // SAFETY: GetAppContainerFolderPath allocates a path string for the SID SDDL.
    let hr = unsafe { GetAppContainerFolderPath(sid_string, &mut folder) };
    // SAFETY: free the SDDL string before checking folder.
    unsafe {
        LocalFree(sid_string.cast());
    }
    if hr < 0 || folder.is_null() {
        return Err(SandboxWindowsError::setup(
            SandboxStage::Profile,
            format!("GetAppContainerFolderPath failed hr=0x{hr:08X}"),
        ));
    }

    let path = wide_ptr_to_pathbuf(folder);
    // SAFETY: folder was allocated by GetAppContainerFolderPath.
    unsafe {
        LocalFree(folder.cast());
    }
    Ok(path)
}

fn wide_ptr_to_pathbuf(ptr: *mut u16) -> PathBuf {
    // SAFETY: `ptr` is a valid NUL-terminated PWSTR from userenv.
    let len = unsafe {
        let mut n = 0usize;
        while *ptr.add(n) != 0 {
            n += 1;
        }
        n
    };
    // SAFETY: `ptr` points to `len` UTF-16 code units.
    let slice = unsafe { slice::from_raw_parts(ptr, len) };
    PathBuf::from(OsString::from_wide(slice))
}

//! Capability drop and no_new_privs.

use std::fs;

use libc::{prctl, PR_CAPBSET_DROP, PR_SET_NO_NEW_PRIVS};

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::native::util::errno_err;

const FALLBACK_CAP_LAST_CAP: i32 = 40;

/// Clears ambient/bounding/effective capabilities and enables no_new_privs.
pub fn drop_capabilities() -> Result<(), SandboxLinuxError> {
    // SAFETY: enable no_new_privs so later exec cannot regain privileges.
    if unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(errno_err(SandboxStage::Caps, "PR_SET_NO_NEW_PRIVS"));
    }

    for cap in 0..=kernel_cap_last_cap() {
        // SAFETY: drop each capability from the bounding set (EINVAL means absent).
        let rc = unsafe { prctl(PR_CAPBSET_DROP, cap, 0, 0, 0) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINVAL) {
                return Err(SandboxLinuxError::setup(
                    SandboxStage::Caps,
                    format!("PR_CAPBSET_DROP({cap}): {err}"),
                ));
            }
        }
    }

    // Clear permitted/effective/inheritable via capset with empty sets.
    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    let mut header = CapHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let data = [CapData {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    // SAFETY: capset with version 3 and zeroed data clears all capabilities.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_capset,
            std::ptr::from_mut(&mut header),
            data.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(errno_err(SandboxStage::Caps, "capset"));
    }
    Ok(())
}

fn kernel_cap_last_cap() -> i32 {
    fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .ok()
        .and_then(|raw| raw.trim().parse::<i32>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(FALLBACK_CAP_LAST_CAP)
}

//! AppContainer profile RAII.

use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{FreeSid, PSID};

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::util::to_wide_null;

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const HRESULT_ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;

/// Monotonic suffix so parallel launches never reuse the same profile name.
static PROFILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Owned AppContainer profile + package SID.
pub struct AppContainerProfile {
    name: String,
    sid: PSID,
}

impl AppContainerProfile {
    /// Creates a unique AppContainer profile with an empty capability list.
    pub fn create_unique() -> Result<Self, SandboxWindowsError> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = PROFILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("agent.kuibysheff.sb.{nanos}.{seq}");
        Self::create(&name)
    }

    fn create(name: &str) -> Result<Self, SandboxWindowsError> {
        let wide_name = to_wide_null(name);
        let wide_display = to_wide_null("agent_Kuibysheff sandbox");
        let wide_desc = to_wide_null("Temporary AppContainer for home.run");
        let mut sid: PSID = ptr::null_mut();

        // SAFETY: CreateAppContainerProfile writes an allocated SID into `sid` on success.
        let hr = unsafe {
            CreateAppContainerProfile(
                wide_name.as_ptr(),
                wide_display.as_ptr(),
                wide_desc.as_ptr(),
                ptr::null(),
                0,
                &mut sid,
            )
        };

        if hr == HRESULT_ALREADY_EXISTS {
            sid = derive_sid(&wide_name)?;
        } else if hr < 0 {
            return Err(SandboxWindowsError::setup(
                SandboxStage::Profile,
                format!("CreateAppContainerProfile failed hr=0x{hr:08X}"),
            ));
        } else if sid.is_null() {
            return Err(SandboxWindowsError::setup(
                SandboxStage::Profile,
                "CreateAppContainerProfile returned a null SID",
            ));
        }

        Ok(Self {
            name: name.to_string(),
            sid,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn sid(&self) -> PSID {
        self.sid
    }
}

impl Drop for AppContainerProfile {
    fn drop(&mut self) {
        if !self.sid.is_null() {
            // SAFETY: SID was allocated by userenv and is uniquely owned.
            unsafe {
                FreeSid(self.sid);
            }
            self.sid = ptr::null_mut();
        }
        let wide = to_wide_null(&self.name);
        // SAFETY: DeleteAppContainerProfile accepts a NUL-terminated profile name.
        let _ = unsafe { DeleteAppContainerProfile(wide.as_ptr()) };
    }
}

fn derive_sid(wide_name: &[u16]) -> Result<PSID, SandboxWindowsError> {
    let mut sid: PSID = ptr::null_mut();
    // SAFETY: DeriveAppContainerSidFromAppContainerName allocates a SID on success.
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(wide_name.as_ptr(), &mut sid) };
    if hr < 0 || sid.is_null() {
        return Err(SandboxWindowsError::setup(
            SandboxStage::Profile,
            format!("DeriveAppContainerSidFromAppContainerName failed hr=0x{hr:08X}"),
        ));
    }
    Ok(sid)
}

/// Best-effort cleanup of a leftover profile name from a prior crash journal.
pub fn delete_profile_name(name: &str) {
    let wide = to_wide_null(name);
    // SAFETY: best-effort cleanup of a NUL-terminated profile name.
    let _ = unsafe { DeleteAppContainerProfile(wide.as_ptr()) };
}

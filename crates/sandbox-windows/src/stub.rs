//! Non-Windows stub so the crate remains a workspace member on other hosts.

use crate::error::SandboxWindowsError;
use crate::request::{SandboxLaunchRequest, SandboxLaunchResult};

/// Stub Windows sandbox API on non-Windows hosts.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsSandbox;

impl WindowsSandbox {
    /// # Errors
    ///
    /// Always unavailable off Windows.
    pub fn probe() -> Result<(), SandboxWindowsError> {
        Err(SandboxWindowsError::unavailable(
            "windows sandbox is only available on Windows hosts",
        ))
    }

    /// # Errors
    ///
    /// Always unavailable off Windows.
    pub fn run(
        _request: &SandboxLaunchRequest,
    ) -> Result<SandboxLaunchResult, SandboxWindowsError> {
        Err(SandboxWindowsError::unavailable(
            "windows sandbox is only available on Windows hosts",
        ))
    }
}

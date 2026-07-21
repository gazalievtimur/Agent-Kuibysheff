//! Non-Linux stub so the crate remains a workspace member on other hosts.

use crate::error::SandboxLinuxError;
use crate::request::{SandboxLaunchRequest, SandboxLaunchResult};

/// Stub Linux sandbox API on non-Linux hosts.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxSandbox;

impl LinuxSandbox {
    /// # Errors
    ///
    /// Always unavailable off Linux.
    pub fn probe() -> Result<(), SandboxLinuxError> {
        Err(SandboxLinuxError::unavailable(
            "linux sandbox is only available on Linux hosts",
        ))
    }

    /// # Errors
    ///
    /// Always unavailable off Linux.
    pub fn run(_request: &SandboxLaunchRequest) -> Result<SandboxLaunchResult, SandboxLinuxError> {
        Err(SandboxLinuxError::unavailable(
            "linux sandbox is only available on Linux hosts",
        ))
    }
}

/// No-op off Linux (helper mode is Linux-only).
pub fn try_run_helper() {}

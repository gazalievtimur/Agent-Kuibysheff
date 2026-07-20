//! Windows AppContainer sandbox orchestration.

mod acl;
mod folder;
mod job;
mod journal;
mod pipes;
mod process;
mod profile;
mod util;

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::request::{SandboxLaunchRequest, SandboxLaunchResult};

use self::acl::AclJournal;
use self::job::JobObject;
use self::journal::{reclaim_stale_profiles, ProfileJournalEntry};
use self::pipes::PipePair;
use self::process::{assert_no_loopback_exemption, run_sandboxed};
use self::profile::AppContainerProfile;

/// Safe entry points for the Windows AppContainer backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsSandbox;

impl WindowsSandbox {
    /// Probes whether AppContainer + Job Object + network isolation APIs work.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxWindowsError::Unavailable`] when the host cannot run the sandbox.
    pub fn probe() -> Result<(), SandboxWindowsError> {
        reclaim_stale_profiles();

        JobObject::create(true, None)
            .map_err(|err| SandboxWindowsError::unavailable(format!("job probe failed: {err}")))?;
        PipePair::create_inheritable()
            .map_err(|err| SandboxWindowsError::unavailable(format!("pipe probe failed: {err}")))?;

        let profile = AppContainerProfile::create_unique().map_err(|err| {
            SandboxWindowsError::unavailable(format!("profile probe failed: {err}"))
        })?;
        assert_no_loopback_exemption(profile.sid()).map_err(|err| match err {
            SandboxWindowsError::Unavailable { reason } => SandboxWindowsError::Unavailable {
                reason: format!("network isolation probe failed: {reason}"),
            },
            other => SandboxWindowsError::unavailable(format!("network isolation probe: {other}")),
        })?;
        drop(profile);

        Ok(())
    }

    /// Runs `request` inside a fresh AppContainer + Job Object.
    ///
    /// # Errors
    ///
    /// Fail-closed on any setup, policy, or cleanup-critical failure.
    pub fn run(request: &SandboxLaunchRequest) -> Result<SandboxLaunchResult, SandboxWindowsError> {
        validate_request(request)?;
        reclaim_stale_profiles();

        let profile = AppContainerProfile::create_unique()?;
        let _journal_entry = ProfileJournalEntry::create(profile.name()).map_err(|err| {
            SandboxWindowsError::setup(SandboxStage::Profile, format!("profile journal: {err}"))
        })?;

        let mut acl_journal = AclJournal::new();
        let result = run_sandboxed(request, &profile, &mut acl_journal);
        acl_journal.restore_all();
        result
    }
}

fn validate_request(request: &SandboxLaunchRequest) -> Result<(), SandboxWindowsError> {
    if !request.executable.is_absolute() {
        return Err(SandboxWindowsError::PolicyDenied {
            reason: "executable must be an absolute path".to_string(),
        });
    }
    if !request.cwd.is_absolute() {
        return Err(SandboxWindowsError::PolicyDenied {
            reason: "cwd must be an absolute path".to_string(),
        });
    }
    if !request.executable.exists() {
        return Err(SandboxWindowsError::PolicyDenied {
            reason: format!("executable not found: {}", request.executable.display()),
        });
    }
    if request.max_output_chars == 0 {
        return Err(SandboxWindowsError::PolicyDenied {
            reason: "max_output_chars must be non-zero".to_string(),
        });
    }
    Ok(())
}

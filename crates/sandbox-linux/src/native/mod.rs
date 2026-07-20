//! Linux sandbox orchestration (helper re-exec + namespace PID1).

mod caps;
mod clone;
mod helper;
mod mount;
mod parent;
mod pid1;
mod probe;
mod seccomp;
mod userns;
mod util;

use crate::error::SandboxLinuxError;
use crate::protocol::{HelperRequest, HELPER_ENV, REQUEST_ENV};
use crate::request::{SandboxLaunchRequest, SandboxLaunchResult};

use self::helper::run_helper;
use self::parent::run_via_helper;
use self::probe::probe_primitives;

/// Safe entry points for the Linux namespace backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxSandbox;

impl LinuxSandbox {
    /// Probes whether required kernel primitives appear available.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxLinuxError::Unavailable`] when the host cannot run the sandbox.
    pub fn probe() -> Result<(), SandboxLinuxError> {
        probe_primitives()
    }

    /// Runs `request` inside a fresh set of namespaces via helper re-exec.
    ///
    /// # Errors
    ///
    /// Fail-closed on any setup, policy, or cleanup-critical failure.
    pub fn run(request: &SandboxLaunchRequest) -> Result<SandboxLaunchResult, SandboxLinuxError> {
        run_via_helper(request)
    }
}

/// If this process was re-exec'd as the sandbox helper, run it and exit.
///
/// Call this at the very start of `main` **before** starting a Tokio runtime.
pub fn try_run_helper() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    let code = match helper_main() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("sandbox-linux helper failed: {err}");
            78
        }
    };
    // SAFETY: helper must not return into the agent runtime.
    std::process::exit(code);
}

fn helper_main() -> Result<i32, SandboxLinuxError> {
    let path = std::env::var_os(REQUEST_ENV)
        .ok_or_else(|| SandboxLinuxError::unavailable("helper missing request path env"))?;
    let file = std::fs::File::open(&path)
        .map_err(|err| SandboxLinuxError::unavailable(format!("open helper request: {err}")))?;
    let request: HelperRequest = serde_json::from_reader(file)
        .map_err(|err| SandboxLinuxError::unavailable(format!("parse helper request: {err}")))?;
    run_helper(request)
}

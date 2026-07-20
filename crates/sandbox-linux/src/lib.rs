//! Linux namespace sandbox FFI for `home.run`.
//!
//! Platform `unsafe` stays in this crate. The agent root crate remains
//! `unsafe_code = "forbid"` and talks only to the safe API below.
//!
//! The parent process re-execs the current executable as a single-threaded
//! helper (`try_run_helper`) so namespace setup never runs inside Tokio
//! `pre_exec` hooks.

#![deny(unsafe_op_in_unsafe_fn)]

mod error;
mod request;

#[cfg(target_os = "linux")]
mod fd;
#[cfg(target_os = "linux")]
mod native;
#[cfg(target_os = "linux")]
mod protocol;

#[cfg(not(target_os = "linux"))]
mod stub;

pub use error::{SandboxLinuxError, SandboxStage};
pub use request::{SandboxLaunchRequest, SandboxLaunchResult};

#[cfg(target_os = "linux")]
pub use fd::OwnedFd;
#[cfg(target_os = "linux")]
pub use native::{try_run_helper, LinuxSandbox};

#[cfg(not(target_os = "linux"))]
pub use stub::{try_run_helper, LinuxSandbox};

/// Auto-enter helper mode for any binary that links this crate (including `cargo test`).
///
/// `LinuxSandbox::run` re-execs `current_exe` with helper env vars; without this constructor
/// the child would re-enter the test harness instead of the sandbox supervisor.
#[cfg(target_os = "linux")]
#[used]
#[link_section = ".init_array"]
static SANDBOX_LINUX_HELPER_INIT: extern "C" fn() = {
    extern "C" fn sandbox_linux_helper_init() {
        try_run_helper();
    }
    sandbox_linux_helper_init
};

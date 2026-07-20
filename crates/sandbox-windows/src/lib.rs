//! Windows `AppContainer` sandbox FFI for `home.run`.
//!
//! Platform `unsafe` stays in this crate. The agent root crate remains
//! `unsafe_code = "forbid"` and talks only to the safe API below.

#![deny(unsafe_op_in_unsafe_fn)]

mod error;
mod request;

#[cfg(windows)]
mod handle;
#[cfg(windows)]
mod native;

#[cfg(not(windows))]
mod stub;

pub use error::{SandboxStage, SandboxWindowsError};
pub use request::{SandboxLaunchRequest, SandboxLaunchResult};

#[cfg(windows)]
pub use handle::OwnedHandle;
#[cfg(windows)]
pub use native::WindowsSandbox;

#[cfg(not(windows))]
pub use stub::WindowsSandbox;

//! Adapters from agent [`SandboxSpec`] to `sandbox-linux` / `sandbox-windows` safe APIs.

use super::{SandboxBackend, SandboxError, SandboxOutput, SandboxSpec};

#[cfg(target_os = "linux")]
mod linux {
    use super::{SandboxBackend, SandboxError, SandboxOutput, SandboxSpec};
    use async_trait::async_trait;
    use sandbox_linux::{LinuxSandbox, SandboxLaunchRequest, SandboxLinuxError};

    #[derive(Debug, Default)]
    pub struct NativeLinuxBackend;

    #[async_trait]
    impl SandboxBackend for NativeLinuxBackend {
        fn probe(&self) -> Result<(), SandboxError> {
            LinuxSandbox::probe().map_err(map_linux_error)
        }

        async fn run(&self, spec: SandboxSpec) -> Result<SandboxOutput, SandboxError> {
            let request = to_linux_request(spec);
            let result = tokio::task::spawn_blocking(move || LinuxSandbox::run(&request))
                .await
                .map_err(|err| SandboxError::Io {
                    reason: format!("linux sandbox join failed: {err}"),
                })?
                .map_err(map_linux_error)?;
            Ok(SandboxOutput {
                stdout: result.stdout,
                stderr: result.stderr,
                stdout_truncated: result.stdout_truncated,
                stderr_truncated: result.stderr_truncated,
                exit_code: result.exit_code,
                timed_out: result.timed_out,
            })
        }
    }

    fn to_linux_request(spec: SandboxSpec) -> SandboxLaunchRequest {
        SandboxLaunchRequest {
            executable: spec.executable,
            argv: spec.argv,
            cwd: spec.cwd,
            env: spec.env,
            home_read: spec.home_read,
            home_write: spec.home_write,
            runtime_read_roots: spec
                .runtime_read_roots
                .into_iter()
                .map(|root| root.as_path().to_path_buf())
                .collect(),
            deadline: spec.deadline,
            max_output_chars: spec.max_output_chars,
            allow_children: spec.allow_children,
        }
    }

    fn map_linux_error(error: SandboxLinuxError) -> SandboxError {
        match error {
            SandboxLinuxError::Unavailable { reason } => SandboxError::Unavailable { reason },
            SandboxLinuxError::PolicyDenied { reason } => SandboxError::PolicyDenied { reason },
            SandboxLinuxError::Setup { stage, reason } => SandboxError::Setup {
                stage: stage.to_string(),
                reason,
            },
            SandboxLinuxError::Io { reason } => SandboxError::Io { reason },
            SandboxLinuxError::TimeoutCleanup { reason } => SandboxError::TimeoutCleanup { reason },
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::{SandboxBackend, SandboxError, SandboxOutput, SandboxSpec};
    use async_trait::async_trait;
    use sandbox_windows::{SandboxLaunchRequest, SandboxWindowsError, WindowsSandbox};

    #[derive(Debug, Default)]
    pub struct NativeWindowsBackend;

    #[async_trait]
    impl SandboxBackend for NativeWindowsBackend {
        fn probe(&self) -> Result<(), SandboxError> {
            WindowsSandbox::probe().map_err(map_windows_error)
        }

        async fn run(&self, spec: SandboxSpec) -> Result<SandboxOutput, SandboxError> {
            let request = to_windows_request(spec);
            let result = tokio::task::spawn_blocking(move || WindowsSandbox::run(&request))
                .await
                .map_err(|err| SandboxError::Io {
                    reason: format!("windows sandbox join failed: {err}"),
                })?
                .map_err(map_windows_error)?;
            Ok(SandboxOutput {
                stdout: result.stdout,
                stderr: result.stderr,
                stdout_truncated: result.stdout_truncated,
                stderr_truncated: result.stderr_truncated,
                exit_code: result.exit_code,
                timed_out: result.timed_out,
            })
        }
    }

    fn to_windows_request(spec: SandboxSpec) -> SandboxLaunchRequest {
        SandboxLaunchRequest {
            executable: spec.executable,
            argv: spec.argv,
            cwd: spec.cwd,
            env: spec.env,
            home_read: spec.home_read,
            home_write: spec.home_write,
            runtime_read_roots: spec
                .runtime_read_roots
                .into_iter()
                .map(|root| root.as_path().to_path_buf())
                .collect(),
            deadline: spec.deadline,
            max_output_chars: spec.max_output_chars,
            allow_children: spec.allow_children,
        }
    }

    fn map_windows_error(error: SandboxWindowsError) -> SandboxError {
        match error {
            SandboxWindowsError::Unavailable { reason } => SandboxError::Unavailable { reason },
            SandboxWindowsError::PolicyDenied { reason } => SandboxError::PolicyDenied { reason },
            SandboxWindowsError::Setup { stage, reason } => SandboxError::Setup {
                stage: stage.to_string(),
                reason,
            },
            SandboxWindowsError::Io { reason } => SandboxError::Io { reason },
            SandboxWindowsError::TimeoutCleanup { reason } => {
                SandboxError::TimeoutCleanup { reason }
            }
        }
    }
}

/// Selects the host-native sandbox backend.
#[must_use]
pub fn native_backend() -> std::sync::Arc<dyn SandboxBackend> {
    #[cfg(target_os = "linux")]
    {
        std::sync::Arc::new(linux::NativeLinuxBackend)
    }
    #[cfg(target_os = "windows")]
    {
        std::sync::Arc::new(windows::NativeWindowsBackend)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        std::sync::Arc::new(super::UnavailableBackend {
            reason: format!("sandbox unsupported on `{}`", std::env::consts::OS),
        })
    }
}

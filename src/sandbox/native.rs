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

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::access::ProgramAlias;
        use sandbox_linux::SandboxStage;
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        use std::time::Duration;

        #[test]
        fn to_linux_request_copies_fields() {
            let spec = SandboxSpec {
                alias: ProgramAlias::parse("fixture").unwrap(),
                executable: PathBuf::from("/bin/true"),
                argv: vec!["a".into()],
                cwd: PathBuf::from("/tmp"),
                env: BTreeMap::from([("K".into(), "V".into())]),
                home_read: vec![PathBuf::from("/r")],
                home_write: vec![PathBuf::from("/w")],
                runtime_read_roots: Vec::new(),
                deadline: Duration::from_secs(3),
                max_output_chars: 99,
                allow_children: true,
            };
            let req = to_linux_request(spec);
            assert_eq!(req.executable, PathBuf::from("/bin/true"));
            assert_eq!(req.argv, vec!["a".to_string()]);
            assert_eq!(req.cwd, PathBuf::from("/tmp"));
            assert_eq!(req.env.get("K").map(String::as_str), Some("V"));
            assert_eq!(req.home_read, vec![PathBuf::from("/r")]);
            assert_eq!(req.home_write, vec![PathBuf::from("/w")]);
            assert!(req.runtime_read_roots.is_empty());
            assert_eq!(req.deadline, Duration::from_secs(3));
            assert_eq!(req.max_output_chars, 99);
            assert!(req.allow_children);
        }

        #[test]
        fn map_linux_error_preserves_variants() {
            assert!(matches!(
                map_linux_error(SandboxLinuxError::Unavailable { reason: "x".into() }),
                SandboxError::Unavailable { .. }
            ));
            assert!(matches!(
                map_linux_error(SandboxLinuxError::PolicyDenied { reason: "x".into() }),
                SandboxError::PolicyDenied { .. }
            ));
            assert!(matches!(
                map_linux_error(SandboxLinuxError::Setup {
                    stage: SandboxStage::Seccomp,
                    reason: "x".into()
                }),
                SandboxError::Setup { .. }
            ));
            assert!(matches!(
                map_linux_error(SandboxLinuxError::Io { reason: "x".into() }),
                SandboxError::Io { .. }
            ));
            assert!(matches!(
                map_linux_error(SandboxLinuxError::TimeoutCleanup { reason: "x".into() }),
                SandboxError::TimeoutCleanup { .. }
            ));
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::access::ProgramAlias;
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        use std::time::Duration;

        #[test]
        fn to_windows_request_copies_fields() {
            let spec = SandboxSpec {
                alias: ProgramAlias::parse("fixture").unwrap(),
                executable: PathBuf::from("C:\\Windows\\System32\\cmd.exe"),
                argv: vec!["/c".into(), "echo".into()],
                cwd: PathBuf::from("C:\\Temp"),
                env: BTreeMap::from([("K".into(), "V".into())]),
                home_read: vec![PathBuf::from("C:\\r")],
                home_write: vec![PathBuf::from("C:\\w")],
                runtime_read_roots: Vec::new(),
                deadline: Duration::from_secs(3),
                max_output_chars: 99,
                allow_children: false,
            };
            let req = to_windows_request(spec);
            assert_eq!(req.argv, vec!["/c".to_string(), "echo".to_string()]);
            assert_eq!(req.env.get("K").map(String::as_str), Some("V"));
            assert_eq!(req.max_output_chars, 99);
            assert!(!req.allow_children);
        }

        #[test]
        fn map_windows_error_preserves_variants() {
            assert!(matches!(
                map_windows_error(SandboxWindowsError::Unavailable { reason: "x".into() }),
                SandboxError::Unavailable { .. }
            ));
            assert!(matches!(
                map_windows_error(SandboxWindowsError::PolicyDenied { reason: "x".into() }),
                SandboxError::PolicyDenied { .. }
            ));
            assert!(matches!(
                map_windows_error(SandboxWindowsError::Setup {
                    stage: "job",
                    reason: "x".into()
                }),
                SandboxError::Setup { .. }
            ));
            assert!(matches!(
                map_windows_error(SandboxWindowsError::Io { reason: "x".into() }),
                SandboxError::Io { .. }
            ));
            assert!(matches!(
                map_windows_error(SandboxWindowsError::TimeoutCleanup { reason: "x".into() }),
                SandboxError::TimeoutCleanup { .. }
            ));
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

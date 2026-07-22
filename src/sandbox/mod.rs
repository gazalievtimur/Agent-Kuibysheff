//! Fail-closed process sandbox facade for `home.run`.
//!
//! Production payloads never use plain `tokio::process::Command`. Native backends live in
//! `sandbox-linux` / `sandbox-windows` and are selected by [`SandboxRunner::platform_default`].

mod collect;
mod native;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tracing::info;

use crate::access::{CanonicalRoot, PathGrantScope, ProgramAlias, RelativeGrant};

pub use collect::{collect_utf8_bounded, truncate_utf8_chars};

/// Request describing one sandboxed payload launch.
#[derive(Debug, Clone)]
pub struct SandboxSpec {
    pub alias: ProgramAlias,
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Absolute host paths for read binds (already resolved under `--home`).
    pub home_read: Vec<PathBuf>,
    /// Absolute host paths for write binds (already resolved under `--home`).
    pub home_write: Vec<PathBuf>,
    pub runtime_read_roots: Vec<CanonicalRoot>,
    pub deadline: Duration,
    pub max_output_chars: usize,
    pub allow_children: bool,
}

/// Captured result of a sandboxed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxOutput {
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// Failures from sandbox setup, policy, I/O, or cleanup.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SandboxError {
    #[error("sandbox unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("sandbox policy denied: {reason}")]
    PolicyDenied { reason: String },
    #[error("sandbox setup failed at stage `{stage}`: {reason}")]
    Setup { stage: String, reason: String },
    #[error("sandbox I/O error: {reason}")]
    Io { reason: String },
    #[error("sandbox timed out and process-tree cleanup failed: {reason}")]
    TimeoutCleanup { reason: String },
}

/// Object-safe sandbox backend; `async_trait` is required because native `async fn` in traits is not
/// dyn-compatible for `Arc<dyn SandboxBackend>`.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Confirms the backend can enforce the sandbox before any payload starts.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Unavailable`] or setup errors when the sandbox cannot run.
    fn probe(&self) -> Result<(), SandboxError>;

    /// Runs the payload inside the sandbox.
    ///
    /// # Errors
    ///
    /// Returns policy, setup, I/O, or timeout-cleanup failures from the backend.
    async fn run(&self, spec: SandboxSpec) -> Result<SandboxOutput, SandboxError>;
}

/// Safe facade selecting and invoking a [`SandboxBackend`].
#[derive(Clone)]
pub struct SandboxRunner {
    backend: Arc<dyn SandboxBackend>,
}

impl SandboxRunner {
    /// Creates a runner with an explicit backend (tests and future native wiring).
    #[must_use]
    pub fn with_backend(backend: Arc<dyn SandboxBackend>) -> Self {
        Self { backend }
    }

    /// Production default: host-native FFI backend (fail-closed until fully implemented).
    #[must_use]
    pub fn platform_default() -> Self {
        Self::with_backend(native::native_backend())
    }

    /// Confirms the backing sandbox is ready.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Unavailable`] (or setup errors) when the backend cannot run.
    pub fn probe(&self) -> Result<(), SandboxError> {
        self.backend.probe()
    }

    /// Validates argv invariants then dispatches to the backend.
    ///
    /// # Errors
    ///
    /// Returns policy/setup/I/O errors from validation or the backend.
    pub async fn run(&self, spec: SandboxSpec) -> Result<SandboxOutput, SandboxError> {
        self.backend.probe()?;
        validate_spec(&spec)?;
        info!(
            capability = "home.run",
            alias = %spec.alias,
            deadline_ms = u64::try_from(spec.deadline.as_millis()).unwrap_or(u64::MAX),
            allow_children = spec.allow_children,
            "sandbox launch allowed"
        );
        match self.backend.run(spec).await {
            Ok(output) => {
                info!(
                    capability = "home.run",
                    exit_code = ?output.exit_code,
                    timed_out = output.timed_out,
                    stdout_truncated = output.stdout_truncated,
                    stderr_truncated = output.stderr_truncated,
                    "sandbox launch finished"
                );
                Ok(output)
            }
            Err(err) => {
                info!(
                    capability = "home.run",
                    error = %err,
                    "sandbox launch denied_or_failed"
                );
                Err(err)
            }
        }
    }
}

/// Resolves home path grants to absolute directories under `home_root` for the OS sandbox.
///
/// Legacy scopes grant the entire home root. Strict empty grants mean deny-all (no binds).
#[must_use]
pub fn absolute_home_grants(home_root: &Path, scope: &PathGrantScope) -> Vec<PathBuf> {
    if scope.is_legacy() {
        return vec![home_root.to_path_buf()];
    }
    scope
        .grants()
        .iter()
        .map(|grant| absolute_grant_path(home_root, grant))
        .collect()
}

fn absolute_grant_path(home_root: &Path, grant: &RelativeGrant) -> PathBuf {
    let relative = grant.as_path();
    if relative.as_os_str().is_empty() || relative == Path::new(".") {
        home_root.to_path_buf()
    } else {
        home_root.join(relative)
    }
}

fn validate_spec(spec: &SandboxSpec) -> Result<(), SandboxError> {
    if spec.executable.as_os_str().is_empty() {
        return Err(SandboxError::PolicyDenied {
            reason: "executable path must not be empty".to_string(),
        });
    }
    for arg in &spec.argv {
        if arg.contains('\0') {
            return Err(SandboxError::PolicyDenied {
                reason: "argv must not contain NUL bytes".to_string(),
            });
        }
    }
    for key in spec.env.keys() {
        if is_forbidden_env_key(key) {
            return Err(SandboxError::PolicyDenied {
                reason: format!("environment key `{key}` is forbidden in sandbox"),
            });
        }
    }
    if spec.deadline.is_zero() {
        return Err(SandboxError::PolicyDenied {
            reason: "deadline must be > 0".to_string(),
        });
    }
    if spec.max_output_chars == 0 {
        return Err(SandboxError::PolicyDenied {
            reason: "max_output_chars must be > 0".to_string(),
        });
    }
    Ok(())
}

fn is_forbidden_env_key(key: &str) -> bool {
    const FORBIDDEN: &[&str] = &[
        "LD_PRELOAD",
        "LD_AUDIT",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "DYLD_FORCE_FLAT_NAMESPACE",
    ];
    FORBIDDEN
        .iter()
        .any(|forbidden| key.eq_ignore_ascii_case(forbidden))
}

/// Backend that always reports unavailable (fail-closed default).
#[derive(Debug, Clone)]
pub struct UnavailableBackend {
    pub reason: String,
}

#[async_trait]
impl SandboxBackend for UnavailableBackend {
    fn probe(&self) -> Result<(), SandboxError> {
        Err(SandboxError::Unavailable {
            reason: self.reason.clone(),
        })
    }

    async fn run(&self, _spec: SandboxSpec) -> Result<SandboxOutput, SandboxError> {
        Err(SandboxError::Unavailable {
            reason: self.reason.clone(),
        })
    }
}

/// Test backend that returns a fixed output or invokes a callback.
pub struct MockBackend {
    output: Result<SandboxOutput, SandboxError>,
}

impl MockBackend {
    #[must_use]
    pub fn with_output(output: SandboxOutput) -> Self {
        Self { output: Ok(output) }
    }

    #[must_use]
    pub fn with_error(error: SandboxError) -> Self {
        Self { output: Err(error) }
    }
}

#[async_trait]
impl SandboxBackend for MockBackend {
    fn probe(&self) -> Result<(), SandboxError> {
        Ok(())
    }

    async fn run(&self, _spec: SandboxSpec) -> Result<SandboxOutput, SandboxError> {
        self.output.clone()
    }
}

/// Builds the clean environment block for a sandboxed process.
#[must_use]
pub fn build_sandbox_env(
    home: &std::path::Path,
    inherit_keys: &[String],
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), home.display().to_string());
    env.insert("TMP".to_string(), home.join("tmp").display().to_string());
    env.insert("TEMP".to_string(), home.join("tmp").display().to_string());
    for key in inherit_keys {
        if is_forbidden_env_key(key) {
            continue;
        }
        if let Ok(value) = std::env::var(key) {
            env.insert(key.clone(), value);
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{PathGrantScope, RelativeGrant};
    use std::time::Duration;

    fn sample_spec() -> SandboxSpec {
        SandboxSpec {
            alias: ProgramAlias::parse("python").unwrap(),
            executable: PathBuf::from("python-stub"),
            argv: vec!["-c".to_string(), "print(1)".to_string()],
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
            home_read: Vec::new(),
            home_write: Vec::new(),
            runtime_read_roots: Vec::new(),
            deadline: Duration::from_secs(1),
            max_output_chars: 100,
            allow_children: false,
        }
    }

    #[test]
    fn absolute_home_grants_legacy_uses_root() {
        let root = PathBuf::from("/tmp/home");
        let grants = absolute_home_grants(&root, &PathGrantScope::legacy());
        assert_eq!(grants, vec![root]);
    }

    #[test]
    fn absolute_home_grants_strict_joins_prefixes() {
        let root = PathBuf::from("/tmp/home");
        let scope = PathGrantScope::strict(vec![RelativeGrant::parse("out").unwrap()]);
        let grants = absolute_home_grants(&root, &scope);
        assert_eq!(grants, vec![PathBuf::from("/tmp/home/out")]);
    }

    #[tokio::test]
    async fn platform_default_probe_matches_host() {
        let runner = SandboxRunner::platform_default();
        let result = runner.probe();
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            // Native backends may still be unavailable on locked-down hosts; accept Ok or Unavailable.
            if let Err(err) = result {
                assert!(
                    matches!(err, SandboxError::Unavailable { .. }),
                    "unexpected probe error: {err}"
                );
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let err = result.expect_err("unsupported hosts remain fail-closed");
            assert!(matches!(err, SandboxError::Unavailable { .. }));
        }
    }

    #[tokio::test]
    async fn mock_backend_returns_output() {
        let expected = SandboxOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            exit_code: Some(0),
            timed_out: false,
        };
        let runner =
            SandboxRunner::with_backend(Arc::new(MockBackend::with_output(expected.clone())));
        let out = runner.run(sample_spec()).await.expect("run");
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn rejects_nul_in_argv() {
        let runner =
            SandboxRunner::with_backend(Arc::new(MockBackend::with_output(SandboxOutput {
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                exit_code: Some(0),
                timed_out: false,
            })));
        let mut spec = sample_spec();
        spec.argv = vec!["a\0b".to_string()];
        let err = runner.run(spec).await.expect_err("nul");
        assert!(matches!(err, SandboxError::PolicyDenied { .. }));
    }

    #[tokio::test]
    async fn rejects_forbidden_env_keys() {
        let runner =
            SandboxRunner::with_backend(Arc::new(MockBackend::with_output(SandboxOutput {
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                exit_code: Some(0),
                timed_out: false,
            })));
        let mut spec = sample_spec();
        spec.env
            .insert("LD_PRELOAD".to_string(), "/tmp/x.so".to_string());
        let err = runner.run(spec).await.expect_err("forbidden env");
        assert!(matches!(err, SandboxError::PolicyDenied { .. }));
    }
}

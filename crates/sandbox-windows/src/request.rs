use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// Safe launch request crossing the agent ↔ Windows FFI boundary.
#[derive(Debug, Clone)]
pub struct SandboxLaunchRequest {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub home_read: Vec<PathBuf>,
    pub home_write: Vec<PathBuf>,
    pub runtime_read_roots: Vec<PathBuf>,
    pub deadline: Duration,
    pub max_output_chars: usize,
    pub allow_children: bool,
}

/// Safe launch result returned to the agent facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxLaunchResult {
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

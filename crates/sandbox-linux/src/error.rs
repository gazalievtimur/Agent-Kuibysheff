use thiserror::Error;

/// Setup stage identifiers for fail-closed diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxStage {
    Probe,
    Helper,
    Clone,
    UserMap,
    Mount,
    PivotRoot,
    Caps,
    Seccomp,
    Exec,
    Reap,
}

impl SandboxStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Helper => "helper",
            Self::Clone => "clone",
            Self::UserMap => "user_map",
            Self::Mount => "mount",
            Self::PivotRoot => "pivot_root",
            Self::Caps => "caps",
            Self::Seccomp => "seccomp",
            Self::Exec => "exec",
            Self::Reap => "reap",
        }
    }
}

/// Errors from the Linux sandbox backend.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SandboxLinuxError {
    #[error("linux sandbox unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("linux sandbox policy denied: {reason}")]
    PolicyDenied { reason: String },
    #[error("linux sandbox setup failed at `{stage}`: {reason}")]
    Setup {
        stage: &'static str,
        reason: String,
        raw_os_error: Option<i32>,
    },
    #[error("linux sandbox I/O error: {reason}")]
    Io {
        reason: String,
        raw_os_error: Option<i32>,
    },
    #[error("linux sandbox timed out and cleanup failed: {reason}")]
    TimeoutCleanup { reason: String },
}

impl SandboxLinuxError {
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn setup(stage: SandboxStage, reason: impl Into<String>) -> Self {
        Self::Setup {
            stage: stage.as_str(),
            reason: reason.into(),
            raw_os_error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_records_no_raw_os_error() {
        assert!(matches!(
            SandboxLinuxError::setup(SandboxStage::Clone, "x"),
            SandboxLinuxError::Setup {
                raw_os_error: None,
                ..
            }
        ));
    }
}

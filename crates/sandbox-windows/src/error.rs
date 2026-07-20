use thiserror::Error;

/// Setup stage identifiers for fail-closed diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxStage {
    Probe,
    Profile,
    AclGrant,
    Job,
    Pipes,
    CreateProcess,
    TokenCheck,
    Resume,
    Cleanup,
}

impl SandboxStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Profile => "profile",
            Self::AclGrant => "acl_grant",
            Self::Job => "job",
            Self::Pipes => "pipes",
            Self::CreateProcess => "create_process",
            Self::TokenCheck => "token_check",
            Self::Resume => "resume",
            Self::Cleanup => "cleanup",
        }
    }
}

/// Errors from the Windows sandbox backend.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SandboxWindowsError {
    #[error("windows sandbox unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("windows sandbox policy denied: {reason}")]
    PolicyDenied { reason: String },
    #[error("windows sandbox setup failed at `{stage}`: {reason}")]
    Setup { stage: &'static str, reason: String },
    #[error("windows sandbox I/O error: {reason}")]
    Io { reason: String },
    #[error("windows sandbox timed out and cleanup failed: {reason}")]
    TimeoutCleanup { reason: String },
}

impl SandboxWindowsError {
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
        }
    }
}

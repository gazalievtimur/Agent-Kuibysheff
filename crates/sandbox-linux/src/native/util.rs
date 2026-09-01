//! Shared helpers for errno and C strings.

use std::ffi::{CString, NulError, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::error::{SandboxLinuxError, SandboxStage};

pub(crate) fn errno_err(stage: SandboxStage, what: &str) -> SandboxLinuxError {
    let err = io::Error::last_os_error();
    SandboxLinuxError::Setup {
        stage: stage.as_str(),
        reason: format!("{what}: {err}"),
        raw_os_error: err.raw_os_error(),
    }
}

pub(crate) fn c_path(path: &Path) -> Result<CString, SandboxLinuxError> {
    c_os_str(path.as_os_str())
}

pub(crate) fn c_os_str(text: &OsStr) -> Result<CString, SandboxLinuxError> {
    CString::new(text.as_bytes()).map_err(|err: NulError| SandboxLinuxError::PolicyDenied {
        reason: format!("path contains NUL: {err}"),
    })
}

pub(crate) fn c_string_str(text: &str) -> Result<CString, SandboxLinuxError> {
    CString::new(text).map_err(|_| SandboxLinuxError::PolicyDenied {
        reason: "string contains NUL".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_err_preserves_raw_os_error() {
        let _ = std::fs::File::open("/no/such/kuibysheff-errno-probe-path");
        match errno_err(SandboxStage::Reap, "open") {
            SandboxLinuxError::Setup {
                raw_os_error,
                reason,
                ..
            } => {
                assert!(raw_os_error.is_some(), "{reason}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

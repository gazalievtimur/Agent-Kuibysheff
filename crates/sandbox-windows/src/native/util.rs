//! Shared Win32 helpers (wide strings, errors).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Diagnostics::Debug::{
    FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};

use crate::error::{SandboxStage, SandboxWindowsError};

pub(crate) fn to_wide_null(text: &str) -> Vec<u16> {
    os_to_wide_null(OsStr::new(text))
}

pub(crate) fn os_to_wide_null(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

pub(crate) fn path_to_wide_null(path: &Path) -> Vec<u16> {
    os_to_wide_null(path.as_os_str())
}

pub(crate) fn last_error_message(prefix: &str) -> String {
    // SAFETY: GetLastError is a process-local TLS read with no preconditions.
    let code = unsafe { GetLastError() };
    let mut buf = [0u16; 512];
    // SAFETY: FormatMessageW writes a system message for `code` into `buf`; IGNORE_INSERTS
    // means Arguments may be null.
    let n = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            ptr::null(),
            code,
            0,
            buf.as_mut_ptr(),
            buf.len() as u32,
            ptr::null_mut(),
        )
    };
    let detail = if n > 0 {
        String::from_utf16_lossy(&buf[..n as usize])
            .trim()
            .to_string()
    } else {
        String::new()
    };
    if detail.is_empty() {
        format!("{prefix} (win32={code})")
    } else {
        format!("{prefix} (win32={code}: {detail})")
    }
}

pub(crate) fn setup_last(stage: SandboxStage, prefix: &str) -> SandboxWindowsError {
    SandboxWindowsError::setup(stage, last_error_message(prefix))
}

/// Quote a Windows process argument per CommandLineToArgvW rules.
pub(crate) fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !needs_quotes {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(backslashes));
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(backslashes * 2));
    out.push('"');
    out
}

pub(crate) fn build_command_line(
    executable: &Path,
    argv: &[String],
) -> Result<Vec<u16>, SandboxWindowsError> {
    let exe = executable
        .to_str()
        .ok_or_else(|| SandboxWindowsError::PolicyDenied {
            reason: "executable path is not valid UTF-8".to_string(),
        })?;
    let mut line = quote_arg(exe);
    for arg in argv {
        line.push(' ');
        line.push_str(&quote_arg(arg));
    }
    Ok(to_wide_null(&line))
}

pub(crate) fn build_environment_block(
    env: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<u16>, SandboxWindowsError> {
    if env.is_empty() {
        // Empty block must be two consecutive NULs.
        return Ok(vec![0, 0]);
    }
    let mut block = Vec::new();
    for (key, value) in env {
        if key.contains('=') || key.contains('\0') || value.contains('\0') {
            return Err(SandboxWindowsError::PolicyDenied {
                reason: format!("invalid environment entry `{key}`"),
            });
        }
        let entry = format!("{key}={value}");
        block.extend(OsStr::new(&entry).encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_arg_handles_spaces_and_quotes() {
        assert_eq!(quote_arg("abc"), "abc");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn last_error_message_includes_win32_code() {
        let msg = last_error_message("CreateProcessW");
        assert!(msg.contains("win32="), "expected win32 code in `{msg}`");
    }
}

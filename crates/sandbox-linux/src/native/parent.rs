//! Parent-side supervisor: re-exec helper and collect output.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::protocol::{HelperRequest, HELPER_ENV, REQUEST_ENV};
use crate::request::{SandboxLaunchRequest, SandboxLaunchResult};

/// Spawns a fresh single-threaded helper via re-exec of the current binary.
pub fn run_via_helper(
    request: &SandboxLaunchRequest,
) -> Result<SandboxLaunchResult, SandboxLinuxError> {
    validate(request)?;

    let exe = std::env::current_exe().map_err(|err| {
        SandboxLinuxError::unavailable(format!("current_exe for helper re-exec: {err}"))
    })?;

    let mut req_file = tempfile::Builder::new()
        .prefix("agent-kuibyshev-sandbox-")
        .suffix(".json")
        .tempfile()
        .map_err(|err| {
            SandboxLinuxError::setup(SandboxStage::Helper, format!("temp request file: {err}"))
        })?;
    let helper_req = HelperRequest::from_launch(request);
    serde_json::to_writer_pretty(&mut req_file, &helper_req).map_err(|err| {
        SandboxLinuxError::setup(SandboxStage::Helper, format!("serialize request: {err}"))
    })?;
    req_file.flush().map_err(|err| {
        SandboxLinuxError::setup(SandboxStage::Helper, format!("flush request: {err}"))
    })?;
    let req_path = req_file.path().to_path_buf();

    let mut child = Command::new(&exe)
        .env(HELPER_ENV, "1")
        .env(REQUEST_ENV, &req_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| SandboxLinuxError::unavailable(format!("spawn sandbox helper: {err}")))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| SandboxLinuxError::setup(SandboxStage::Helper, "helper stdout missing"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| SandboxLinuxError::setup(SandboxStage::Helper, "helper stderr missing"))?;

    let max = request.max_output_chars;
    let stdout_thread = std::thread::spawn(move || read_bounded(&mut stdout, max));
    let stderr_thread = std::thread::spawn(move || read_bounded(&mut stderr, max));

    // Helper enforces the payload deadline; add a small parent grace period.
    let parent_wait = request.deadline + Duration::from_secs(5);
    let status = wait_with_timeout(&mut child, parent_wait)?;

    let (stdout_text, stdout_truncated) = stdout_thread
        .join()
        .unwrap_or_else(|_| (String::new(), true));
    let (stderr_text, stderr_truncated) = stderr_thread
        .join()
        .unwrap_or_else(|_| (String::new(), true));

    let timed_out = status.code() == Some(124);
    Ok(SandboxLaunchResult {
        stdout: stdout_text,
        stderr: stderr_text,
        stdout_truncated,
        stderr_truncated,
        exit_code: status.code(),
        timed_out,
    })
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    deadline: Duration,
) -> Result<std::process::ExitStatus, SandboxLinuxError> {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if started.elapsed() >= deadline {
                    let _ = child.kill();
                    let status = child
                        .wait()
                        .map_err(|err| SandboxLinuxError::TimeoutCleanup {
                            reason: format!("helper kill/wait failed: {err}"),
                        })?;
                    return Ok(status);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => {
                return Err(SandboxLinuxError::setup(
                    SandboxStage::Helper,
                    format!("helper wait: {err}"),
                ));
            }
        }
    }
}

fn read_bounded(reader: &mut impl Read, max_chars: usize) -> (String, bool) {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if truncated {
                    // Keep draining so the payload cannot hang on a full pipe.
                    continue;
                }
                bytes.extend_from_slice(&buf[..n]);
                let lossy = String::from_utf8_lossy(&bytes);
                if lossy.chars().count() > max_chars {
                    bytes = lossy
                        .chars()
                        .take(max_chars)
                        .collect::<String>()
                        .into_bytes();
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    let lossy = String::from_utf8_lossy(&bytes);
    if lossy.chars().count() > max_chars {
        (lossy.chars().take(max_chars).collect(), true)
    } else {
        (lossy.into_owned(), truncated)
    }
}

fn validate(request: &SandboxLaunchRequest) -> Result<(), SandboxLinuxError> {
    if !request.executable.is_absolute() {
        return Err(SandboxLinuxError::PolicyDenied {
            reason: "executable must be absolute".to_string(),
        });
    }
    if !request.cwd.is_absolute() {
        return Err(SandboxLinuxError::PolicyDenied {
            reason: "cwd must be absolute".to_string(),
        });
    }
    if request.max_output_chars == 0 {
        return Err(SandboxLinuxError::PolicyDenied {
            reason: "max_output_chars must be non-zero".to_string(),
        });
    }
    if !request.executable.exists() {
        return Err(SandboxLinuxError::PolicyDenied {
            reason: format!("executable not found: {}", request.executable.display()),
        });
    }
    Ok(())
}

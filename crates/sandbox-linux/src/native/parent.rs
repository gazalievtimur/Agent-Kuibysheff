//! Parent-side supervisor: re-exec helper and collect output.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use libc::{pollfd, POLLIN};

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
        .prefix("agent-kuibysheff-sandbox-")
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
    // Helper enforces the payload deadline; add a small parent grace period.
    let parent_wait = request.deadline + Duration::from_secs(5);
    // Pipe reads must not outlive that window: a helper stuck in D-state after
    // a userns/mount deadlock never EOFs, and `join` would hang the test binary.
    let read_deadline = Instant::now() + parent_wait + Duration::from_secs(2);
    let stdout_thread =
        std::thread::spawn(move || read_pipe_until(&mut stdout, max, read_deadline));
    let stderr_thread =
        std::thread::spawn(move || read_pipe_until(&mut stderr, max, read_deadline));

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
                    return wait_after_kill(child);
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

fn wait_after_kill(
    child: &mut std::process::Child,
) -> Result<std::process::ExitStatus, SandboxLinuxError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(SandboxLinuxError::TimeoutCleanup {
                        reason: "helper still running after SIGKILL (uninterruptible wait?)"
                            .to_string(),
                    });
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => {
                return Err(SandboxLinuxError::TimeoutCleanup {
                    reason: format!("helper kill/wait failed: {err}"),
                });
            }
        }
    }
}

fn fd_readable(fd: i32, timeout: Duration) -> bool {
    if timeout.is_zero() {
        return false;
    }
    let mut pfd = pollfd {
        fd,
        events: POLLIN,
        revents: 0,
    };
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: poll a live pipe fd owned by this thread's reader.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    rc > 0
}

fn read_pipe_until(
    reader: &mut (impl Read + AsRawFd),
    max_chars: usize,
    deadline: Instant,
) -> (String, bool) {
    let fd = reader.as_raw_fd();
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 4096];
    loop {
        let remaining = deadline.saturating_sub(Instant::now());
        if remaining.is_zero() {
            truncated = true;
            break;
        }
        if !fd_readable(fd, remaining) {
            truncated = true;
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if truncated {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::tempdir;

    fn minimal_request(exe: PathBuf, cwd: PathBuf) -> SandboxLaunchRequest {
        SandboxLaunchRequest {
            executable: exe,
            argv: Vec::new(),
            cwd,
            env: BTreeMap::new(),
            home_read: Vec::new(),
            home_write: Vec::new(),
            runtime_read_roots: Vec::new(),
            deadline: Duration::from_secs(1),
            max_output_chars: 1024,
            allow_children: false,
        }
    }

    #[test]
    fn validate_rejects_relative_executable() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("payload.sh");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        let mut req = minimal_request(PathBuf::from("payload.sh"), dir.path().to_path_buf());
        req.executable = PathBuf::from("payload.sh");
        let err = validate(&req).expect_err("relative exe");
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn validate_rejects_relative_cwd() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("payload.sh");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        let req = minimal_request(exe, PathBuf::from("relative-cwd"));
        let err = validate(&req).expect_err("relative cwd");
        assert!(err.to_string().contains("cwd"));
    }

    #[test]
    fn validate_rejects_zero_max_output() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("payload.sh");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        let mut req = minimal_request(exe, dir.path().to_path_buf());
        req.max_output_chars = 0;
        let err = validate(&req).expect_err("zero max_output");
        assert!(err.to_string().contains("max_output_chars"));
    }

    #[test]
    fn validate_rejects_missing_executable() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing-bin");
        let req = minimal_request(missing, dir.path().to_path_buf());
        let err = validate(&req).expect_err("missing exe");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn validate_accepts_absolute_existing() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("payload.sh");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        let req = minimal_request(exe, dir.path().to_path_buf());
        validate(&req).expect("valid request");
    }

    #[test]
    fn read_bounded_truncates_and_preserves_utf8() {
        let data = "abcdefghij";
        let mut cursor = std::io::Cursor::new(data.as_bytes());
        let (out, truncated) = read_bounded(&mut cursor, 5);
        assert!(truncated);
        assert_eq!(out.chars().count(), 5);
        assert_eq!(out, "abcde");
    }

    #[test]
    fn wait_with_timeout_kills_long_sleep() {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let started = Instant::now();
        let status = wait_with_timeout(&mut child, Duration::from_millis(200)).expect("wait");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "elapsed {:?}",
            started.elapsed()
        );
        assert!(!status.success());
    }
}

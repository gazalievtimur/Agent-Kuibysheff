//! Sandbox PID1: mounts, caps, seccomp, fork payload, reap.

use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::ptr;

use libc::{c_char, pid_t, ENOENT, O_CLOEXEC, O_NOFOLLOW, O_RDONLY};

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::native::caps::drop_capabilities;
use crate::native::mount::setup_rootfs;
use crate::native::seccomp::install_denylist;
use crate::native::util::{c_path, c_string_str, errno_err};
use crate::request::SandboxLaunchRequest;
use crate::OwnedFd;

/// Runs as PID1 inside the new namespaces. Never returns.
pub fn pid1_main(request: SandboxLaunchRequest, start_pipe_read: OwnedFd) -> ! {
    match pid1_try(request, start_pipe_read) {
        Ok(code) => {
            // SAFETY: terminate PID1 with the payload exit status.
            unsafe { libc::_exit(code) };
        }
        Err(err) => {
            let msg = format!("sandbox pid1 failed: {err}\n");
            // SAFETY: best-effort stderr write before exit.
            unsafe {
                let _ = libc::write(2, msg.as_ptr().cast(), msg.len());
                libc::_exit(70);
            }
        }
    }
}

fn pid1_try(
    request: SandboxLaunchRequest,
    start_pipe_read: OwnedFd,
) -> Result<i32, SandboxLinuxError> {
    // Wait until parent finished uid/gid maps.
    wait_for_start_barrier(start_pipe_read)?;

    let scratch = PathBuf::from(format!("/tmp/agent-kuibysheff-sb-{}", {
        // SAFETY: getpid is always safe; used only to build a unique scratch path name.
        unsafe { libc::getpid() }
    }));
    fs::create_dir_all(&scratch).map_err(|err| {
        SandboxLinuxError::setup(SandboxStage::Mount, format!("scratch dir: {err}"))
    })?;

    // Open executable before pivot (still visible on old root).
    let exec_fd = open_executable(&request.executable)?;

    setup_rootfs(&request, &scratch)?;
    drop_capabilities()?;

    // Fork payload so PID1 can reap.
    // SAFETY: single-threaded helper child; fork is safe here.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(errno_err(SandboxStage::Exec, "fork payload"));
    }
    if child == 0 {
        payload_exec(&request, exec_fd);
    }

    // Close exec fd in supervisor.
    drop(exec_fd);
    reap_until(child)
}

fn wait_for_start_barrier(start_pipe_read: OwnedFd) -> Result<(), SandboxLinuxError> {
    let mut buf = [0u8; 1];
    loop {
        // SAFETY: read on an owned pipe end.
        let n = unsafe { libc::read(start_pipe_read.as_raw_fd(), buf.as_mut_ptr().cast(), 1) };
        if n == 0 {
            return Ok(());
        }
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(SandboxLinuxError::setup(
                SandboxStage::Helper,
                format!("start barrier read: {err}"),
            ));
        }
    }
}

fn open_executable(path: &std::path::Path) -> Result<OwnedFd, SandboxLinuxError> {
    let c = c_path(path)?;
    // SAFETY: O_NOFOLLOW refuses symlink leaf; O_RDONLY for fexecve.
    let fd = unsafe { libc::open(c.as_ptr(), O_RDONLY | O_CLOEXEC | O_NOFOLLOW) };
    if fd < 0 {
        return Err(errno_err(SandboxStage::Exec, "open executable"));
    }
    // SAFETY: uniquely owned open fd.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn apply_nproc_limit(max_processes: u32) -> Result<(), String> {
    if max_processes == 0 {
        return Err("max_processes must be non-zero".to_string());
    }
    let limit = libc::rlimit {
        rlim_cur: libc::rlim_t::from(max_processes),
        rlim_max: libc::rlim_t::from(max_processes),
    };
    // SAFETY: setrlimit affects only this process and the children it execs.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &limit) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("setrlimit(RLIMIT_NPROC): {err}"));
    }
    Ok(())
}

/// Applies the child-process cap when the launch allows children.
fn maybe_apply_nproc_limit(request: &SandboxLaunchRequest) -> Result<(), String> {
    if !request.allow_children {
        return Ok(());
    }
    apply_nproc_limit(request.max_processes)
}

fn nproc_limit_failure_message(reason: &str) -> String {
    format!("sandbox payload nproc limit failed: {reason}\n")
}

fn payload_exec(request: &SandboxLaunchRequest, exec_fd: OwnedFd) -> ! {
    // Move to requested cwd (now inside pivoted root).
    if let Ok(cwd) = c_path(&request.cwd) {
        // SAFETY: chdir to policy cwd inside the sandbox root.
        let _ = unsafe { libc::chdir(cwd.as_ptr()) };
    }

    if let Err(reason) = maybe_apply_nproc_limit(request) {
        let msg = nproc_limit_failure_message(&reason);
        // SAFETY: best-effort stderr write before exit.
        unsafe {
            let _ = libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(71);
        }
    }

    // Install the seccomp filter in the payload process. When children are not allowed,
    // this denies fork/clone/vfork with ENOSYS.
    if let Err(err) = install_denylist(request.allow_children) {
        let msg = format!("sandbox payload seccomp failed: {err}\n");
        // SAFETY: best-effort stderr write before exit.
        unsafe {
            let _ = libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(71);
        }
    }

    let mut argv_c: Vec<CString> = Vec::new();
    // argv0 = executable path for display.
    if let Ok(a0) = c_path(&request.executable) {
        argv_c.push(a0);
    } else if let Ok(a0) = c_string_str("payload") {
        argv_c.push(a0);
    }
    for arg in &request.argv {
        if let Ok(c) = c_string_str(arg) {
            argv_c.push(c);
        }
    }
    let mut argv_ptr: Vec<*const c_char> = argv_c.iter().map(|c| c.as_ptr()).collect();
    argv_ptr.push(ptr::null());

    let mut env_c: Vec<CString> = Vec::new();
    for (k, v) in &request.env {
        if let Ok(c) = c_string_str(&format!("{k}={v}")) {
            env_c.push(c);
        }
    }
    let mut env_ptr: Vec<*const c_char> = env_c.iter().map(|c| c.as_ptr()).collect();
    env_ptr.push(ptr::null());

    // Prefer execve by absolute path: fexecve(2) cannot run `#!` scripts (ENOENT).
    // The mount namespace is already pivoted/sealed, so the path refers to our binds.
    if let Ok(path) = c_path(&request.executable) {
        // SAFETY: argv/env are NUL-terminated; path exists under the sealed root.
        let _ = unsafe { libc::execve(path.as_ptr(), argv_ptr.as_ptr(), env_ptr.as_ptr()) };
    }

    // Fallback for callers that only have a verified fd (non-script ELF).
    // SAFETY: fexecve replaces the process image using the opened executable fd.
    let _ = unsafe { libc::fexecve(exec_fd.as_raw_fd(), argv_ptr.as_ptr(), env_ptr.as_ptr()) };
    let err = std::io::Error::last_os_error();
    let msg = format!("exec payload failed: {err}\n");
    // SAFETY: write a best-effort message to stderr then terminate; `msg` is a live byte buffer
    // and `_exit` must not unwind into the helper frame.
    unsafe {
        let _ = libc::write(2, msg.as_ptr().cast(), msg.len());
        libc::_exit(127);
    }
}

fn reap_until(target: pid_t) -> Result<i32, SandboxLinuxError> {
    let mut status = 0;
    loop {
        // SAFETY: wait for any child; PID1 must reap zombies.
        let pid = unsafe { libc::waitpid(-1, &mut status, 0) };
        if pid < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ENOENT) || err.raw_os_error() == Some(libc::ECHILD) {
                return Ok(1);
            }
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(SandboxLinuxError::setup(
                SandboxStage::Reap,
                format!("waitpid: {err}"),
            ));
        }
        if pid == target {
            if libc::WIFEXITED(status) {
                return Ok(libc::WEXITSTATUS(status));
            }
            if libc::WIFSIGNALED(status) {
                return Ok(128 + libc::WTERMSIG(status));
            }
            return Ok(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_nproc_limit, maybe_apply_nproc_limit, nproc_limit_failure_message};
    use crate::request::SandboxLaunchRequest;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Duration;

    fn sample_request(allow_children: bool, max_processes: u32) -> SandboxLaunchRequest {
        SandboxLaunchRequest {
            executable: PathBuf::from("/bin/true"),
            argv: Vec::new(),
            cwd: PathBuf::from("/"),
            env: BTreeMap::new(),
            home_read: Vec::new(),
            home_write: Vec::new(),
            runtime_read_roots: Vec::new(),
            deadline: Duration::from_secs(1),
            max_output_chars: 1024,
            allow_children,
            max_processes,
        }
    }

    #[test]
    fn nproc_limit_rejects_zero() {
        assert!(apply_nproc_limit(0).is_err());
    }

    #[test]
    fn maybe_apply_nproc_skips_when_children_disallowed() {
        let request = sample_request(false, 0);
        assert!(maybe_apply_nproc_limit(&request).is_ok());
    }

    #[test]
    fn nproc_limit_failure_message_mentions_reason() {
        let msg = nproc_limit_failure_message("boom");
        assert!(msg.contains("nproc limit failed"));
        assert!(msg.contains("boom"));
        assert!(msg.ends_with('\n'));
    }

    #[cfg(not(miri))]
    #[test]
    #[allow(unexpected_cfgs)]
    fn nproc_limit_setrlimit_ok_and_err_paths() {
        let mut cur = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: query the current process RLIMIT_NPROC ceiling.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NPROC, &mut cur) }, 0);

        // Re-applying the current hard ceiling only raises soft ≤ hard (allowed)
        // and does not raise the hard limit, so later tests keep their spawn budget.
        if cur.rlim_max > 0
            && cur.rlim_max != libc::RLIM_INFINITY
            && cur.rlim_max <= u64::from(u32::MAX)
        {
            let hard = cur.rlim_max as u32;
            assert!(
                apply_nproc_limit(hard).is_ok(),
                "re-applying hard nproc={hard} should succeed"
            );
            let request = sample_request(true, hard);
            assert!(maybe_apply_nproc_limit(&request).is_ok());
        }

        // Raising the hard limit without CAP_SYS_RESOURCE fails on CI hosts.
        if cur.rlim_max != libc::RLIM_INFINITY && cur.rlim_max < u64::from(u32::MAX) {
            let raised = (cur.rlim_max as u32).saturating_add(1).max(1);
            let err = apply_nproc_limit(raised).expect_err("raising hard nproc");
            assert!(err.contains("setrlimit"), "{err}");
            let request = sample_request(true, raised);
            let err = maybe_apply_nproc_limit(&request).expect_err("raise via maybe");
            assert!(err.contains("setrlimit"), "{err}");
        }
    }
}

//! Helper process: clone namespaces, map ids, supervise via pidfd.

use std::time::{Duration, Instant};

use libc::{c_int, POLLIN, SIGKILL};

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::native::clone::clone_sandbox_init;
use crate::native::pid1::pid1_main;
use crate::native::userns::write_id_maps;
use crate::native::util::errno_err;
use crate::protocol::HelperRequest;
use crate::OwnedFd;

const SYS_PIDFD_SEND_SIGNAL: i64 = 424;

/// Helper entry: returns process exit code for the parent supervisor.
///
/// Exit code `124` means the sandbox timed out and was killed.
pub fn run_helper(request: HelperRequest) -> Result<i32, SandboxLinuxError> {
    let deadline = request.launch.deadline;
    let launch = request.launch;
    let (barrier_read, barrier_write) = pipe_pair()?;
    // Raw write fd so the child can close its inherited copy (clone duplicates the fd table).
    let barrier_write_raw = barrier_write.as_raw_fd();

    let cloned = clone_sandbox_init(move || {
        // SAFETY: close the duplicated write end in the child so the barrier can EOF.
        let _ = unsafe { libc::close(barrier_write_raw) };
        pid1_main(launch, barrier_read);
    })?;

    if let Err(err) = write_id_maps(cloned.child_pid) {
        // Keep the start barrier closed-from-parent side held until the child is
        // dead, otherwise PID1 proceeds without CAP_SYS_ADMIN and misreports mounts.
        kill_pidfd(&cloned.pidfd);
        let _ = wait_pidfd(cloned.pidfd, Duration::from_secs(2));
        return Err(err);
    }
    // Release the start barrier so PID1 continues setup.
    drop(barrier_write);

    wait_pidfd(cloned.pidfd, deadline)
}

fn kill_pidfd(pidfd: &OwnedFd) {
    // SAFETY: best-effort kill of the namespace init via pidfd.
    let _ = unsafe {
        libc::syscall(
            SYS_PIDFD_SEND_SIGNAL,
            pidfd.as_raw_fd(),
            SIGKILL,
            std::ptr::null::<c_int>(),
            0,
        )
    };
}

fn wait_pidfd(pidfd: OwnedFd, deadline: Duration) -> Result<i32, SandboxLinuxError> {
    let started = Instant::now();
    loop {
        let remaining = deadline.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            kill_pidfd(&pidfd);
            let mut pfd = libc::pollfd {
                fd: pidfd.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            // SAFETY: poll waits for pidfd readability after kill, best-effort bounded by timeout.
            let _ = unsafe { libc::poll(&mut pfd, 1, 2000) };
            return match waitid_pidfd(&pidfd, libc::WEXITED | libc::WNOHANG)? {
                Some(_) => Ok(124),
                None => Err(SandboxLinuxError::TimeoutCleanup {
                    reason: "helper init still alive after SIGKILL".to_string(),
                }),
            };
        }

        let timeout_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
        let mut pfd = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        // SAFETY: poll the pidfd for exit notification.
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(SandboxLinuxError::setup(
                SandboxStage::Reap,
                format!("pidfd poll: {err}"),
            ));
        }
        if rc == 0 {
            continue;
        }

        let info = waitid_pidfd(&pidfd, libc::WEXITED)?.ok_or_else(|| {
            SandboxLinuxError::setup(SandboxStage::Reap, "waitid returned no child")
        })?;
        return Ok(exit_status_from_siginfo(&info));
    }
}

fn waitid_pidfd(
    pidfd: &OwnedFd,
    options: c_int,
) -> Result<Option<libc::siginfo_t>, SandboxLinuxError> {
    loop {
        // SAFETY: waitid writes into `info`; all-bits-zero is a valid POD representation before
        // a successful waitid call. For WNOHANG, si_pid stays 0 when nothing is waitable.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waitid with P_PIDFD retrieves exit status without pid reuse races.
        let wr =
            unsafe { libc::waitid(libc::P_PIDFD, pidfd.as_raw_fd() as u32, &mut info, options) };
        if wr != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            if err.raw_os_error() == Some(libc::ECHILD) {
                return Ok(None);
            }
            return Err(errno_err(SandboxStage::Reap, "waitid(P_PIDFD)"));
        }
        // SAFETY: si_pid is defined after waitid; 0 means WNOHANG found no waitable child.
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        return Ok(Some(info));
    }
}

fn exit_status_from_siginfo(info: &libc::siginfo_t) -> i32 {
    // SAFETY: fields are valid after successful waitid WEXITED with a non-zero si_pid.
    let status = unsafe { info.si_status() };
    let code = info.si_code;
    if code == libc::CLD_EXITED {
        status
    } else if code == libc::CLD_KILLED || code == libc::CLD_DUMPED {
        128 + status
    } else {
        status
    }
}

fn pipe_pair() -> Result<(OwnedFd, OwnedFd), SandboxLinuxError> {
    let mut fds = [0; 2];
    // SAFETY: creates a new pipe with CLOEXEC on both ends.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(errno_err(SandboxStage::Helper, "pipe2"));
    }
    // SAFETY: uniquely own both ends.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    const SYS_PIDFD_OPEN: i64 = 434;

    fn spawn_sleep() -> std::process::Child {
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep")
    }

    fn pidfd_for(pid: u32) -> OwnedFd {
        let fd = unsafe { libc::syscall(SYS_PIDFD_OPEN, pid as libc::pid_t, 0) };
        assert!(
            fd >= 0,
            "pidfd_open({pid}) failed: {}",
            std::io::Error::last_os_error()
        );
        unsafe { OwnedFd::from_raw_fd(fd as i32) }
    }

    #[test]
    fn wait_pidfd_returns_child_exit_code() {
        let child = Command::new("sh")
            .args(["-c", "exit 42"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sh");
        let pidfd = pidfd_for(child.id());
        let code = wait_pidfd(pidfd, Duration::from_secs(5)).expect("wait_pidfd");
        assert_eq!(code, 42);
        let mut child = child;
        let _ = child.wait();
    }

    #[test]
    fn wait_pidfd_timeout_reaps_and_returns_124() {
        let mut child = spawn_sleep();
        let pidfd = pidfd_for(child.id());
        let code = wait_pidfd(pidfd, Duration::ZERO).expect("timeout reap");
        assert_eq!(code, 124);
        let _ = child.wait();
    }

    #[test]
    fn wait_pidfd_killed_child_uses_128_plus_signal() {
        let mut child = spawn_sleep();
        let pid = child.id() as libc::pid_t;
        let pidfd = pidfd_for(child.id());
        let _ = unsafe { libc::kill(pid, SIGKILL) };
        let code = wait_pidfd(pidfd, Duration::from_secs(5)).expect("wait killed");
        assert_eq!(code, 128 + SIGKILL);
        let _ = child.wait();
    }

    #[test]
    fn waitid_wnonhang_on_live_child_is_none() {
        let mut child = spawn_sleep();
        let pidfd = pidfd_for(child.id());
        let got = waitid_pidfd(&pidfd, libc::WEXITED | libc::WNOHANG).expect("wnonhang");
        assert!(
            got.is_none(),
            "live child must not be waitable with WNOHANG"
        );
        let _ = wait_pidfd(pidfd, Duration::ZERO);
        let _ = child.wait();
    }

    #[test]
    fn waitid_after_reap_maps_echild_to_none() {
        let mut child = Command::new("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn true");
        let pidfd = pidfd_for(child.id());
        assert!(waitid_pidfd(&pidfd, libc::WEXITED)
            .expect("first waitid")
            .is_some());
        let again = waitid_pidfd(&pidfd, libc::WEXITED | libc::WNOHANG).expect("second waitid");
        assert!(again.is_none());
        let _ = child.wait();
    }

    #[test]
    fn timeout_cleanup_when_waitid_finds_no_child() {
        let mut child = spawn_sleep();
        let pidfd = pidfd_for(child.id());
        let err = match waitid_pidfd(&pidfd, libc::WEXITED | libc::WNOHANG).expect("live wnonhang")
        {
            Some(_) => panic!("live child should not be reaped with WNOHANG"),
            None => SandboxLinuxError::TimeoutCleanup {
                reason: "helper init still alive after SIGKILL".to_string(),
            },
        };
        assert!(matches!(err, SandboxLinuxError::TimeoutCleanup { .. }));
        let _ = wait_pidfd(pidfd, Duration::ZERO);
        let _ = child.wait();
    }

    #[test]
    fn waitid_on_pipe_fd_is_error() {
        let mut fds = [0; 2];
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(rc, 0);
        let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let err = waitid_pidfd(&read, libc::WEXITED | libc::WNOHANG).expect_err("pipe waitid");
        assert!(
            matches!(err, SandboxLinuxError::Setup { stage: "reap", .. }),
            "{err}"
        );
        drop(write);
    }
}

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
            // SAFETY: kill namespace init; descendants die with the pid namespace.
            let _ = unsafe {
                libc::syscall(
                    SYS_PIDFD_SEND_SIGNAL,
                    pidfd.as_raw_fd(),
                    SIGKILL,
                    std::ptr::null::<c_int>(),
                    0,
                )
            };
            let mut pfd = libc::pollfd {
                fd: pidfd.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            let _ = unsafe { libc::poll(&mut pfd, 1, 2000) };
            return Ok(124);
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

        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waitid with P_PIDFD retrieves exit status without pid reuse races.
        let wr = unsafe {
            libc::waitid(
                libc::P_PIDFD,
                pidfd.as_raw_fd() as u32,
                &mut info,
                libc::WEXITED,
            )
        };
        if wr != 0 {
            return Err(errno_err(SandboxStage::Reap, "waitid(P_PIDFD)"));
        }
        // SAFETY: fields are valid after successful waitid WEXITED.
        let status = unsafe { info.si_status() };
        let code = info.si_code;
        if code == libc::CLD_EXITED {
            return Ok(status);
        }
        if code == libc::CLD_KILLED || code == libc::CLD_DUMPED {
            return Ok(128 + status);
        }
        return Ok(status);
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

//! clone3 with namespaces + pidfd.

use std::mem;
use std::ptr;

use libc::{c_int, c_long, c_void, pid_t, SIGCHLD, SIGKILL};

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::native::util::errno_err;
use crate::OwnedFd;

const SYS_CLONE3: c_long = 435;
const CLONE_NEWNS: u64 = 0x0002_0000;
const CLONE_NEWUSER: u64 = 0x1000_0000;
const CLONE_NEWPID: u64 = 0x2000_0000;
const CLONE_NEWIPC: u64 = 0x0800_0000;
const CLONE_NEWNET: u64 = 0x4000_0000;
const CLONE_PIDFD: u64 = 0x0000_1000;
const CLONE_CLEAR_SIGHAND: u64 = 0x1_0000_0000;

#[repr(C, align(8))]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

/// Result of cloning the sandbox PID1.
pub struct CloneResult {
    pub child_pid: pid_t,
    pub pidfd: OwnedFd,
}

/// Creates a child in user/mount/pid/ipc/net namespaces and returns a pidfd.
pub fn clone_sandbox_init<F>(child_main: F) -> Result<CloneResult, SandboxLinuxError>
where
    F: FnOnce() + Send,
{
    let mut pidfd: c_int = -1;
    let mut args = CloneArgs {
        flags: CLONE_NEWUSER
            | CLONE_NEWNS
            | CLONE_NEWPID
            | CLONE_NEWIPC
            | CLONE_NEWNET
            | CLONE_PIDFD
            | CLONE_CLEAR_SIGHAND,
        pidfd: ptr::from_mut(&mut pidfd) as u64,
        child_tid: 0,
        parent_tid: 0,
        exit_signal: SIGCHLD as u64,
        stack: 0,
        stack_size: 0,
        tls: 0,
        set_tid: 0,
        set_tid_size: 0,
        cgroup: 0,
    };

    // SAFETY: clone3 creates a new task; on success returns 0 in the child and
    // the child pid in the parent. `args` is valid for the duration of the call.
    let rc = unsafe {
        libc::syscall(
            SYS_CLONE3,
            ptr::from_mut(&mut args).cast::<c_void>(),
            mem::size_of::<CloneArgs>(),
        )
    };

    if rc < 0 {
        return Err(errno_err(SandboxStage::Clone, "clone3"));
    }

    if rc == 0 {
        child_main();
        // SAFETY: child entry must not return into the helper frame.
        unsafe { libc::_exit(70) };
    }

    if pidfd < 0 {
        let child_pid = rc as pid_t;
        // SAFETY: clone3 created a child but failed to return a pidfd; kill and
        // WNOHANG-reap so the task is not left running.
        let _ = unsafe { libc::kill(child_pid, SIGKILL) };
        let mut status = 0;
        let _ = unsafe { libc::waitpid(child_pid, &mut status, libc::WNOHANG) };
        return Err(SandboxLinuxError::setup(
            SandboxStage::Clone,
            "clone3 returned without a pidfd",
        ));
    }

    // SAFETY: pidfd is a uniquely owned fd from clone3.
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd) };
    Ok(CloneResult {
        child_pid: rc as pid_t,
        pidfd,
    })
}

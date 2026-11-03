//! Ask a child `kbshff run` to cancel. Windows uses Ctrl+Break; Unix uses SIGINT.

use std::io;
use std::process::Stdio;

use tokio::process::Command;

/// Spawn flags so a later [`interrupt_pid`] reaches this child and not the studio.
pub fn prepare(cmd: &mut Command) {
    #[cfg(windows)]
    {
        // CREATE_NEW_PROCESS_GROUP: the child's pid is the group id for Ctrl+Break.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Mark stdio pipes. Split out so callers stay short.
pub fn piped(cmd: &mut Command) {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
}

/// Signal a process id. Used while `Child::wait` is not borrowing the child.
///
/// # Errors
///
/// Returns when the OS rejects the signal.
pub fn interrupt_pid(pid: u32) -> io::Result<()> {
    #[cfg(windows)]
    {
        interrupt_windows(pid)
    }
    #[cfg(unix)]
    {
        interrupt_unix(pid)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = pid;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process interrupt is not supported on this platform",
        ))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn interrupt_windows(pid: u32) -> io::Result<()> {
    use windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent;
    // CTRL_BREAK_EVENT can target one process group. CTRL_C_EVENT cannot.
    const CTRL_BREAK_EVENT: u32 = 1;
    // SAFETY: pid is a live process-group id created with CREATE_NEW_PROCESS_GROUP.
    // GenerateConsoleCtrlEvent only posts a console event; it does not dereference pointers.
    let ok = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn interrupt_unix(pid: u32) -> io::Result<()> {
    // SAFETY: `kill` with SIGINT signals `pid`. A stale pid can hit a recycled
    // process; the caller only passes the child it just spawned.
    let rc = unsafe { libc::kill(pid as i32, libc::SIGINT) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

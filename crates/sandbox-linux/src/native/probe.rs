//! Probe required kernel primitives without running a payload.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::error::SandboxLinuxError;

const MS_REC: u64 = 0x4000;
const MS_PRIVATE: u64 = 1 << 18;

/// Returns `Ok(())` when unprivileged user namespaces and mounts look usable.
pub fn probe_primitives() -> Result<(), SandboxLinuxError> {
    if !Path::new("/proc/self/ns/user").exists() {
        return Err(SandboxLinuxError::unavailable(
            "user namespaces are not available (/proc/self/ns/user missing)",
        ));
    }
    if !Path::new("/proc/self/ns/pid").exists() {
        return Err(SandboxLinuxError::unavailable(
            "pid namespaces are not available",
        ));
    }
    if !Path::new("/proc/self/ns/mnt").exists() {
        return Err(SandboxLinuxError::unavailable(
            "mount namespaces are not available",
        ));
    }
    if !Path::new("/proc/self/ns/net").exists() {
        return Err(SandboxLinuxError::unavailable(
            "network namespaces are not available",
        ));
    }

    if let Ok(contents) = fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone") {
        if contents.trim() == "0" {
            return Err(SandboxLinuxError::unavailable(
                "kernel.unprivileged_userns_clone=0",
            ));
        }
    }

    // Full probe: unshare + uid/gid map + MS_PRIVATE remount.
    // A bare unshare can succeed while AppArmor still denies capabilities in the
    // new user namespace (`kernel.apparmor_restrict_unprivileged_userns=1`).
    probe_userns_mount_capability()?;

    Ok(())
}

fn probe_userns_mount_capability() -> Result<(), SandboxLinuxError> {
    let mut fds = [0; 2];
    // SAFETY: pipe for start barrier between parent mapper and child.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(SandboxLinuxError::unavailable(format!(
            "probe pipe2 failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);

    // SAFETY: fork a short-lived probe child.
    let child = unsafe { libc::fork() };
    if child < 0 {
        // SAFETY: close both pipe ends on fork failure.
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(SandboxLinuxError::unavailable(format!(
            "fork for namespace probe failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    if child == 0 {
        // SAFETY: child closes write end and waits for maps.
        unsafe {
            libc::close(write_fd);
        }
        let unshare_rc =
            unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET | libc::CLONE_NEWNS) };
        if unshare_rc != 0 {
            unsafe { libc::_exit(1) };
        }
        // Block until parent closes write end (maps written) or errors out.
        let mut buf = [0u8; 1];
        loop {
            let n = unsafe { libc::read(read_fd, buf.as_mut_ptr().cast(), 1) };
            if n == 0 {
                break;
            }
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                unsafe { libc::_exit(2) };
            }
        }
        let root = c"/";
        // SAFETY: remount / private — requires CAP_SYS_ADMIN in the new userns.
        let mount_rc = unsafe {
            libc::mount(
                std::ptr::null(),
                root.as_ptr(),
                std::ptr::null(),
                MS_REC | MS_PRIVATE,
                std::ptr::null(),
            )
        };
        unsafe { libc::_exit(if mount_rc == 0 { 0 } else { 3 }) };
    }

    // SAFETY: parent keeps write end until maps succeed.
    unsafe {
        libc::close(read_fd);
    }

    let map_result = write_probe_id_maps(child);
    // Always close write end so the child cannot hang on read.
    // SAFETY: release barrier regardless of map outcome.
    unsafe {
        libc::close(write_fd);
    }

    let mut status = 0;
    // SAFETY: wait for probe child.
    let waited = unsafe { libc::waitpid(child, &mut status, 0) };
    if waited < 0 {
        return Err(SandboxLinuxError::unavailable(format!(
            "waitpid for namespace probe failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    if let Err(err) = map_result {
        return Err(err);
    }

    if !libc::WIFEXITED(status) {
        return Err(SandboxLinuxError::unavailable(
            "namespace probe child did not exit cleanly",
        ));
    }
    match libc::WEXITSTATUS(status) {
        0 => Ok(()),
        1 => Err(SandboxLinuxError::unavailable(
            "unshare(CLONE_NEWUSER|CLONE_NEWNET|CLONE_NEWNS) failed in probe child",
        )),
        3 => Err(SandboxLinuxError::unavailable(
            "mount(MS_PRIVATE|/) denied in user namespace; on Ubuntu set \
             kernel.apparmor_restrict_unprivileged_userns=0 or install an AppArmor \
             profile that allows userns for this binary",
        )),
        code => Err(SandboxLinuxError::unavailable(format!(
            "namespace mount probe failed with exit {code}"
        ))),
    }
}

fn write_probe_id_maps(child_pid: libc::pid_t) -> Result<(), SandboxLinuxError> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let proc = format!("/proc/{child_pid}");

    // Give the child a moment to reach unshare before maps are writable.
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    loop {
        match try_write_maps(&proc, uid, gid) {
            Ok(()) => return Ok(()),
            Err(err) if std::time::Instant::now() < deadline => {
                let _ = err;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(err) => {
                return Err(SandboxLinuxError::unavailable(format!(
                    "probe uid/gid map failed ({err}); on Ubuntu check \
                     kernel.apparmor_restrict_unprivileged_userns"
                )));
            }
        }
    }
}

fn try_write_maps(proc: &str, uid: u32, gid: u32) -> std::io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .open(format!("{proc}/setgroups"))?
        .write_all(b"deny\n")?;
    fs::OpenOptions::new()
        .write(true)
        .open(format!("{proc}/uid_map"))?
        .write_all(format!("0 {uid} 1\n").as_bytes())?;
    fs::OpenOptions::new()
        .write(true)
        .open(format!("{proc}/gid_map"))?
        .write_all(format!("0 {gid} 1\n").as_bytes())?;
    Ok(())
}

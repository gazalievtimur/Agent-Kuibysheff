//! UID/GID map setup for the new user namespace.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use libc::pid_t;

use crate::error::{SandboxLinuxError, SandboxStage};

/// Maps the caller's UID/GID to 0 inside the child's user namespace.
pub fn write_id_maps(child_pid: pid_t) -> Result<(), SandboxLinuxError> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let proc = PathBuf::from(format!("/proc/{child_pid}"));

    write_file(
        &proc.join("setgroups"),
        b"deny\n",
        SandboxStage::UserMap,
        "setgroups",
    )?;
    write_file(
        &proc.join("uid_map"),
        format!("0 {uid} 1\n").as_bytes(),
        SandboxStage::UserMap,
        "uid_map",
    )?;
    write_file(
        &proc.join("gid_map"),
        format!("0 {gid} 1\n").as_bytes(),
        SandboxStage::UserMap,
        "gid_map",
    )?;
    Ok(())
}

fn write_file(
    path: &std::path::Path,
    bytes: &[u8],
    stage: SandboxStage,
    label: &str,
) -> Result<(), SandboxLinuxError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|err| {
            SandboxLinuxError::setup(stage, format!("open {label} ({}): {err}", path.display()))
        })?;
    file.write_all(bytes).map_err(|err| {
        SandboxLinuxError::setup(stage, format!("write {label} ({}): {err}", path.display()))
    })?;
    Ok(())
}

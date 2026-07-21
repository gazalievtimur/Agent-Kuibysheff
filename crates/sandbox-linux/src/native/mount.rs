//! Mount namespace setup and pivot_root.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use libc::{MS_BIND, MS_PRIVATE, MS_RDONLY, MS_REC, MS_REMOUNT};

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::native::util::{c_path, c_string_str, errno_err};
use crate::request::SandboxLaunchRequest;

const MS_NOSUID: u64 = 2;
const MS_NODEV: u64 = 4;
const MS_NOEXEC: u64 = 8;
const SYS_MOUNT_SETATTR: i64 = 442;
const MOUNT_ATTR_RDONLY: u64 = 0x0000_0001;
const AT_EMPTY_PATH: i32 = 0x1000;
const AT_RECURSIVE: i32 = 0x8000;

#[repr(C)]
struct MountAttr {
    attr_set: u64,
    attr_clr: u64,
    propagation: u64,
    userns_fd: u64,
}

/// Builds an isolated rootfs and pivots into it.
pub fn setup_rootfs(
    request: &SandboxLaunchRequest,
    scratch: &Path,
) -> Result<(), SandboxLinuxError> {
    // Make mount propagation private.
    mount_raw(
        None,
        Path::new("/"),
        None,
        MS_REC | MS_PRIVATE,
        None,
        SandboxStage::Mount,
        "make / private",
    )?;

    let new_root = scratch.join("new_root");
    let old_root = new_root.join("old_root");
    fs::create_dir_all(&new_root).map_err(|err| {
        SandboxLinuxError::setup(SandboxStage::Mount, format!("mkdir new_root: {err}"))
    })?;

    mount_raw(
        Some(Path::new("tmpfs")),
        &new_root,
        Some("tmpfs"),
        MS_NOSUID | MS_NODEV,
        Some("mode=755,size=64m"),
        SandboxStage::Mount,
        "tmpfs new_root",
    )?;

    fs::create_dir_all(&old_root).map_err(|err| {
        SandboxLinuxError::setup(SandboxStage::Mount, format!("mkdir old_root: {err}"))
    })?;
    for dir in ["proc", "dev", "tmp", "run", "etc"] {
        fs::create_dir_all(new_root.join(dir)).map_err(|err| {
            SandboxLinuxError::setup(SandboxStage::Mount, format!("mkdir {dir}: {err}"))
        })?;
    }

    // Private /tmp and /run
    mount_raw(
        Some(Path::new("tmpfs")),
        &new_root.join("tmp"),
        Some("tmpfs"),
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
        Some("mode=1777,size=32m"),
        SandboxStage::Mount,
        "tmpfs /tmp",
    )?;
    mount_raw(
        Some(Path::new("tmpfs")),
        &new_root.join("run"),
        Some("tmpfs"),
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
        Some("mode=755,size=8m"),
        SandboxStage::Mount,
        "tmpfs /run",
    )?;

    // Minimal /dev
    for node in ["null", "zero", "urandom", "random", "tty"] {
        let src = PathBuf::from("/dev").join(node);
        let dst = new_root.join("dev").join(node);
        touch_file(&dst)?;
        if src.exists() {
            mount_raw(
                Some(&src),
                &dst,
                None,
                MS_BIND,
                None,
                SandboxStage::Mount,
                &format!("bind /dev/{node}"),
            )?;
        }
    }

    // Writable grants first; read-only binds skip anything already writable so we
    // never remount a write grant MS_RDONLY (EPERM on some tmpfs layouts).
    let mut writable = BTreeSet::new();
    for path in &request.home_write {
        let logical = absolute_logical(path)?;
        bind_into_root(&new_root, &logical, false)?;
        writable.insert(logical);
    }
    let cwd = absolute_logical(&request.cwd)?;
    if !writable.contains(&cwd) {
        bind_into_root(&new_root, &cwd, false)?;
        writable.insert(cwd);
    }

    let mut readonly = BTreeSet::new();
    for path in request
        .home_read
        .iter()
        .chain(request.runtime_read_roots.iter())
    {
        readonly.insert(absolute_logical(path)?);
    }
    readonly.insert(absolute_logical(&request.executable)?);
    if let Some(parent) = request.executable.parent() {
        readonly.insert(absolute_logical(parent)?);
    }
    for essential in [
        "/bin",
        "/lib",
        "/lib64",
        "/usr/bin",
        "/usr/lib",
        "/usr/lib64",
    ] {
        let p = Path::new(essential);
        if p.exists() {
            readonly.insert(absolute_logical(p)?);
        }
    }
    for path in readonly {
        if writable.iter().any(|w| path == *w || path.starts_with(w)) {
            continue;
        }
        bind_into_root(&new_root, &path, true)?;
    }

    // Dynamic linker cache (file bind; remount RO separately).
    let ld_cache = Path::new("/etc/ld.so.cache");
    if ld_cache.exists() {
        let dst = new_root.join("etc/ld.so.cache");
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).ok();
        }
        touch_file(&dst)?;
        mount_raw(
            Some(ld_cache),
            &dst,
            None,
            MS_BIND,
            None,
            SandboxStage::Mount,
            "bind ld.so.cache",
        )?;
        remount_ro(&dst)?;
    }

    // Mount proc before pivot while still PID1 in the new pid namespace.
    // Some kernels reject proc mounts after pivot_root/umount of the old root.
    // SAFETY: getpid is always safe; used only to build a unique scratch path name.
    let pid = unsafe { libc::getpid() };
    if pid != 1 {
        return Err(SandboxLinuxError::setup(
            SandboxStage::Mount,
            format!("expected pid 1 inside pid namespace before proc mount, got {pid}"),
        ));
    }
    mount_raw(
        Some(Path::new("proc")),
        &new_root.join("proc"),
        Some("proc"),
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
        None,
        SandboxStage::Mount,
        "proc",
    )?;

    // pivot_root
    let new_c = c_path(&new_root)?;
    let old_c = c_path(&old_root)?;
    // SAFETY: both paths exist and are directories on the tmpfs mount.
    let rc = unsafe { libc::syscall(libc::SYS_pivot_root, new_c.as_ptr(), old_c.as_ptr()) };
    if rc != 0 {
        return Err(errno_err(SandboxStage::PivotRoot, "pivot_root"));
    }

    let root = c_string_str("/")?;
    // SAFETY: chdir to new root after pivot.
    if unsafe { libc::chdir(root.as_ptr()) } != 0 {
        return Err(errno_err(SandboxStage::PivotRoot, "chdir /"));
    }

    // Detach old root.
    let old = c_string_str("/old_root")?;
    // SAFETY: lazy unmount of the old host root.
    if unsafe { libc::umount2(old.as_ptr(), libc::MNT_DETACH) } != 0 {
        return Err(errno_err(SandboxStage::PivotRoot, "umount2 old_root"));
    }
    let _ = fs::remove_dir("/old_root");

    Ok(())
}

fn bind_into_root(
    new_root: &Path,
    host_path: &Path,
    read_only: bool,
) -> Result<(), SandboxLinuxError> {
    // Keep the guest path identical to the request (e.g. `/bin`), even when the
    // host path is a symlink to `/usr/bin` — shebangs need `/bin/sh`.
    let logical = absolute_logical(host_path)?;
    let source = fs::canonicalize(host_path).map_err(|err| {
        SandboxLinuxError::setup(
            SandboxStage::Mount,
            format!("canonicalize {}: {err}", host_path.display()),
        )
    })?;

    let rel = strip_root(&logical);
    let dst = new_root.join(rel);
    if source.is_dir() {
        fs::create_dir_all(&dst).map_err(|err| {
            SandboxLinuxError::setup(
                SandboxStage::Mount,
                format!("mkdir {}: {err}", dst.display()),
            )
        })?;
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                SandboxLinuxError::setup(
                    SandboxStage::Mount,
                    format!("mkdir {}: {err}", parent.display()),
                )
            })?;
        }
        touch_file(&dst)?;
    }

    let flags = MS_BIND | MS_REC;
    mount_raw(
        Some(&source),
        &dst,
        None,
        flags,
        None,
        SandboxStage::Mount,
        &format!("bind {} -> {}", source.display(), logical.display()),
    )?;
    if read_only {
        remount_ro(&dst)?;
    }
    Ok(())
}

fn remount_ro(path: &Path) -> Result<(), SandboxLinuxError> {
    if mount_setattr_rdonly(path).is_ok() {
        return Ok(());
    }
    // Prefer non-recursive remount: MS_REC can EPERM when a locked parent
    // mount is pulled into the recursive walk (common for binds under /tmp).
    let target_c = c_path(path)?;
    // SAFETY: remount the bind we just created as read-only.
    let rc = unsafe {
        libc::mount(
            target_c.as_ptr(),
            target_c.as_ptr(),
            std::ptr::null(),
            MS_BIND | MS_RDONLY | MS_REMOUNT,
            std::ptr::null(),
        )
    };
    if rc == 0 {
        return Ok(());
    }
    mount_raw(
        Some(path),
        path,
        None,
        MS_BIND | MS_REC | MS_RDONLY | MS_REMOUNT,
        None,
        SandboxStage::Mount,
        &format!("remount ro {}", path.display()),
    )
}

fn mount_setattr_rdonly(path: &Path) -> Result<(), SandboxLinuxError> {
    let c = c_path(path)?;
    // SAFETY: O_PATH opens the mount point without requiring read access.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(errno_err(
            SandboxStage::Mount,
            "open O_PATH for mount_setattr",
        ));
    }
    let attr = MountAttr {
        attr_set: MOUNT_ATTR_RDONLY,
        attr_clr: 0,
        propagation: 0,
        userns_fd: 0,
    };
    let empty = c_string_str("")?;
    // SAFETY: mount_setattr(2) on an O_PATH fd with AT_EMPTY_PATH.
    let rc = unsafe {
        libc::syscall(
            SYS_MOUNT_SETATTR,
            fd,
            empty.as_ptr(),
            AT_EMPTY_PATH | AT_RECURSIVE,
            std::ptr::from_ref(&attr),
            std::mem::size_of::<MountAttr>(),
        )
    };
    // SAFETY: close the O_PATH fd.
    unsafe {
        libc::close(fd);
    }
    if rc != 0 {
        return Err(errno_err(SandboxStage::Mount, "mount_setattr RDONLY"));
    }
    Ok(())
}

fn canonicalize(path: &Path) -> Result<PathBuf, SandboxLinuxError> {
    fs::canonicalize(path).map_err(|err| {
        SandboxLinuxError::setup(
            SandboxStage::Mount,
            format!("canonicalize {}: {err}", path.display()),
        )
    })
}

fn absolute_logical(path: &Path) -> Result<PathBuf, SandboxLinuxError> {
    if !path.is_absolute() {
        return Err(SandboxLinuxError::PolicyDenied {
            reason: format!("bind path must be absolute: {}", path.display()),
        });
    }
    // Normalize `.` / `..` without resolving symlinks (preserve `/bin` vs `/usr/bin`).
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = out.pop();
                if out.as_os_str().is_empty() {
                    out.push("/");
                }
            }
            Component::Normal(c) => out.push(c),
            Component::Prefix(_) => {
                return Err(SandboxLinuxError::PolicyDenied {
                    reason: format!("unexpected path prefix: {}", path.display()),
                });
            }
        }
    }
    let _ = canonicalize(path)?;
    Ok(out)
}

fn strip_root(path: &Path) -> PathBuf {
    path.components()
        .filter(|c| !matches!(c, Component::RootDir | Component::Prefix(_)))
        .collect()
}

fn touch_file(path: &Path) -> Result<(), SandboxLinuxError> {
    if path.exists() {
        return Ok(());
    }
    fs::File::create(path).map_err(|err| {
        SandboxLinuxError::setup(
            SandboxStage::Mount,
            format!("create {}: {err}", path.display()),
        )
    })?;
    Ok(())
}

fn mount_raw(
    source: Option<&Path>,
    target: &Path,
    fstype: Option<&str>,
    flags: u64,
    data: Option<&str>,
    stage: SandboxStage,
    label: &str,
) -> Result<(), SandboxLinuxError> {
    let source_c = match source {
        Some(p) => Some(c_path(p)?),
        None => None,
    };
    let target_c = c_path(target)?;
    let fstype_c = match fstype {
        Some(s) => Some(c_string_str(s)?),
        None => None,
    };
    let data_c = match data {
        Some(s) => Some(c_string_str(s)?),
        None => None,
    };

    // SAFETY: all CStrings are NUL-terminated; flags match mount(2).
    let rc = unsafe {
        libc::mount(
            source_c.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            target_c.as_ptr(),
            fstype_c.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            flags,
            data_c
                .as_ref()
                .map_or(std::ptr::null(), |s| s.as_ptr().cast()),
        )
    };
    if rc != 0 {
        return Err(errno_err(stage, label));
    }
    Ok(())
}

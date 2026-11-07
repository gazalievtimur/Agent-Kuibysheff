//! Junction used as git's process cwd.
//!
//! Git resolves the long path of its working directory by listing every
//! ancestor. `C:\Users` is not listable by an AppContainer on the GitHub
//! runner, and its DACL cannot be extended from this token. A mount point
//! under `ProgramData` is listable, and the junction still names the real
//! working tree.

use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::util::{path_to_wide_null, setup_last};

/// `FSCTL_SET_REPARSE_POINT` from `Win32_System_Ioctl`.
const FSCTL_SET_REPARSE_POINT: u32 = 589_988;
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

static LINK_SEQ: AtomicU64 = AtomicU64::new(0);

/// A directory junction. Drop removes the link and leaves the target in place.
pub(crate) struct CwdJunction {
    link: PathBuf,
}

impl CwdJunction {
    pub(crate) fn mount(target: &Path) -> Result<Self, SandboxWindowsError> {
        let target = std::fs::canonicalize(target).map_err(|err| {
            SandboxWindowsError::setup(SandboxStage::AclGrant, format!("canonicalize cwd: {err}"))
        })?;
        let parent = junction_parent()?;
        std::fs::create_dir_all(&parent).map_err(|err| {
            SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("create {}: {err}", parent.display()),
            )
        })?;
        let link = parent.join(format!(
            "cwd-{}-{}",
            std::process::id(),
            LINK_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        if link.exists() {
            let _ = std::fs::remove_dir(&link);
        }
        std::fs::create_dir(&link).map_err(|err| {
            SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("create {}: {err}", link.display()),
            )
        })?;
        if let Err(err) = set_mount_point(&link, &target) {
            let _ = std::fs::remove_dir(&link);
            return Err(err);
        }
        Ok(Self { link })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.link
    }
}

impl Drop for CwdJunction {
    fn drop(&mut self) {
        // `RemoveDirectoryW` deletes the junction, not the target tree.
        let _ = std::fs::remove_dir(&self.link);
        if let Some(parent) = self.link.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn junction_parent() -> Result<PathBuf, SandboxWindowsError> {
    let root = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    Ok(root.join("agent.kuibysheff"))
}

fn set_mount_point(link: &Path, target: &Path) -> Result<(), SandboxWindowsError> {
    let win_target = strip_verbatim(target);
    let substitute: Vec<u16> = format!(r"\??\{win_target}").encode_utf16().collect();
    let print: Vec<u16> = win_target.encode_utf16().collect();
    let sub_bytes = u16::try_from(substitute.len() * 2).map_err(|_| {
        SandboxWindowsError::setup(SandboxStage::AclGrant, "cwd junction path is too long")
    })?;
    let print_bytes = u16::try_from(print.len() * 2).map_err(|_| {
        SandboxWindowsError::setup(SandboxStage::AclGrant, "cwd junction path is too long")
    })?;
    let print_offset = sub_bytes.saturating_add(2);
    let mut path_units = substitute;
    path_units.push(0);
    path_units.extend(print);
    path_units.push(0);
    let path_bytes = u16::try_from(path_units.len() * 2).map_err(|_| {
        SandboxWindowsError::setup(SandboxStage::AclGrant, "cwd junction path is too long")
    })?;
    // Four USHORTs plus the UTF-16 buffer. The 8-byte header is not included.
    let data_length = 8u16.saturating_add(path_bytes);

    let mut buffer = Vec::with_capacity(8 + data_length as usize);
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend(data_length.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(sub_bytes.to_le_bytes());
    buffer.extend(print_offset.to_le_bytes());
    buffer.extend(print_bytes.to_le_bytes());
    for unit in path_units {
        buffer.extend(unit.to_le_bytes());
    }

    let wide = path_to_wide_null(link);
    // SAFETY: the directory was just created; OPEN_REPARSE_POINT keeps CreateFile on the link.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("CreateFileW({})", link.display()),
        ));
    }
    let mut returned = 0u32;
    // SAFETY: `buffer` is a REPARSE_DATA_BUFFER for a mount point; `handle` owns the link.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            u32::try_from(buffer.len()).unwrap_or(0),
            ptr::null_mut(),
            0,
            &mut returned,
            ptr::null_mut(),
        )
    };
    // SAFETY: `handle` is uniquely owned here.
    unsafe {
        CloseHandle(handle);
    }
    if ok == FALSE {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("FSCTL_SET_REPARSE_POINT({})", link.display()),
        ));
    }
    Ok(())
}

fn strip_verbatim(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::CwdJunction;

    #[test]
    fn junction_writes_through_and_drop_keeps_the_target() {
        let target = tempfile::tempdir().expect("target");
        let link = CwdJunction::mount(target.path()).expect("junction");
        let through = link.path().join("note.txt");
        std::fs::write(&through, b"hi").expect("write through junction");
        assert_eq!(
            std::fs::read(target.path().join("note.txt")).expect("target file"),
            b"hi"
        );
        let link_path = link.path().to_path_buf();
        drop(link);
        assert!(!link_path.exists());
        assert_eq!(
            std::fs::read(target.path().join("note.txt")).expect("target survived"),
            b"hi"
        );
    }
}

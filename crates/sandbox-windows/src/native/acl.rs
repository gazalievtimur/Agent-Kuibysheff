//! Temporary ACL grants for AppContainer package SID, with rollback journal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, FALSE};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_TRAVERSE,
};

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::util::{path_to_wide_null, setup_last};

pub(crate) const ACCESS_READ: u32 = FILE_GENERIC_READ | FILE_LIST_DIRECTORY | FILE_TRAVERSE;
pub(crate) const ACCESS_WRITE: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_LIST_DIRECTORY | FILE_TRAVERSE;
pub(crate) const ACCESS_EXECUTE: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | FILE_TRAVERSE;

/// Strips Windows verbatim (`\\?\`) prefixes that break some security APIs.
pub(crate) fn acl_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn is_protected_system_path(path: &Path) -> bool {
    let path = acl_path(path);
    let Ok(system_root) = std::env::var("SystemRoot") else {
        return false;
    };
    let system_root = PathBuf::from(system_root);
    let path_s = path.to_string_lossy();
    let root_s = system_root.to_string_lossy();
    path_s
        .to_ascii_lowercase()
        .starts_with(&root_s.to_ascii_lowercase())
}

/// One path whose DACL was mutated for the AppContainer SID.
struct AclGrant {
    path: PathBuf,
    original_sd: PSECURITY_DESCRIPTOR,
}

/// Journal of ACL grants; Drop restores every original DACL.
pub struct AclJournal {
    grants: Vec<AclGrant>,
    seen: HashSet<PathBuf>,
}

impl AclJournal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            grants: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// Grants `access` on `path` for `package_sid`, saving the prior DACL for restore.
    pub fn grant_path(
        &mut self,
        path: &Path,
        package_sid: PSID,
        access: u32,
        inherit: bool,
    ) -> Result<(), SandboxWindowsError> {
        let canonical = acl_path(&std::fs::canonicalize(path).map_err(|err| {
            SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("canonicalize({}): {err}", path.display()),
            )
        })?);
        self.grant_canonical(&canonical, package_sid, access, inherit)
    }

    fn grant_canonical(
        &mut self,
        path: &Path,
        package_sid: PSID,
        access: u32,
        inherit: bool,
    ) -> Result<(), SandboxWindowsError> {
        let path_buf = acl_path(path);
        if !self.seen.insert(path_buf.clone()) {
            return Ok(());
        }

        // System roots already expose RX to AppContainers via OS ACLs; mutating them
        // often fails with ERROR_INVALID_PARAMETER / ACCESS_DENIED for non-admin hosts.
        if is_protected_system_path(&path_buf) {
            return Ok(());
        }

        match self.try_grant_dacl(&path_buf, package_sid, access, inherit) {
            Ok(()) => Ok(()),
            // Some third-party install trees (e.g. C:\Python312) reject inheritable ACE
            // updates with ERROR_INVALID_PARAMETER; retry without inheritance.
            Err(first) if inherit => match self.try_grant_dacl(&path_buf, package_sid, access, false)
            {
                Ok(()) => Ok(()),
                Err(_) => {
                    self.seen.remove(&path_buf);
                    Err(first)
                }
            },
            Err(err) => {
                self.seen.remove(&path_buf);
                Err(err)
            }
        }
    }

    fn try_grant_dacl(
        &mut self,
        path_buf: &Path,
        package_sid: PSID,
        access: u32,
        inherit: bool,
    ) -> Result<(), SandboxWindowsError> {
        let wide = path_to_wide_null(path_buf);
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let mut dacl: *mut ACL = ptr::null_mut();

        // SAFETY: GetNamedSecurityInfoW allocates a security descriptor on success.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut sd,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(setup_last(
                SandboxStage::AclGrant,
                &format!("GetNamedSecurityInfoW({})", path_buf.display()),
            ));
        }

        let trustee = TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: package_sid.cast(),
        };

        let mut ea = EXPLICIT_ACCESS_W {
            grfAccessPermissions: access,
            grfAccessMode: windows_sys::Win32::Security::Authorization::GRANT_ACCESS,
            grfInheritance: if inherit {
                SUB_CONTAINERS_AND_OBJECTS_INHERIT
            } else {
                0
            },
            Trustee: trustee,
        };

        let mut new_dacl: *mut ACL = ptr::null_mut();
        // SAFETY: SetEntriesInAclW merges `ea` into `dacl` and allocates `new_dacl`.
        let acl_status = unsafe { SetEntriesInAclW(1, &mut ea, dacl, &mut new_dacl) };
        if acl_status != ERROR_SUCCESS {
            // SAFETY: free the SD we just retrieved before returning.
            unsafe {
                LocalFree(sd.cast());
            }
            return Err(SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("SetEntriesInAclW failed status={acl_status}"),
            ));
        }

        // SAFETY: SetNamedSecurityInfoW applies `new_dacl` to the named object.
        let set_status = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                new_dacl,
                ptr::null_mut(),
            )
        };
        // SAFETY: new_dacl was allocated by SetEntriesInAclW.
        unsafe {
            LocalFree(new_dacl.cast());
        }
        if set_status != ERROR_SUCCESS {
            unsafe {
                LocalFree(sd.cast());
            }
            return Err(setup_last(
                SandboxStage::AclGrant,
                &format!("SetNamedSecurityInfoW({})", path_buf.display()),
            ));
        }

        self.grants.push(AclGrant {
            path: path_buf.to_path_buf(),
            original_sd: sd,
        });
        Ok(())
    }

    /// Restores all grants in reverse order (best-effort).
    pub fn restore_all(&mut self) {
        while let Some(grant) = self.grants.pop() {
            restore_one(grant);
        }
    }
}

impl Default for AclJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AclJournal {
    fn drop(&mut self) {
        self.restore_all();
    }
}

fn restore_one(grant: AclGrant) {
    let mut saved_dacl: *mut ACL = ptr::null_mut();
    let mut present: i32 = FALSE;
    let mut defaulted: i32 = FALSE;
    // SAFETY: GetSecurityDescriptorDacl reads fields from the owned original SD.
    let ok = unsafe {
        GetSecurityDescriptorDacl(
            grant.original_sd,
            &mut present,
            &mut saved_dacl,
            &mut defaulted,
        )
    };
    if ok != FALSE && present != FALSE {
        let wide = path_to_wide_null(&acl_path(&grant.path));
        // SAFETY: restore the pre-grant DACL onto the path.
        let _ = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                saved_dacl,
                ptr::null_mut(),
            )
        };
    }
    // SAFETY: original_sd was allocated by GetNamedSecurityInfoW and is uniquely owned.
    unsafe {
        LocalFree(grant.original_sd.cast());
    }
}

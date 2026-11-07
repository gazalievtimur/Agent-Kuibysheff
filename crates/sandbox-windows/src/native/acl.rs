//! Temporary ACL grants for AppContainer package SID, with rollback journal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_SUCCESS, FALSE, INVALID_HANDLE_VALUE, LUID, TRUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetNamedSecurityInfoW, ProgressInvokeNever, SetEntriesInAclW,
    SetNamedSecurityInfoW, TreeSetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
    REVOKE_ACCESS, SE_FILE_OBJECT, TREE_SEC_INFO_SET, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AclSizeInformation, AdjustTokenPrivileges, EqualSid, GetAce, GetAclInformation,
    GetSecurityDescriptorDacl, InitializeSecurityDescriptor, LookupPrivilegeValueW,
    MakeSelfRelativeSD, SetKernelObjectSecurity, SetSecurityDescriptorDacl, ACCESS_ALLOWED_ACE,
    ACE_HEADER, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, LUID_AND_ATTRIBUTES,
    PSECURITY_DESCRIPTOR, PSID, SECURITY_DESCRIPTOR, SE_PRIVILEGE_ENABLED,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, OPEN_EXISTING, WRITE_DAC,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::util::{path_to_wide_null, setup_last, to_wide_null};

/// SID on every AppContainer token. Program Files already grants this read/execute.
const ALL_APPLICATION_PACKAGES_SDDL: &str = "S-1-15-2-1";
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const OBJECT_INHERIT_ACE: u8 = 1;
const CONTAINER_INHERIT_ACE: u8 = 2;

pub(crate) const ACCESS_READ: u32 = FILE_GENERIC_READ | FILE_LIST_DIRECTORY | FILE_TRAVERSE;
pub(crate) const ACCESS_WRITE: u32 = FILE_GENERIC_READ
    | FILE_GENERIC_WRITE
    | FILE_GENERIC_EXECUTE
    | FILE_LIST_DIRECTORY
    | FILE_TRAVERSE
    | DELETE
    | FILE_DELETE_CHILD;
pub(crate) const ACCESS_EXECUTE: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | FILE_TRAVERSE;

/// Ancestors of `path` that still have a parent, nearest first.
///
/// The drive root is omitted: AppContainer can already traverse it, and its
/// DACL is not ours to edit.
pub(crate) fn traverse_ancestors(path: &Path) -> Vec<PathBuf> {
    let mut ancestors = Vec::new();
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        if parent.parent().is_none() {
            break;
        }
        ancestors.push(parent.to_path_buf());
        current = parent.to_path_buf();
    }
    ancestors
}

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

/// OS install locations AppContainer can already read and execute.
///
/// Mutating their DACLs fails for a non-admin host and is unnecessary: Windows
/// grants `ALL APPLICATION PACKAGES` read/execute on these trees.
pub(crate) fn is_os_code_plane(path: &Path) -> bool {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path = acl_path(&canonical);
    os_code_roots()
        .iter()
        .any(|root| path_is_within_root(root, &path))
}

/// First directory under a Program Files root (`...\Git` for `...\Git\cmd\git.exe`).
pub(crate) fn program_files_product_root(executable: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
    product_root_under(&program_files_roots(), &acl_path(&canonical))
}

fn os_code_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        roots.push(PathBuf::from(system_root));
    }
    roots.extend(program_files_roots());
    roots
}

fn program_files_roots() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(PathBuf::from))
        .collect()
}

/// `candidate` equals `root` or is a descendant, compared per path component.
///
/// A string prefix would treat `C:\Program Files` as a parent of
/// `C:\Program Files (x86)`.
pub(crate) fn path_is_within_root(root: &Path, candidate: &Path) -> bool {
    if components_eq_ci(root, candidate) {
        return true;
    }
    strip_prefix_ci(root, candidate).is_some()
}

pub(crate) fn product_root_under(roots: &[PathBuf], executable: &Path) -> Option<PathBuf> {
    for root in roots {
        let rest = strip_prefix_ci(root, executable)?;
        let name = rest.components().next()?;
        let std::path::Component::Normal(name) = name else {
            continue;
        };
        return Some(root.join(name));
    }
    None
}

fn components_eq_ci(left: &Path, right: &Path) -> bool {
    let left: Vec<String> = normal_components(left);
    let right: Vec<String> = normal_components(right);
    left == right
}

fn strip_prefix_ci(root: &Path, path: &Path) -> Option<PathBuf> {
    let root_comps = normal_components(root);
    let path_norm = normal_components(path);
    if path_norm.len() <= root_comps.len() || path_norm[..root_comps.len()] != root_comps[..] {
        return None;
    }
    let mut rest = PathBuf::new();
    for part in path.components().skip(root.components().count()) {
        rest.push(part);
    }
    if rest.as_os_str().is_empty() {
        None
    } else {
        Some(rest)
    }
}

fn normal_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect()
}

fn is_protected_tree(path: &Path) -> bool {
    let comps: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    comps
        .windows(2)
        .any(|pair| pair[0] == ".kuibysheff" && pair[1] == "protected")
}

fn is_reparse_point(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

enum AclRestore {
    /// Full DACL captured before the grant.
    Snapshot(PSECURITY_DESCRIPTOR),
    /// ACE added for the package SID; cleanup revokes that trustee only.
    RevokePackageSid,
}

/// One path whose DACL was mutated for the AppContainer SID.
struct AclGrant {
    path: PathBuf,
    object_type: i32,
    restore: AclRestore,
    /// Children inherited the root ACE through `TreeSetNamedSecurityInfoW`.
    propagated: bool,
    /// Set with `SetKernelObjectSecurity`, which does not walk child objects.
    /// `SetNamedSecurityInfoW` on a profile directory rewrites the whole tree.
    object_only: bool,
}

/// Previous privilege state for the host process, restored on drop.
#[repr(C)]
#[derive(Clone, Copy)]
struct PrivilegeUpdate {
    count: u32,
    entries: [LUID_AND_ATTRIBUTES; 3],
}

/// Enables backup/restore on the host token when those privileges are already present.
///
/// An elevated GitHub runner can then add a traverse ACE on `C:\Users`, which a
/// normal owner-only `WRITE_DAC` open cannot change. A filtered token has none
/// of these privileges, and enabling is a no-op.
pub(crate) struct HostPrivilegeGuard {
    token: windows_sys::Win32::Foundation::HANDLE,
    previous: PrivilegeUpdate,
    active: bool,
}

impl HostPrivilegeGuard {
    pub(crate) fn enable() -> Self {
        let mut guard = Self {
            token: ptr::null_mut(),
            previous: PrivilegeUpdate {
                count: 0,
                entries: [LUID_AND_ATTRIBUTES {
                    Luid: LUID {
                        LowPart: 0,
                        HighPart: 0,
                    },
                    Attributes: 0,
                }; 3],
            },
            active: false,
        };
        // SAFETY: the process pseudo-handle is only used to open a real token.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut guard.token,
            )
        };
        if opened == FALSE || guard.token.is_null() {
            guard.token = ptr::null_mut();
            return guard;
        }

        let mut update = guard.previous;
        for name in [
            "SeBackupPrivilege",
            "SeRestorePrivilege",
            "SeTakeOwnershipPrivilege",
        ] {
            let wide = to_wide_null(name);
            let mut luid = LUID {
                LowPart: 0,
                HighPart: 0,
            };
            // SAFETY: `wide` is NUL-terminated; `luid` is an out parameter.
            let found = unsafe { LookupPrivilegeValueW(ptr::null(), wide.as_ptr(), &mut luid) };
            if found == FALSE {
                continue;
            }
            let index = update.count as usize;
            update.entries[index] = LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            };
            update.count += 1;
        }
        if update.count == 0 {
            return guard;
        }

        let mut previous_len = 0u32;
        // SAFETY: `update` matches TOKEN_PRIVILEGES for up to three entries.
        // `previous` receives the prior attributes of those same privileges.
        let adjusted = unsafe {
            AdjustTokenPrivileges(
                guard.token,
                FALSE,
                ptr::from_ref(&update).cast::<TOKEN_PRIVILEGES>(),
                u32::try_from(std::mem::size_of::<PrivilegeUpdate>()).unwrap_or(0),
                ptr::from_mut(&mut guard.previous).cast::<TOKEN_PRIVILEGES>(),
                &mut previous_len,
            )
        };
        guard.active = adjusted != FALSE && guard.previous.count > 0;
        guard
    }
}

impl Drop for HostPrivilegeGuard {
    fn drop(&mut self) {
        if !self.token.is_null() {
            if self.active {
                // SAFETY: `previous` is the buffer filled by AdjustTokenPrivileges.
                unsafe {
                    AdjustTokenPrivileges(
                        self.token,
                        FALSE,
                        ptr::from_ref(&self.previous).cast::<TOKEN_PRIVILEGES>(),
                        0,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                }
            }
            // SAFETY: token was opened for this guard and is uniquely owned.
            unsafe {
                CloseHandle(self.token);
            }
        }
    }
}

/// Journal of ACL grants; Drop restores every original DACL.
pub struct AclJournal {
    grants: Vec<AclGrant>,
    seen: HashSet<PathBuf>,
    /// Package SID for revoke-on-cleanup grants. Not owned; valid for the journal lifetime.
    package_sid: PSID,
}

impl AclJournal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            grants: Vec::new(),
            seen: HashSet::new(),
            package_sid: ptr::null_mut(),
        }
    }

    /// Grants `access` on `path` and on existing children, skipping reparse points.
    ///
    /// OS code-plane paths and `.kuibysheff/protected` are left untouched.
    /// The root keeps an inheritable ACE so files created during the run stay
    /// reachable. Existing children receive that ACE in one tree update.
    /// A tree that already contains `.kuibysheff/protected` is walked
    /// file-by-file so that directory is skipped.
    pub fn grant_tree(
        &mut self,
        path: &Path,
        package_sid: PSID,
        access: u32,
    ) -> Result<(), SandboxWindowsError> {
        let canonical = acl_path(&std::fs::canonicalize(path).map_err(|err| {
            SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("canonicalize({}): {err}", path.display()),
            )
        })?);
        if is_os_code_plane(&canonical) || is_protected_tree(&canonical) {
            return Ok(());
        }
        self.grant_canonical(&canonical, package_sid, access, true)?;
        let is_dir = std::fs::symlink_metadata(&canonical)
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if is_dir && !is_reparse_point(&canonical) {
            self.propagate_children(&canonical, package_sid, access)?;
        }
        Ok(())
    }

    /// Read/execute for a runtime root, stored on the folder.
    ///
    /// The ACE belongs to `ALL APPLICATION PACKAGES` and is inheritable, so a
    /// later launch only checks the folder and one child. A tree that contains
    /// `.kuibysheff/protected` keeps the per-run package grant instead.
    pub fn grant_runtime_root(
        &mut self,
        path: &Path,
        package_sid: PSID,
    ) -> Result<(), SandboxWindowsError> {
        let canonical = acl_path(&std::fs::canonicalize(path).map_err(|err| {
            SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("canonicalize({}): {err}", path.display()),
            )
        })?);
        if is_os_code_plane(&canonical) || is_protected_tree(&canonical) {
            return Ok(());
        }
        let is_dir = std::fs::symlink_metadata(&canonical)
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if !is_dir || is_reparse_point(&canonical) {
            return self.grant_tree(&canonical, package_sid, ACCESS_EXECUTE);
        }
        let app_packages = LocalSid::from_sddl(ALL_APPLICATION_PACKAGES_SDDL)?;
        let child_missing = first_regular_child(&canonical)
            .is_some_and(|child| !acl_has_allow_sid(&child, app_packages.as_ptr(), false));
        let folder_ready = acl_has_allow_sid(&canonical, app_packages.as_ptr(), true);
        if folder_ready && !child_missing {
            return Ok(());
        }
        if contains_protected_descendant(&canonical) {
            return self.grant_tree(&canonical, package_sid, ACCESS_EXECUTE);
        }
        if !folder_ready {
            write_dacl(
                &canonical,
                app_packages.as_ptr(),
                ACCESS_EXECUTE,
                GRANT_ACCESS,
                true,
            )?;
        }
        if child_missing {
            tree_set_current_dacl(&canonical)?;
        }
        Ok(())
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

    /// Grants traverse on each ancestor of `path`, stopping above the drive root.
    ///
    /// AppContainer tokens do not receive the Users group. A grant on the leaf
    /// directory does not let `getcwd` walk `C:\Users\...` parents. Traverse and
    /// read-attributes do not allow listing those directories or reading files
    /// inside them. Failures are ignored so a locked ancestor does not abort
    /// the launch.
    pub fn grant_ancestor_traverse(&mut self, path: &Path, package_sid: PSID) {
        let Ok(canonical) = std::fs::canonicalize(path) else {
            return;
        };
        // Git opens the working directory, then asks for its long path. That
        // walk lists every ancestor. File contents stay unread: this mask has
        // no FILE_READ_DATA. The ACE is removed when the launch journal drops.
        let access = FILE_TRAVERSE | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY;
        for ancestor in traverse_ancestors(&acl_path(&canonical)) {
            let _ = self.try_grant_object_dacl(&ancestor, package_sid, access);
        }
    }

    /// Adds one ACE to `path` only. Does not propagate to children.
    fn try_grant_object_dacl(
        &mut self,
        path: &Path,
        package_sid: PSID,
        access: u32,
    ) -> Result<(), SandboxWindowsError> {
        let path_buf = match std::fs::canonicalize(path) {
            Ok(path) => acl_path(&path),
            Err(err) => {
                return Err(SandboxWindowsError::setup(
                    SandboxStage::AclGrant,
                    format!("canonicalize({}): {err}", path.display()),
                ));
            }
        };
        if !self.seen.insert(path_buf.clone()) {
            return Ok(());
        }
        if is_os_code_plane(&path_buf) || is_protected_tree(&path_buf) {
            return Ok(());
        }
        match write_object_dacl(&path_buf, package_sid, access) {
            Ok(sd) => {
                self.grants.push(AclGrant {
                    path: path_buf,
                    object_type: SE_FILE_OBJECT,
                    restore: AclRestore::Snapshot(sd),
                    propagated: false,
                    object_only: true,
                });
                Ok(())
            }
            Err(err) => {
                self.seen.remove(&path_buf);
                Err(err)
            }
        }
    }

    /// Grants `path` itself and does not resolve a mount point.
    ///
    /// `canonicalize` would follow the junction and put the ACE on the target
    /// instead of the link git actually walks.
    pub fn grant_reparse_point(
        &mut self,
        path: &Path,
        package_sid: PSID,
        access: u32,
    ) -> Result<(), SandboxWindowsError> {
        let path_buf = acl_path(path);
        if !self.seen.insert(path_buf.clone()) {
            return Ok(());
        }
        match write_object_dacl(&path_buf, package_sid, access) {
            Ok(sd) => {
                self.grants.push(AclGrant {
                    path: path_buf,
                    object_type: SE_FILE_OBJECT,
                    restore: AclRestore::Snapshot(sd),
                    propagated: false,
                    object_only: true,
                });
                Ok(())
            }
            Err(err) => {
                self.seen.remove(&path_buf);
                Err(err)
            }
        }
    }

    /// Lets the package open the Windows NUL device read-write.
    ///
    /// Git maps `/dev/null` to `NUL` and opens it that way during startup
    /// (`sanitize_stdfds`). Some AppContainer hosts deny that open until the
    /// package SID is on the device DACL. Changing the DACL needs `WRITE_DAC`,
    /// which a non-admin token often lacks; those hosts already allow the open,
    /// so a failure is ignored and the launch continues.
    pub fn grant_nul_device(&mut self, package_sid: PSID) {
        let access = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
        // Best-effort: a non-admin token gets ACCESS_DENIED here. Hosts that
        // already let AppContainers open NUL do not need the ACE.
        let _ = self.try_grant_named(r"\\.\NUL", SE_FILE_OBJECT, package_sid, access);
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

        // OS code roots already expose RX to AppContainers. Mutating them fails
        // with ACCESS_DENIED for a non-admin host.
        if is_os_code_plane(&path_buf) {
            return Ok(());
        }

        match self.try_grant_dacl(&path_buf, package_sid, access, inherit) {
            Ok(()) => Ok(()),
            // Some third-party install trees (e.g. C:\Python312) reject inheritable ACE
            // updates with ERROR_INVALID_PARAMETER; retry without inheritance.
            Err(first) if inherit => {
                match self.try_grant_dacl(&path_buf, package_sid, access, false) {
                    Ok(()) => Ok(()),
                    Err(_) => {
                        self.seen.remove(&path_buf);
                        Err(first)
                    }
                }
            }
            Err(err) => {
                self.seen.remove(&path_buf);
                Err(err)
            }
        }
    }

    fn try_grant_named(
        &mut self,
        name: &str,
        object_type: i32,
        package_sid: PSID,
        access: u32,
    ) -> Result<(), SandboxWindowsError> {
        let wide = to_wide_null(name);
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let mut dacl: *mut ACL = ptr::null_mut();
        // SAFETY: GetNamedSecurityInfoW allocates a security descriptor on success.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                object_type,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut sd,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("GetNamedSecurityInfoW({name}) status={status}"),
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
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: trustee,
        };
        let mut new_dacl: *mut ACL = ptr::null_mut();
        // SAFETY: SetEntriesInAclW merges `ea` into `dacl` and allocates `new_dacl`.
        let acl_status = unsafe { SetEntriesInAclW(1, &mut ea, dacl, &mut new_dacl) };
        if acl_status != ERROR_SUCCESS {
            // SAFETY: `sd` was allocated by GetNamedSecurityInfoW and is uniquely owned here.
            unsafe {
                LocalFree(sd.cast());
            }
            return Err(SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("SetEntriesInAclW({name}) status={acl_status}"),
            ));
        }
        // SAFETY: SetNamedSecurityInfoW applies `new_dacl` to the named object.
        let set_status = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                object_type,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                new_dacl,
                ptr::null_mut(),
            )
        };
        // SAFETY: `new_dacl` was allocated by SetEntriesInAclW.
        unsafe {
            LocalFree(new_dacl.cast());
        }
        if set_status != ERROR_SUCCESS {
            // SAFETY: `sd` was allocated by GetNamedSecurityInfoW and is uniquely owned here.
            unsafe {
                LocalFree(sd.cast());
            }
            return Err(SandboxWindowsError::setup(
                SandboxStage::AclGrant,
                format!("SetNamedSecurityInfoW({name}) status={set_status}"),
            ));
        }
        self.grants.push(AclGrant {
            path: PathBuf::from(name),
            object_type,
            restore: AclRestore::Snapshot(sd),
            propagated: false,
            object_only: false,
        });
        Ok(())
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
            // SAFETY: sd was allocated by GetNamedSecurityInfoW and is uniquely owned here.
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
            object_type: SE_FILE_OBJECT,
            restore: AclRestore::Snapshot(sd),
            propagated: false,
            object_only: false,
        });
        Ok(())
    }

    fn propagate_children(
        &mut self,
        root: &Path,
        package_sid: PSID,
        access: u32,
    ) -> Result<(), SandboxWindowsError> {
        if contains_protected_descendant(root) {
            return self.propagate_children_manual(root, package_sid, access);
        }
        tree_set_current_dacl(root)?;
        if let Some(grant) = self.grants.last_mut() {
            if grant.path == root {
                grant.propagated = true;
            }
        }
        Ok(())
    }

    fn propagate_children_manual(
        &mut self,
        root: &Path,
        package_sid: PSID,
        access: u32,
    ) -> Result<(), SandboxWindowsError> {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).map_err(|err| {
                SandboxWindowsError::setup(
                    SandboxStage::AclGrant,
                    format!("read_dir {}: {err}", dir.display()),
                )
            })?;
            for entry in entries {
                let entry = entry.map_err(|err| {
                    SandboxWindowsError::setup(
                        SandboxStage::AclGrant,
                        format!("read_dir {}: {err}", dir.display()),
                    )
                })?;
                let path = entry.path();
                if is_protected_tree(&path) {
                    continue;
                }
                let reparse = is_reparse_point(&path);
                let is_dir = std::fs::symlink_metadata(&path)
                    .map(|meta| meta.is_dir())
                    .unwrap_or(false);
                let inherit = is_dir && !reparse;
                self.grant_revocable(&path, package_sid, access, inherit)?;
                if inherit {
                    stack.push(path);
                }
            }
        }
        Ok(())
    }

    fn grant_revocable(
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
        if is_os_code_plane(&path_buf) || is_protected_tree(&path_buf) {
            return Ok(());
        }
        self.package_sid = package_sid;
        if let Err(err) = write_dacl(&path_buf, package_sid, access, GRANT_ACCESS, inherit) {
            self.seen.remove(&path_buf);
            return Err(err);
        }
        self.grants.push(AclGrant {
            path: path_buf,
            object_type: SE_FILE_OBJECT,
            restore: AclRestore::RevokePackageSid,
            propagated: false,
            object_only: false,
        });
        Ok(())
    }

    /// Restores all grants in reverse order (best-effort).
    pub fn restore_all(&mut self) {
        let package_sid = self.package_sid;
        while let Some(grant) = self.grants.pop() {
            restore_one(grant, package_sid);
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

fn restore_one(grant: AclGrant, package_sid: PSID) {
    let propagated = grant.propagated;
    let path = grant.path.clone();
    match grant.restore {
        AclRestore::Snapshot(original_sd) => {
            if grant.object_only {
                restore_object_dacl(&path, original_sd);
            } else {
                restore_snapshot(&path, grant.object_type, original_sd);
            }
        }
        AclRestore::RevokePackageSid => {
            if !package_sid.is_null() {
                let _ = write_dacl(&path, package_sid, 0, REVOKE_ACCESS, false);
            }
        }
    }
    if propagated {
        // The restored root DACL no longer contains the package ACE. Pushing
        // that DACL back down drops the inherited copy from existing children.
        let _ = tree_set_current_dacl(&path);
    }
}

/// Applies the path's current DACL to the path and its inheriting descendants.
///
/// `TreeSetNamedSecurityInfoW` does not follow reparse points. Callers must
/// not use this on a tree that contains `.kuibysheff/protected`.
fn tree_set_current_dacl(path: &Path) -> Result<(), SandboxWindowsError> {
    let wide = path_to_wide_null(path);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: GetNamedSecurityInfoW allocates `sd` on success; `dacl` points into it.
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
            &format!("GetNamedSecurityInfoW({})", path.display()),
        ));
    }
    // SAFETY: `dacl` stays alive until this call returns; the kernel copies it
    // onto each descendant. `sd` is freed immediately afterwards.
    let tree_status = unsafe {
        TreeSetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
            TREE_SEC_INFO_SET,
            None,
            ProgressInvokeNever,
            ptr::null(),
        )
    };
    // SAFETY: `sd` was allocated by GetNamedSecurityInfoW and is uniquely owned here.
    unsafe {
        LocalFree(sd.cast());
    }
    if tree_status != ERROR_SUCCESS {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("TreeSetNamedSecurityInfoW({})", path.display()),
        ));
    }
    Ok(())
}

/// True when a descendant is `.kuibysheff/protected`. Reparse points are not entered.
fn contains_protected_descendant(root: &Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return true;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_protected_tree(&path) {
                return true;
            }
            if is_reparse_point(&path) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    false
}

struct LocalSid(PSID);

impl LocalSid {
    fn from_sddl(sddl: &str) -> Result<Self, SandboxWindowsError> {
        let wide = to_wide_null(sddl);
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: ConvertStringSidToSidW writes an allocated SID on success.
        let ok = unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut sid) };
        if ok == FALSE || sid.is_null() {
            return Err(setup_last(
                SandboxStage::AclGrant,
                &format!("ConvertStringSidToSidW({sddl})"),
            ));
        }
        Ok(Self(sid))
    }

    fn as_ptr(&self) -> PSID {
        self.0
    }
}

impl Drop for LocalSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: ConvertStringSidToSidW allocated this SID with LocalAlloc.
            unsafe {
                LocalFree(self.0.cast());
            }
        }
    }
}

fn first_regular_child(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if is_protected_tree(&path) || is_reparse_point(&path) {
            continue;
        }
        return Some(path);
    }
    None
}

fn acl_has_allow_sid(path: &Path, sid: PSID, require_inherit: bool) -> bool {
    if sid.is_null() {
        return false;
    }
    let wide = path_to_wide_null(&acl_path(path));
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: GetNamedSecurityInfoW allocates `sd` on success.
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
        return false;
    }
    let found = dacl_has_allow_sid(dacl, sid, require_inherit);
    // SAFETY: `sd` was allocated by GetNamedSecurityInfoW and is uniquely owned here.
    unsafe {
        LocalFree(sd.cast());
    }
    found
}

fn dacl_has_allow_sid(dacl: *mut ACL, sid: PSID, require_inherit: bool) -> bool {
    if dacl.is_null() {
        return false;
    }
    let mut info = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: `dacl` points into a security descriptor that is still allocated.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            ptr::addr_of_mut!(info).cast(),
            u32::try_from(std::mem::size_of::<ACL_SIZE_INFORMATION>()).unwrap_or(0),
            AclSizeInformation,
        )
    };
    if ok == FALSE {
        return false;
    }
    for index in 0..info.AceCount {
        let mut ace: *mut core::ffi::c_void = ptr::null_mut();
        // SAFETY: `index` is within the ACE count of this ACL.
        let got = unsafe { GetAce(dacl, index, &mut ace) };
        if got == FALSE || ace.is_null() {
            continue;
        }
        // SAFETY: GetAce returned an ACE that lives inside `dacl`.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        if require_inherit
            && (header.AceFlags & OBJECT_INHERIT_ACE == 0
                || header.AceFlags & CONTAINER_INHERIT_ACE == 0)
        {
            continue;
        }
        // SAFETY: an access-allowed ACE begins with ACE_HEADER; SidStart is the SID.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let ace_sid: PSID = ptr::addr_of!(allowed.SidStart).cast_mut().cast();
        // SAFETY: `sid` is non-null and `ace_sid` points at the embedded ACE SID.
        let same = unsafe { EqualSid(ace_sid, sid) };
        if same != FALSE {
            return true;
        }
    }
    false
}

fn write_object_dacl(
    path_buf: &Path,
    package_sid: PSID,
    access: u32,
) -> Result<PSECURITY_DESCRIPTOR, SandboxWindowsError> {
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
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
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

    let applied = set_object_dacl(path_buf, new_dacl);
    // SAFETY: new_dacl was allocated by SetEntriesInAclW. set_object_dacl copies it.
    unsafe {
        LocalFree(new_dacl.cast());
    }
    if let Err(err) = applied {
        // SAFETY: sd was allocated by GetNamedSecurityInfoW and is uniquely owned here.
        unsafe {
            LocalFree(sd.cast());
        }
        return Err(err);
    }
    Ok(sd)
}

fn restore_object_dacl(path: &Path, original_sd: PSECURITY_DESCRIPTOR) {
    let mut saved_dacl: *mut ACL = ptr::null_mut();
    let mut present: i32 = FALSE;
    let mut defaulted: i32 = FALSE;
    // SAFETY: GetSecurityDescriptorDacl reads fields from the owned original SD.
    let ok = unsafe {
        GetSecurityDescriptorDacl(original_sd, &mut present, &mut saved_dacl, &mut defaulted)
    };
    if ok != FALSE && present != FALSE {
        let _ = set_object_dacl(&acl_path(path), saved_dacl);
    }
    // SAFETY: original_sd was allocated by GetNamedSecurityInfoW and is uniquely owned.
    unsafe {
        LocalFree(original_sd.cast());
    }
}

/// Writes `dacl` onto `path` alone.
///
/// `SetNamedSecurityInfoW` reapplies inheritable ACEs to every child, which
/// rewrites a profile tree. `SetKernelObjectSecurity` updates one object.
fn set_object_dacl(path: &Path, dacl: *mut ACL) -> Result<(), SandboxWindowsError> {
    let mut absolute: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
    // SAFETY: absolute is a local descriptor. `dacl` stays alive for the copy below.
    let ready = unsafe {
        InitializeSecurityDescriptor(
            ptr::addr_of_mut!(absolute).cast(),
            SECURITY_DESCRIPTOR_REVISION,
        ) != FALSE
            && SetSecurityDescriptorDacl(
                ptr::addr_of_mut!(absolute).cast(),
                TRUE,
                dacl.cast(),
                FALSE,
            ) != FALSE
    };
    if !ready {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("InitializeSecurityDescriptor({})", path.display()),
        ));
    }

    let mut size = 0u32;
    // SAFETY: a null buffer asks for the self-relative size.
    unsafe {
        MakeSelfRelativeSD(
            ptr::addr_of_mut!(absolute).cast(),
            ptr::null_mut(),
            &mut size,
        );
    }
    if size == 0 {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("MakeSelfRelativeSD({})", path.display()),
        ));
    }
    let mut relative = vec![0u8; size as usize];
    // SAFETY: `relative` has the size reported above; the call copies `dacl` into it.
    let made = unsafe {
        MakeSelfRelativeSD(
            ptr::addr_of_mut!(absolute).cast(),
            relative.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if made == FALSE {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("MakeSelfRelativeSD({})", path.display()),
        ));
    }

    let wide = path_to_wide_null(&acl_path(path));
    // SAFETY: WRITE_DAC opens this directory only; OPEN_REPARSE_POINT does not follow a junction.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            WRITE_DAC,
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
            &format!("CreateFileW(WRITE_DAC, {})", path.display()),
        ));
    }
    // SAFETY: `handle` is the directory opened above; `relative` is a self-relative SD.
    let set_ok = unsafe {
        SetKernelObjectSecurity(
            handle,
            DACL_SECURITY_INFORMATION,
            relative.as_mut_ptr().cast(),
        )
    };
    // SAFETY: `handle` is uniquely owned here.
    unsafe {
        CloseHandle(handle);
    }
    if set_ok == FALSE {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("SetKernelObjectSecurity({})", path.display()),
        ));
    }
    Ok(())
}

fn restore_snapshot(path: &Path, object_type: i32, original_sd: PSECURITY_DESCRIPTOR) {
    let mut saved_dacl: *mut ACL = ptr::null_mut();
    let mut present: i32 = FALSE;
    let mut defaulted: i32 = FALSE;
    // SAFETY: GetSecurityDescriptorDacl reads fields from the owned original SD.
    let ok = unsafe {
        GetSecurityDescriptorDacl(original_sd, &mut present, &mut saved_dacl, &mut defaulted)
    };
    if ok != FALSE && present != FALSE {
        let wide = path_to_wide_null(&acl_path(path));
        // SAFETY: restore the pre-grant DACL onto the path.
        let _ = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                object_type,
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
        LocalFree(original_sd.cast());
    }
}

fn write_dacl(
    path_buf: &Path,
    package_sid: PSID,
    access: u32,
    mode: i32,
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
        grfAccessMode: mode,
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
    // SAFETY: both allocations are uniquely owned here.
    unsafe {
        LocalFree(new_dacl.cast());
        LocalFree(sd.cast());
    }
    if set_status != ERROR_SUCCESS {
        return Err(setup_last(
            SandboxStage::AclGrant,
            &format!("SetNamedSecurityInfoW({})", path_buf.display()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        acl_has_allow_sid, is_protected_tree, path_is_within_root, product_root_under, AclJournal,
        LocalSid, ALL_APPLICATION_PACKAGES_SDDL,
    };
    use std::path::{Path, PathBuf};
    use std::ptr;

    #[test]
    fn program_files_is_not_a_prefix_of_x86() {
        let root = Path::new(r"C:\Program Files");
        let git = Path::new(r"C:\Program Files\Git\cmd\git.exe");
        let x86 = Path::new(r"C:\Program Files (x86)\Git\cmd\git.exe");
        assert!(path_is_within_root(root, git));
        assert!(!path_is_within_root(root, x86));
        assert!(path_is_within_root(
            Path::new(r"C:\Program Files (x86)"),
            x86
        ));
    }

    #[test]
    fn windows_old_is_outside_system_root() {
        let root = Path::new(r"C:\Windows");
        assert!(path_is_within_root(
            root,
            Path::new(r"C:\Windows\System32\cmd.exe")
        ));
        assert!(!path_is_within_root(
            root,
            Path::new(r"C:\Windows.old\System32\cmd.exe")
        ));
    }

    #[test]
    fn object_only_grant_does_not_reach_children() {
        let profile =
            crate::native::profile::AppContainerProfile::create_unique().expect("profile");
        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("parent");
        let child = parent.join("child.txt");
        std::fs::create_dir(&parent).expect("parent");
        std::fs::write(&child, b"x").expect("child");
        let access =
            super::FILE_TRAVERSE | super::FILE_READ_ATTRIBUTES | super::FILE_LIST_DIRECTORY;
        let mut journal = AclJournal::new();
        journal
            .try_grant_object_dacl(&parent, profile.sid(), access)
            .expect("grant");
        assert!(
            acl_has_allow_sid(&parent, profile.sid(), false),
            "parent is missing the package ACE"
        );
        assert!(
            !acl_has_allow_sid(&child, profile.sid(), false),
            "child inherited the package ACE"
        );
        drop(journal);
        assert!(
            !acl_has_allow_sid(&parent, profile.sid(), false),
            "package ACE survived restore"
        );
    }

    #[test]
    fn traverse_ancestors_stop_above_the_drive_root() {
        let ancestors = super::traverse_ancestors(Path::new(r"C:\Users\me\AppData\Local\Temp\job"));
        assert_eq!(
            ancestors.first().map(PathBuf::as_path),
            Some(Path::new(r"C:\Users\me\AppData\Local\Temp"))
        );
        assert_eq!(
            ancestors.last().map(PathBuf::as_path),
            Some(Path::new(r"C:\Users"))
        );
        assert!(ancestors.iter().all(|path| path.parent().is_some()));
    }

    #[test]
    fn product_root_is_the_first_directory_under_program_files() {
        let roots = [PathBuf::from(r"C:\Program Files")];
        let product = product_root_under(&roots, Path::new(r"C:\Program Files\Git\cmd\git.exe"))
            .expect("product");
        assert_eq!(product, PathBuf::from(r"C:\Program Files\Git"));
        assert!(product_root_under(&roots, Path::new(r"C:\Python312\python.exe")).is_none());
    }

    #[test]
    fn runtime_root_keeps_an_inheritable_ace_on_the_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("note.txt"), "x").expect("write");
        let mut journal = AclJournal::new();
        journal
            .grant_runtime_root(dir.path(), ptr::null_mut())
            .expect("grant");
        journal
            .grant_runtime_root(dir.path(), ptr::null_mut())
            .expect("grant again");
        let sid = LocalSid::from_sddl(ALL_APPLICATION_PACKAGES_SDDL).expect("sid");
        assert!(acl_has_allow_sid(dir.path(), sid.as_ptr(), true));
        assert!(acl_has_allow_sid(
            &dir.path().join("note.txt"),
            sid.as_ptr(),
            false
        ));
    }

    #[test]
    fn protected_store_is_detected() {
        assert!(is_protected_tree(Path::new(
            r"C:\work\.kuibysheff\protected\agents\demo"
        )));
        assert!(!is_protected_tree(Path::new(r"C:\work\src\main.rs")));
    }
}

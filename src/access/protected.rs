//! Hard-deny for the agent protected store (settings readable only by the agent binary).

use std::path::{Path, PathBuf};

use crate::project_paths::{
    is_protected_path, path_contains_protected_segment, protected_root, KUIBYSHEFF_DIR,
    PROTECTED_DIR,
};

/// Reason string used in tool / policy denials.
pub const PROTECTED_DENY_REASON: &str =
    "path is inside the agent protected store (readable only by agent_Kuibysheff)";

/// Returns true when `canonical_path` must be denied to tools / sandboxed children.
#[must_use]
pub fn is_denied_protected_path(project_root: Option<&Path>, canonical_path: &Path) -> bool {
    let Some(root) = project_root else {
        // Without a project root, still deny paths that lexically contain
        // `.kuibysheff/protected` as a segment pair.
        return path_contains_protected_segment(canonical_path);
    };
    is_protected_path(root, canonical_path)
}

/// Deny if `workspace_root` itself is under protected, or a read grant names `protected`.
///
/// # Errors
///
/// Returns a validation message when the workspace would expose the protected store.
pub fn validate_workspace_excludes_protected(
    project_root: &Path,
    workspace_root: &Path,
    read_grants: &[String],
) -> Result<(), String> {
    if is_protected_path(project_root, workspace_root) {
        return Err(format!(
            "access.filesystem.workspace.root must not be under `{}`",
            protected_root(project_root).display()
        ));
    }
    for grant in read_grants {
        let trimmed = grant.trim();
        if trimmed.is_empty() || trimmed == "." {
            // Whole-root grant: ensure workspace root is not an ancestor of protected
            // that would include it. If workspace is project root, `.` would include
            // `.kuibysheff/protected` — reject that combination.
            let protected = protected_root(project_root);
            if protected.starts_with(workspace_root) || path_is_prefix(workspace_root, &protected) {
                return Err(
                    "access.filesystem.workspace.read grant covering the project root would \
                     expose `.kuibysheff/protected`; narrow read grants or set workspace.root \
                     outside the project root"
                        .to_string(),
                );
            }
            continue;
        }
        if trimmed == PROTECTED_DIR
            || trimmed.starts_with("protected/")
            || trimmed.starts_with(".kuibysheff/protected")
            || trimmed.contains("/protected/")
        {
            return Err(format!(
                "access.filesystem.workspace.read must not grant protected store path `{trimmed}`"
            ));
        }
    }
    Ok(())
}

fn path_is_prefix(prefix: &Path, path: &Path) -> bool {
    path.strip_prefix(prefix).is_ok()
}

/// Best-effort OS permissions: owner-only access on Unix; restrictive DACL on Windows.
pub fn apply_protected_dir_acl(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(dir, perms)?;
    }
    #[cfg(windows)]
    {
        let _ = dir;
        // Best-effort: icacls to remove inheritance and grant only current user.
        // Failure is non-fatal for callers that log and continue.
        if let Ok(user) = std::env::var("USERNAME") {
            let dir_s = dir.to_string_lossy();
            let _ = std::process::Command::new("icacls")
                .args([
                    dir_s.as_ref(),
                    "/inheritance:r",
                    "/grant:r",
                    &format!("{user}:(OI)(CI)F"),
                ])
                .output();
        }
    }
    let _ = dir;
    Ok(())
}

/// Ensure parent chain for a protected profile exists with restrictive ACLs.
pub fn ensure_protected_profile_dirs(profile_dir: &Path) -> std::io::Result<()> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut cur = profile_dir.to_path_buf();
    chain.push(cur.clone());
    while let Some(parent) = cur.parent() {
        if parent.as_os_str().is_empty() {
            break;
        }
        chain.push(parent.to_path_buf());
        // Stop once we created/seen `.kuibysheff`
        if parent.file_name().is_some_and(|n| n == KUIBYSHEFF_DIR) {
            break;
        }
        cur = parent.to_path_buf();
    }
    chain.reverse();
    for dir in chain {
        std::fs::create_dir_all(&dir)?;
        if dir.components().any(|c| c.as_os_str() == PROTECTED_DIR) {
            let _ = apply_protected_dir_acl(&dir);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_protected_under_project() {
        let root = Path::new("proj");
        let inside = protected_root(root).join("agents/a/x");
        assert!(is_denied_protected_path(Some(root), &inside));
        assert!(!is_denied_protected_path(
            Some(root),
            &root.join(".kuibysheff/homes/a")
        ));
    }

    #[test]
    fn rejects_workspace_root_grant_over_project() {
        let root = PathBuf::from("proj");
        let err = validate_workspace_excludes_protected(&root, &root, &[".".to_string()])
            .expect_err("should reject");
        assert!(err.contains("protected"), "{err}");
    }
}

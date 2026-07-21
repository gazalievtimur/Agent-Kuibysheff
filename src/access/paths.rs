//! Shared path normalization and grant checks (component-based, not string prefixes).

use std::path::{Component, Path, PathBuf};

use super::{
    AccessMode, CanonicalRoot, PathOperation, ProgramAlias, RelativeGrant, ResolvedAccessPolicy,
    ResolvedProgramPolicy,
};

/// Whether a canonical path stays inside a canonical root (component/`strip_prefix` based).
#[must_use]
pub fn is_within_root(root: &Path, candidate: &Path) -> bool {
    candidate.strip_prefix(root).is_ok()
}

/// Relativizes `candidate` against `root` after both are expected to be canonical.
///
/// # Errors
///
/// Returns a reason when `candidate` escapes `root`.
pub fn strip_root<'a>(root: &Path, candidate: &'a Path) -> Result<&'a Path, String> {
    candidate.strip_prefix(root).map_err(|_| {
        format!(
            "path `{}` escapes root `{}`",
            candidate.display(),
            root.display()
        )
    })
}

/// Parses a relative path into normalized components (no `..`, no absolute).
///
/// # Errors
///
/// Returns a reason when the path is absolute or contains `..`.
pub fn relative_components(path: &Path) -> Result<Vec<String>, String> {
    if path.as_os_str().is_empty() || path == Path::new(".") {
        return Ok(Vec::new());
    }
    if path.is_absolute() {
        return Err(format!("path `{}` must be relative", path.display()));
    }

    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.is_empty() {
                    return Err(format!(
                        "path `{}` contains an empty segment",
                        path.display()
                    ));
                }
                components.push(text.into_owned());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path `{}` must not contain `..`", path.display()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path `{}` must be relative", path.display()));
            }
        }
    }
    Ok(components)
}

/// Returns true when `relative` is covered by `grant` (exact prefix by components).
///
/// Empty grant components mean the root itself (covers everything under the root).
#[must_use]
pub fn grant_covers(grant: &RelativeGrant, relative: &[String]) -> bool {
    let grant_parts = grant.components();
    if grant_parts.is_empty() {
        return true;
    }
    if relative.len() < grant_parts.len() {
        return false;
    }
    relative[..grant_parts.len()] == *grant_parts
}

/// Returns true when any grant covers the relative components.
#[must_use]
pub fn any_grant_covers(grants: &[RelativeGrant], relative: &[String]) -> bool {
    grants.iter().any(|grant| grant_covers(grant, relative))
}

/// Filesystem grant scope for home or workspace tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGrantScope {
    mode: AccessMode,
    grants: Vec<RelativeGrant>,
}

impl PathGrantScope {
    #[must_use]
    pub fn legacy() -> Self {
        Self {
            mode: AccessMode::Legacy,
            grants: Vec::new(),
        }
    }

    #[must_use]
    pub fn strict(grants: Vec<RelativeGrant>) -> Self {
        Self {
            mode: AccessMode::Strict,
            grants,
        }
    }

    #[must_use]
    pub fn deny_all() -> Self {
        Self::strict(Vec::new())
    }

    #[must_use]
    pub fn is_legacy(&self) -> bool {
        self.mode == AccessMode::Legacy
    }

    #[must_use]
    pub fn grants(&self) -> &[RelativeGrant] {
        &self.grants
    }

    /// Checks whether a relative path is allowed for `operation`.
    ///
    /// # Errors
    ///
    /// Returns a reason when the path is malformed or outside the grant set.
    pub fn allows_relative(&self, relative: &Path, operation: PathOperation) -> Result<(), String> {
        let components = relative_components(relative)?;
        if self.is_legacy() {
            return Ok(());
        }
        if any_grant_covers(&self.grants, &components) {
            Ok(())
        } else {
            Err(format!(
                "{operation} denied for `{}` by access policy",
                display_relative(relative)
            ))
        }
    }
}

fn display_relative(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        ".".to_string()
    } else {
        path.display().to_string()
    }
}

/// Home tool path + run program policy derived from [`ResolvedAccessPolicy`].
#[derive(Debug, Clone)]
pub struct HomeFsPolicy {
    pub read: PathGrantScope,
    pub write: PathGrantScope,
    pub programs: std::collections::BTreeMap<ProgramAlias, ResolvedProgramPolicy>,
    pub max_args: usize,
    pub max_arg_chars: usize,
    pub max_output_chars: usize,
    pub max_timeout_ms: u64,
}

impl HomeFsPolicy {
    #[must_use]
    pub fn from_access(policy: &ResolvedAccessPolicy) -> Self {
        if policy.is_legacy() {
            Self::legacy_defaults(policy)
        } else {
            Self {
                read: PathGrantScope::strict(policy.home_read().to_vec()),
                write: PathGrantScope::strict(policy.home_write().to_vec()),
                programs: policy.programs().clone(),
                max_args: policy.max_args(),
                max_arg_chars: policy.max_arg_chars(),
                max_output_chars: policy.max_output_chars(),
                max_timeout_ms: policy.max_timeout_ms(),
            }
        }
    }

    #[must_use]
    pub fn legacy() -> Self {
        Self::legacy_defaults(&ResolvedAccessPolicy::legacy())
    }

    fn legacy_defaults(policy: &ResolvedAccessPolicy) -> Self {
        Self {
            read: PathGrantScope::legacy(),
            write: PathGrantScope::legacy(),
            programs: policy.programs().clone(),
            max_args: policy.max_args(),
            max_arg_chars: policy.max_arg_chars(),
            max_output_chars: policy.max_output_chars(),
            max_timeout_ms: policy.max_timeout_ms(),
        }
    }
}

/// Workspace research-tool path policy.
#[derive(Debug, Clone)]
pub struct WorkspaceFsPolicy {
    pub read: PathGrantScope,
}

impl WorkspaceFsPolicy {
    #[must_use]
    pub fn from_access(policy: &ResolvedAccessPolicy) -> Self {
        if policy.is_legacy() {
            return Self {
                read: PathGrantScope::legacy(),
            };
        }
        match policy.workspace() {
            Some(workspace) => Self {
                read: PathGrantScope::strict(workspace.read.clone()),
            },
            None => Self {
                read: PathGrantScope::deny_all(),
            },
        }
    }

    #[must_use]
    pub fn legacy() -> Self {
        Self {
            read: PathGrantScope::legacy(),
        }
    }
}

/// `--files` input root policy.
#[derive(Debug, Clone)]
pub struct InputFilesPolicy {
    unrestricted: bool,
    roots: Vec<PathBuf>,
}

impl InputFilesPolicy {
    #[must_use]
    pub fn from_access(policy: &ResolvedAccessPolicy) -> Self {
        if policy.is_legacy() {
            Self {
                unrestricted: true,
                roots: Vec::new(),
            }
        } else {
            Self {
                unrestricted: false,
                roots: policy
                    .input_roots()
                    .iter()
                    .map(|root| root.as_path().to_path_buf())
                    .collect(),
            }
        }
    }

    #[must_use]
    pub fn legacy() -> Self {
        Self {
            unrestricted: true,
            roots: Vec::new(),
        }
    }

    /// Strict mode limited to the given canonical roots.
    #[must_use]
    pub fn strict(roots: Vec<PathBuf>) -> Self {
        Self {
            unrestricted: false,
            roots,
        }
    }

    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.unrestricted
    }

    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Ensures a canonical file path is inside at least one configured input root.
    ///
    /// # Errors
    ///
    /// Returns a reason when the file is outside all roots in strict mode.
    pub fn allows_canonical_file(&self, canonical: &Path) -> Result<(), String> {
        if self.unrestricted {
            return Ok(());
        }
        if self
            .roots
            .iter()
            .any(|root| is_within_root(root, canonical))
        {
            Ok(())
        } else {
            Err(format!(
                "input file `{}` is outside configured `access.filesystem.input_roots`",
                canonical.display()
            ))
        }
    }
}

/// Resolves workspace host root for local tools.
#[must_use]
pub fn workspace_root_for_run(access: &ResolvedAccessPolicy, current_dir: &Path) -> PathBuf {
    if let Some(workspace) = access.workspace() {
        workspace.root.as_path().to_path_buf()
    } else {
        current_dir.to_path_buf()
    }
}

/// Helper used by tests and diagnostics.
#[must_use]
pub fn canonical_roots(roots: &[CanonicalRoot]) -> Vec<&Path> {
    roots.iter().map(CanonicalRoot::as_path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_prefix_out_does_not_match_outside() {
        let grant = RelativeGrant::parse("out").unwrap();
        assert!(grant_covers(&grant, &["out".to_string(), "a".to_string()]));
        assert!(!grant_covers(&grant, &["outside".to_string()]));
        assert!(!grant_covers(&grant, &["ou".to_string()]));
    }

    #[test]
    fn relative_components_reject_parent_and_absolute() {
        assert!(relative_components(Path::new("..")).is_err());
        assert!(relative_components(Path::new("a/../b")).is_err());
        #[cfg(windows)]
        assert!(relative_components(Path::new(r"C:\a")).is_err());
        #[cfg(unix)]
        assert!(relative_components(Path::new("/a")).is_err());
    }

    #[test]
    fn strict_scope_denies_ungranted_path() {
        let scope = PathGrantScope::strict(vec![RelativeGrant::parse("out").unwrap()]);
        assert!(scope
            .allows_relative(Path::new("out/x.txt"), PathOperation::Write)
            .is_ok());
        assert!(scope
            .allows_relative(Path::new("outside/x.txt"), PathOperation::Write)
            .is_err());
    }
}

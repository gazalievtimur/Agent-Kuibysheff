//! Resolve agent paths under a project's `.kuibyshev` directory.

use std::path::{Path, PathBuf};

/// Directory name for per-project Kuibyshev settings and run homes.
pub const KUIBYSHEV_DIR: &str = ".kuibyshev";

/// When `project_root` is set, resolve a relative `path` against
/// `{project_root}/.kuibyshev/`. Absolute paths are returned unchanged.
#[must_use]
pub fn resolve_under_kuibyshev(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    project_root.join(KUIBYSHEV_DIR).join(path)
}

/// Resolve config / settings-dir / home for a worker or ACP turn.
#[must_use]
pub fn resolve_agent_paths(
    project_root: Option<&Path>,
    config: &Path,
    settings_dir: &Path,
    home: &Path,
) -> (PathBuf, PathBuf, PathBuf) {
    match project_root {
        Some(root) => (
            resolve_under_kuibyshev(root, config),
            resolve_under_kuibyshev(root, settings_dir),
            resolve_under_kuibyshev(root, home),
        ),
        None => (
            config.to_path_buf(),
            settings_dir.to_path_buf(),
            home.to_path_buf(),
        ),
    }
}

/// Prefer a non-empty session cwd, otherwise the CLI `--project-root`.
#[must_use]
pub fn effective_project_root(
    session_cwd: Option<&Path>,
    cli_project_root: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(cwd) = session_cwd {
        if !cwd.as_os_str().is_empty() {
            return Some(cwd.to_path_buf());
        }
    }
    cli_project_root.map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_unchanged() {
        let root = Path::new("/proj");
        #[cfg(windows)]
        let abs = PathBuf::from(r"C:\abs\config.yaml");
        #[cfg(not(windows))]
        let abs = PathBuf::from("/abs/config.yaml");
        assert_eq!(resolve_under_kuibyshev(root, &abs), abs);
    }

    #[test]
    fn relative_paths_join_kuibyshev() {
        let root = PathBuf::from("proj");
        let resolved = resolve_under_kuibyshev(&root, Path::new("agents/1c-analyst"));
        assert_eq!(resolved, root.join(KUIBYSHEV_DIR).join("agents/1c-analyst"));
    }

    #[test]
    fn effective_root_prefers_session_cwd() {
        let cwd = Path::new("/from-ide");
        let cli = Path::new("/from-cli");
        assert_eq!(
            effective_project_root(Some(cwd), Some(cli)).as_deref(),
            Some(Path::new("/from-ide"))
        );
        assert_eq!(
            effective_project_root(Some(Path::new("")), Some(cli)).as_deref(),
            Some(Path::new("/from-cli"))
        );
        assert_eq!(effective_project_root(None, None), None);
    }
}

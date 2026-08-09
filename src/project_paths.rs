//! Resolve agent paths under a project's `.kuibysheff` directory.
//!
//! Canonical layout (agent-owned; operators address `--agent` + `--project-root`):
//!
//! ```text
//! {project-root}/.kuibysheff/
//!   protected/agents/{agent-id}/   # settings — agent binary only
//!   homes/{agent-id}/
//!   mcp-runtime/{agent-id}/{server}/
//! ```

use std::path::{Component, Path, PathBuf};

/// Directory name for per-project Kuibysheff data.
pub const KUIBYSHEFF_DIR: &str = ".kuibysheff";

/// Subtree that only the agent binary may read/write.
pub const PROTECTED_DIR: &str = "protected";

/// Agent profiles live under `protected/agents/{id}/`.
pub const AGENTS_DIR: &str = "agents";

/// Default tool homes: `homes/{id}/`.
pub const HOMES_DIR: &str = "homes";

/// MCP child cwd/scratch: `mcp-runtime/{id}/{server}/`.
pub const MCP_RUNTIME_DIR: &str = "mcp-runtime";

/// Runtime config filename inside an agent profile.
pub const AGENT_CONFIG_FILE: &str = "agent-config.yaml";

/// Errors when resolving or validating agent identity / home paths.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentPathError {
    #[error("invalid agent id `{0}`: use `[a-z0-9][a-z0-9_-]*` without path separators")]
    InvalidAgentId(String),
    #[error("`--home` must be relative under `.kuibysheff/` and must not be under `protected/`")]
    InvalidHomePath,
    #[error("home path escapes `.kuibysheff/` or enters `protected/`: {0}")]
    HomeNotAllowed(String),
    #[error("invalid MCP server name for runtime dir `{0}`")]
    InvalidMcpServerName(String),
}

/// Validate `agent-id`: `[a-z0-9][a-z0-9_-]*`, no path separators.
///
/// # Errors
///
/// Returns [`AgentPathError::InvalidAgentId`] when the id is empty or malformed.
pub fn validate_agent_id(agent_id: &str) -> Result<(), AgentPathError> {
    let mut chars = agent_id.chars();
    let Some(first) = chars.next() else {
        return Err(AgentPathError::InvalidAgentId(agent_id.to_string()));
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(AgentPathError::InvalidAgentId(agent_id.to_string()));
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err(AgentPathError::InvalidAgentId(agent_id.to_string()));
    }
    if agent_id.contains('/') || agent_id.contains('\\') || agent_id.contains("..") {
        return Err(AgentPathError::InvalidAgentId(agent_id.to_string()));
    }
    Ok(())
}

/// `{project_root}/.kuibysheff`.
#[must_use]
pub fn kuibysheff_root(project_root: &Path) -> PathBuf {
    project_root.join(KUIBYSHEFF_DIR)
}

/// `{project_root}/.kuibysheff/protected`.
#[must_use]
pub fn protected_root(project_root: &Path) -> PathBuf {
    kuibysheff_root(project_root).join(PROTECTED_DIR)
}

/// `{project_root}/.kuibysheff/protected/agents`.
#[must_use]
pub fn protected_agents_root(project_root: &Path) -> PathBuf {
    protected_root(project_root).join(AGENTS_DIR)
}

/// Profile directory for `agent_id` (settings + config).
///
/// # Errors
///
/// Returns [`AgentPathError::InvalidAgentId`] when `agent_id` is invalid.
pub fn agent_profile_dir(project_root: &Path, agent_id: &str) -> Result<PathBuf, AgentPathError> {
    validate_agent_id(agent_id)?;
    Ok(protected_agents_root(project_root).join(agent_id))
}

/// Settings directory is the profile directory.
///
/// # Errors
///
/// Propagates [`agent_profile_dir`] errors.
#[allow(dead_code)]
pub fn agent_settings_dir(project_root: &Path, agent_id: &str) -> Result<PathBuf, AgentPathError> {
    agent_profile_dir(project_root, agent_id)
}

/// `{profile}/agent-config.yaml`.
///
/// # Errors
///
/// Propagates [`agent_profile_dir`] errors.
#[allow(dead_code)]
pub fn agent_config_path(project_root: &Path, agent_id: &str) -> Result<PathBuf, AgentPathError> {
    Ok(agent_profile_dir(project_root, agent_id)?.join(AGENT_CONFIG_FILE))
}

/// Default home: `{project}/.kuibysheff/homes/{agent_id}`.
///
/// # Errors
///
/// Propagates invalid agent id.
pub fn agent_home_dir(project_root: &Path, agent_id: &str) -> Result<PathBuf, AgentPathError> {
    validate_agent_id(agent_id)?;
    Ok(kuibysheff_root(project_root).join(HOMES_DIR).join(agent_id))
}

/// MCP scratch/cwd: `{project}/.kuibysheff/mcp-runtime/{agent_id}/{server}`.
///
/// # Errors
///
/// Propagates invalid agent id. `server` must be a single path segment (no separators).
pub fn mcp_runtime_dir(
    project_root: &Path,
    agent_id: &str,
    server: &str,
) -> Result<PathBuf, AgentPathError> {
    validate_agent_id(agent_id)?;
    if server.is_empty()
        || server.contains('/')
        || server.contains('\\')
        || server.contains("..")
        || Path::new(server).components().count() != 1
    {
        return Err(AgentPathError::InvalidMcpServerName(server.to_string()));
    }
    Ok(kuibysheff_root(project_root)
        .join(MCP_RUNTIME_DIR)
        .join(agent_id)
        .join(server))
}

/// Resolved paths for one agent under a project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentPaths {
    pub project_root: PathBuf,
    pub agent_id: String,
    pub profile_dir: PathBuf,
    pub config: PathBuf,
    pub settings_dir: PathBuf,
    pub home: PathBuf,
}

/// Resolve canonical agent paths. Optional `home_override` must be relative under
/// `.kuibysheff/` and must not enter `protected/`. Absolute home is rejected.
///
/// # Errors
///
/// Invalid agent id or disallowed home override.
pub fn resolve_agent_identity(
    project_root: &Path,
    agent_id: &str,
    home_override: Option<&Path>,
) -> Result<ResolvedAgentPaths, AgentPathError> {
    validate_agent_id(agent_id)?;
    let profile_dir = agent_profile_dir(project_root, agent_id)?;
    let config = profile_dir.join(AGENT_CONFIG_FILE);
    let settings_dir = profile_dir.clone();
    let home = match home_override {
        None => agent_home_dir(project_root, agent_id)?,
        Some(raw) => resolve_home_override(project_root, raw)?,
    };
    Ok(ResolvedAgentPaths {
        project_root: project_root.to_path_buf(),
        agent_id: agent_id.to_string(),
        profile_dir,
        config,
        settings_dir,
        home,
    })
}

fn resolve_home_override(project_root: &Path, home: &Path) -> Result<PathBuf, AgentPathError> {
    if home.is_absolute() {
        return Err(AgentPathError::InvalidHomePath);
    }
    if path_has_parent_component(home) {
        return Err(AgentPathError::InvalidHomePath);
    }
    let first = home.components().next();
    if matches!(first, Some(Component::Normal(c)) if c == PROTECTED_DIR) {
        return Err(AgentPathError::HomeNotAllowed(home.display().to_string()));
    }
    // Relative home is always under `.kuibysheff/`.
    let resolved = kuibysheff_root(project_root).join(home);
    if is_under_protected_root(project_root, &resolved) {
        return Err(AgentPathError::HomeNotAllowed(
            resolved.display().to_string(),
        ));
    }
    Ok(resolved)
}

fn path_has_parent_component(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
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

/// Whether `path` is under `{project}/.kuibysheff/protected/` (lexical check).
///
/// Callers should pass canonical paths when available; this also rejects paths that
/// lexically contain `protected` under kuibysheff even before canonicalize.
#[must_use]
pub fn is_protected_path(project_root: &Path, path: &Path) -> bool {
    is_under_protected_root(project_root, path)
}

fn is_under_protected_root(project_root: &Path, path: &Path) -> bool {
    let protected = protected_root(project_root);
    path_is_within(&protected, path)
}

/// Lexical "within root" check (also true when `path == root`).
#[must_use]
pub fn path_is_within(root: &Path, path: &Path) -> bool {
    let root_c = normalize_lexically(root);
    let path_c = normalize_lexically(path);
    path_c.starts_with(&root_c)
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// When `project_root` is set, resolve a relative `path` against
/// `{project_root}/.kuibysheff/`. Absolute paths are returned unchanged.
///
/// Prefer [`resolve_agent_identity`] for agent config/settings/home.
#[must_use]
#[allow(dead_code)]
pub fn resolve_under_kuibysheff(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    project_root.join(KUIBYSHEFF_DIR).join(path)
}

/// Resolve the config file path used to locate a config-directory `.env`.
#[must_use]
pub fn resolve_config_path_for_dotenv(config: &Path, launch_cwd: &Path) -> PathBuf {
    if config.is_absolute() {
        config.to_path_buf()
    } else {
        launch_cwd.join(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_layout() {
        let root = Path::new("proj");
        let profile = agent_profile_dir(root, "demo").expect("id");
        assert_eq!(
            profile,
            root.join(KUIBYSHEFF_DIR)
                .join(PROTECTED_DIR)
                .join(AGENTS_DIR)
                .join("demo")
        );
        assert_eq!(
            agent_config_path(root, "demo").expect("cfg"),
            profile.join(AGENT_CONFIG_FILE)
        );
        assert_eq!(
            agent_home_dir(root, "demo").expect("home"),
            root.join(KUIBYSHEFF_DIR).join(HOMES_DIR).join("demo")
        );
        assert_eq!(
            mcp_runtime_dir(root, "demo", "docs").expect("mcp"),
            root.join(KUIBYSHEFF_DIR)
                .join(MCP_RUNTIME_DIR)
                .join("demo")
                .join("docs")
        );
    }

    #[test]
    fn rejects_bad_agent_id() {
        assert!(validate_agent_id("").is_err());
        assert!(validate_agent_id("../x").is_err());
        assert!(validate_agent_id("A").is_err());
        assert!(validate_agent_id("ok-1").is_ok());
    }

    #[test]
    fn home_override_rules() {
        let root = Path::new("proj");
        let resolved = resolve_agent_identity(root, "a", Some(Path::new("homes/custom"))).unwrap();
        assert_eq!(
            resolved.home,
            root.join(KUIBYSHEFF_DIR).join("homes/custom")
        );
        #[cfg(windows)]
        assert!(resolve_agent_identity(root, "a", Some(Path::new(r"C:\abs"))).is_err());
        #[cfg(not(windows))]
        assert!(resolve_agent_identity(root, "a", Some(Path::new("/abs"))).is_err());
        assert!(resolve_agent_identity(root, "a", Some(Path::new("protected/x"))).is_err());
        assert!(resolve_agent_identity(root, "a", Some(Path::new("../outside"))).is_err());
    }

    #[test]
    fn protected_path_detection() {
        let root = Path::new("proj");
        let inside = protected_agents_root(root)
            .join("a")
            .join("agent-config.yaml");
        assert!(is_protected_path(root, &inside));
        let home = agent_home_dir(root, "a").unwrap();
        assert!(!is_protected_path(root, &home));
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

    #[test]
    fn dotenv_joins_launch_cwd_for_relative() {
        let launch = Path::new("launch");
        assert_eq!(
            resolve_config_path_for_dotenv(Path::new("a.yaml"), launch),
            PathBuf::from("launch").join("a.yaml")
        );
    }
}

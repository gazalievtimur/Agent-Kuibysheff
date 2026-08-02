//! Access policy compilation and fail-closed runtime types.
//!
//! Raw YAML maps to [`config::AccessPolicyConfig`]. After startup resolution the
//! run uses only [`ResolvedAccessPolicy`] — never raw string paths without normalization.
//!
//! This module owns both raw DTOs and resolve so it does not depend on [`crate::config`].

pub mod config;
pub mod paths;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::tools::registry;

pub use config::{
    AccessPolicyConfig, FilesystemPolicyConfig, HomeFsPolicyConfig, ProgramPolicyConfig,
    RunPolicyConfig, ToolsPolicyConfig, WorkspacePolicyConfig,
};
pub use paths::{
    workspace_root_for_run, HomeFsPolicy, InputFilesPolicy, PathGrantScope, WorkspaceFsPolicy,
};

/// Errors while validating or resolving an access policy section.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessError {
    #[error("{0}")]
    Validation(String),
}

/// Built-in tools advertised by the agent (qualified `server.tool` names).
///
/// Source of truth: [`crate::tools::registry::BUILTINS`].
pub fn known_builtins() -> impl Iterator<Item = &'static str> {
    registry::known_builtin_names()
}

/// Built-ins available in legacy mode (`access` omitted). `home.run` requires an explicit
/// sandbox profile under `access.run`.
pub fn legacy_builtins() -> impl Iterator<Item = &'static str> {
    registry::legacy_builtin_names()
}

/// Environment keys that must never be inherited into sandboxed `home.run` processes.
const FORBIDDEN_INHERIT_ENV: &[&str] = &[
    "LD_PRELOAD",
    "LD_AUDIT",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FORCE_FLAT_NAMESPACE",
];

/// Whether the policy was compiled from an explicit `access` section or legacy defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// No `access` section: preserve historical filesystem behavior; hide `home.run`.
    Legacy,
    /// Explicit `access`: everything not listed is denied.
    Strict,
}

/// Path capability checked by filesystem tools and the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathOperation {
    Read,
    Write,
    Execute,
}

impl fmt::Display for PathOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => write!(f, "read"),
            Self::Write => write!(f, "write"),
            Self::Execute => write!(f, "execute"),
        }
    }
}

/// Qualified tool identity parsed only from `server.tool` (bare names are rejected).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QualifiedTool {
    server: String,
    tool: String,
}

impl QualifiedTool {
    /// Creates a validated qualified tool from raw server and tool segments.
    ///
    /// # Errors
    ///
    /// Returns a reason when either segment is empty or contains `.`.
    pub fn new(server: impl Into<String>, tool: impl Into<String>) -> Result<Self, String> {
        let server = server.into();
        let tool = tool.into();
        if server.is_empty() || tool.is_empty() {
            return Err(
                "qualified tool name must use non-empty `server.tool` segments".to_string(),
            );
        }
        if server.contains('.') || tool.contains('.') {
            return Err("qualified tool name segments must not contain `.`".to_string());
        }
        Ok(Self { server, tool })
    }

    /// Parses `server.tool`. Bare names and empty segments are rejected.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason when the name is not a single `server.tool` pair.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        let Some((server, tool)) = trimmed.split_once('.') else {
            return Err(format!(
                "bare tool name `{trimmed}` is not allowed; use qualified `server.tool`"
            ));
        };
        Self::new(server, tool)
            .map_err(|reason| format!("tool name `{trimmed}` is invalid: {reason}"))
    }

    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }

    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    #[must_use]
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.server, self.tool)
    }
}

impl fmt::Display for QualifiedTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.qualified())
    }
}

/// Normalized relative prefix grant (no `..`, no absolute path).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelativeGrant {
    components: Vec<String>,
}

impl RelativeGrant {
    /// Parses a relative grant path into normalized components.
    ///
    /// `#`, `.`, and empty string mean the grant root itself.
    ///
    /// # Errors
    ///
    /// Returns a reason when the path is absolute, contains `..`, or has empty segments.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "." {
            return Ok(Self {
                components: Vec::new(),
            });
        }

        let path = Path::new(trimmed);
        if path.is_absolute() {
            return Err(format!("grant `{trimmed}` must be a relative path"));
        }

        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => {
                    let text = part.to_string_lossy();
                    if text.is_empty() {
                        return Err(format!("grant `{trimmed}` contains an empty segment"));
                    }
                    components.push(text.into_owned());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(format!("grant `{trimmed}` must not contain `..`"));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(format!("grant `{trimmed}` must be a relative path"));
                }
            }
        }

        Ok(Self { components })
    }

    #[must_use]
    pub fn components(&self) -> &[String] {
        &self.components
    }

    #[must_use]
    pub fn as_path(&self) -> PathBuf {
        if self.components.is_empty() {
            PathBuf::from(".")
        } else {
            self.components.iter().collect()
        }
    }
}

/// Canonicalized host filesystem root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalRoot {
    path: PathBuf,
}

impl CanonicalRoot {
    /// Canonicalizes an existing path.
    ///
    /// # Errors
    ///
    /// Returns [`AccessError::Validation`] when the path cannot be canonicalized.
    pub fn canonicalize(path: &Path) -> Result<Self, AccessError> {
        let canonical = fs::canonicalize(path).map_err(|source| {
            AccessError::Validation(format!(
                "failed to canonicalize `{}`: {source}",
                path.display()
            ))
        })?;
        Ok(Self { path: canonical })
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

/// Stable program alias used by `home.run.program` (not a host path).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramAlias(String);

impl ProgramAlias {
    /// Creates an alias from a non-empty name.
    ///
    /// # Errors
    ///
    /// Returns a reason when the name is empty or whitespace-only.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("program alias must not be empty".to_string());
        }
        Ok(Self(trimmed.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProgramAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Resolved executable identity and sandbox inputs for one program alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProgramPolicy {
    pub alias: ProgramAlias,
    pub executable: CanonicalRoot,
    pub runtime_read_roots: Vec<CanonicalRoot>,
    pub inherit_env: Vec<String>,
    pub allow_children: bool,
}

/// Workspace read scope under a canonical host root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspacePolicy {
    pub root: CanonicalRoot,
    pub read: Vec<RelativeGrant>,
}

/// Immutable access policy for a single agent run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAccessPolicy {
    mode: AccessMode,
    allowed_builtins: BTreeSet<QualifiedTool>,
    home_read: Vec<RelativeGrant>,
    home_write: Vec<RelativeGrant>,
    workspace: Option<ResolvedWorkspacePolicy>,
    input_roots: Vec<CanonicalRoot>,
    programs: BTreeMap<ProgramAlias, ResolvedProgramPolicy>,
    max_args: usize,
    max_arg_chars: usize,
    max_output_chars: usize,
    max_timeout_ms: u64,
}

impl ResolvedAccessPolicy {
    #[must_use]
    pub fn mode(&self) -> AccessMode {
        self.mode
    }

    #[must_use]
    pub fn is_legacy(&self) -> bool {
        self.mode == AccessMode::Legacy
    }

    #[must_use]
    pub fn allowed_builtins(&self) -> &BTreeSet<QualifiedTool> {
        &self.allowed_builtins
    }

    #[must_use]
    pub fn allows_builtin(&self, tool: &QualifiedTool) -> bool {
        self.allowed_builtins.contains(tool)
    }

    #[must_use]
    pub fn home_read(&self) -> &[RelativeGrant] {
        &self.home_read
    }

    #[must_use]
    pub fn home_write(&self) -> &[RelativeGrant] {
        &self.home_write
    }

    #[must_use]
    pub fn workspace(&self) -> Option<&ResolvedWorkspacePolicy> {
        self.workspace.as_ref()
    }

    #[must_use]
    pub fn input_roots(&self) -> &[CanonicalRoot] {
        &self.input_roots
    }

    #[must_use]
    pub fn programs(&self) -> &BTreeMap<ProgramAlias, ResolvedProgramPolicy> {
        &self.programs
    }

    #[must_use]
    pub fn max_args(&self) -> usize {
        self.max_args
    }

    #[must_use]
    pub fn max_arg_chars(&self) -> usize {
        self.max_arg_chars
    }

    #[must_use]
    pub fn max_output_chars(&self) -> usize {
        self.max_output_chars
    }

    #[must_use]
    pub fn max_timeout_ms(&self) -> u64 {
        self.max_timeout_ms
    }

    /// Compiles legacy defaults when `access` is omitted.
    #[must_use]
    pub fn legacy() -> Self {
        Self {
            mode: AccessMode::Legacy,
            allowed_builtins: parse_known_builtins(legacy_builtins()),
            home_read: Vec::new(),
            home_write: Vec::new(),
            workspace: None,
            input_roots: Vec::new(),
            programs: BTreeMap::new(),
            max_args: RunPolicyConfig::default_max_args(),
            max_arg_chars: RunPolicyConfig::default_max_arg_chars(),
            max_output_chars: RunPolicyConfig::default_max_output_chars(),
            max_timeout_ms: RunPolicyConfig::default_max_timeout_ms(),
        }
    }
}

/// Effective tool allowlist for one run: gated builtins ∪ unconditionally trusted MCP tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveToolPolicy {
    tools: BTreeSet<QualifiedTool>,
}

impl EffectiveToolPolicy {
    /// Compiles the runtime tool allowlist.
    ///
    /// Built-ins: registry known set ∩ `access.tools.builtins` ∩ `skills.allowed_tools`.
    /// MCP tools: `discovered_mcp_tools ∩ skills.allowed_tools`.
    #[must_use]
    pub fn compile(
        access: &ResolvedAccessPolicy,
        skills_allowed: &BTreeSet<QualifiedTool>,
        mcp_tools: impl IntoIterator<Item = QualifiedTool>,
    ) -> Self {
        let known = parse_known_builtins(known_builtins());
        let mut tools = BTreeSet::new();
        for tool in &known {
            if access.allows_builtin(tool) && skills_allowed.contains(tool) {
                tools.insert(tool.clone());
            }
        }
        for tool in mcp_tools {
            if skills_allowed.contains(&tool) {
                tools.insert(tool);
            }
        }
        Self { tools }
    }

    #[must_use]
    pub fn allows(&self, tool: &QualifiedTool) -> bool {
        self.tools.contains(tool)
    }

    #[must_use]
    pub fn allows_server_tool(&self, server: &str, tool: &str) -> bool {
        QualifiedTool::new(server, tool)
            .map(|qualified| self.tools.contains(&qualified))
            .unwrap_or(false)
    }

    /// Sorted qualified names for prompt advertisement.
    #[must_use]
    pub fn advertised(&self) -> Vec<String> {
        self.tools.iter().map(QualifiedTool::qualified).collect()
    }

    #[must_use]
    pub fn tools(&self) -> &BTreeSet<QualifiedTool> {
        &self.tools
    }
}

/// Parses MCP/`available_tools` entries into qualified tools.
///
/// # Errors
///
/// Returns a reason when an entry is not a valid `server.tool` name.
pub fn parse_tool_list(
    entries: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<BTreeSet<QualifiedTool>, String> {
    let mut tools = BTreeSet::new();
    for entry in entries {
        tools.insert(QualifiedTool::parse(entry.as_ref())?);
    }
    Ok(tools)
}

/// Borrowed inputs for compiling a raw access policy into [`ResolvedAccessPolicy`].
#[derive(Debug, Clone, Copy)]
pub struct AccessResolveInput<'a> {
    pub access: Option<&'a AccessPolicyConfig>,
    pub config_dir: &'a Path,
}

impl TryFrom<AccessResolveInput<'_>> for ResolvedAccessPolicy {
    type Error = AccessError;

    fn try_from(input: AccessResolveInput<'_>) -> Result<Self, Self::Error> {
        resolve_access_policy(input.access, input.config_dir)
    }
}

/// Resolves optional raw access config into an immutable policy.
///
/// Host paths (`workspace.root`, `input_roots`, executables, runtime roots) are resolved
/// relative to `config_dir` and canonicalized. Home grants stay relative prefixes.
///
/// Prefer [`ResolvedAccessPolicy::try_from`] with [`AccessResolveInput`] at call sites that
/// already hold borrowed DTOs.
///
/// # Errors
///
/// Returns [`AccessError::Validation`] for invalid grants, missing host roots, or bad programs.
pub fn resolve_access_policy(
    access: Option<&AccessPolicyConfig>,
    config_dir: &Path,
) -> Result<ResolvedAccessPolicy, AccessError> {
    let Some(access) = access else {
        return Ok(ResolvedAccessPolicy::legacy());
    };

    let allowed_builtins = resolve_builtins(&access.tools)?;
    let home_read = resolve_relative_grants(&access.filesystem.home.read, "filesystem.home.read")?;
    let home_write =
        resolve_relative_grants(&access.filesystem.home.write, "filesystem.home.write")?;
    let workspace = resolve_workspace(&access.filesystem, config_dir)?;
    let input_roots = resolve_input_roots(&access.filesystem.input_roots, config_dir)?;
    let resolved_run = resolve_run(&access.run, config_dir)?;

    Ok(ResolvedAccessPolicy {
        mode: AccessMode::Strict,
        allowed_builtins,
        home_read,
        home_write,
        workspace,
        input_roots,
        programs: resolved_run.programs,
        max_args: resolved_run.max_args,
        max_arg_chars: resolved_run.max_arg_chars,
        max_output_chars: resolved_run.max_output_chars,
        max_timeout_ms: resolved_run.max_timeout_ms,
    })
}

/// Validates access-related fields that do not require filesystem I/O.
///
/// # Errors
///
/// Returns [`AccessError::Validation`] for duplicate aliases, unknown builtins, forbidden env,
/// reserved MCP names, or zero limits.
pub fn validate_access_config(
    access: Option<&AccessPolicyConfig>,
    mcp_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(), AccessError> {
    for name in mcp_names {
        let name = name.as_ref();
        if registry::is_builtin_server(name) {
            return Err(AccessError::Validation(format!(
                "mcp server name `{name}` is reserved for built-in tools"
            )));
        }
    }

    let Some(access) = access else {
        return Ok(());
    };

    validate_builtins_list(&access.tools.builtins)?;
    validate_relative_grant_list(&access.filesystem.home.read, "filesystem.home.read")?;
    validate_relative_grant_list(&access.filesystem.home.write, "filesystem.home.write")?;

    if let Some(workspace) = &access.filesystem.workspace {
        if workspace.root.as_os_str().is_empty() {
            return Err(AccessError::Validation(
                "`access.filesystem.workspace.root` must not be empty".to_string(),
            ));
        }
        validate_relative_grant_list(&workspace.read, "filesystem.workspace.read")?;
    }

    for root in &access.filesystem.input_roots {
        if root.as_os_str().is_empty() {
            return Err(AccessError::Validation(
                "`access.filesystem.input_roots` entries must not be empty".to_string(),
            ));
        }
    }

    validate_run_limits(&access.run)?;
    validate_program_aliases(&access.run.programs)?;

    Ok(())
}

fn validate_builtins_list(builtins: &[String]) -> Result<(), AccessError> {
    let mut seen = BTreeSet::new();
    for raw in builtins {
        let tool = QualifiedTool::parse(raw).map_err(|reason| {
            AccessError::Validation(format!("access.tools.builtins: {reason}"))
        })?;
        let qualified = tool.qualified();
        if !registry::is_known_builtin(&qualified) {
            return Err(AccessError::Validation(format!(
                "unknown built-in tool `{qualified}` in `access.tools.builtins`"
            )));
        }
        if !seen.insert(qualified.clone()) {
            return Err(AccessError::Validation(format!(
                "duplicate built-in tool `{qualified}` in `access.tools.builtins`"
            )));
        }
    }
    Ok(())
}

fn validate_relative_grant_list(grants: &[String], field: &str) -> Result<(), AccessError> {
    for raw in grants {
        RelativeGrant::parse(raw)
            .map_err(|reason| AccessError::Validation(format!("`access.{field}`: {reason}")))?;
    }
    Ok(())
}

fn validate_run_limits(run: &RunPolicyConfig) -> Result<(), AccessError> {
    if run.max_args == 0 {
        return Err(AccessError::Validation(
            "`access.run.max_args` must be > 0".to_string(),
        ));
    }
    if run.max_arg_chars == 0 {
        return Err(AccessError::Validation(
            "`access.run.max_arg_chars` must be > 0".to_string(),
        ));
    }
    if run.max_output_chars == 0 {
        return Err(AccessError::Validation(
            "`access.run.max_output_chars` must be > 0".to_string(),
        ));
    }
    if run.max_timeout_ms == 0 {
        return Err(AccessError::Validation(
            "`access.run.max_timeout_ms` must be > 0".to_string(),
        ));
    }
    Ok(())
}

fn validate_program_aliases(programs: &[ProgramPolicyConfig]) -> Result<(), AccessError> {
    let mut aliases = BTreeSet::new();
    for program in programs {
        let alias = ProgramAlias::parse(&program.name).map_err(|reason| {
            AccessError::Validation(format!("`access.run.programs[].name`: {reason}"))
        })?;
        if !aliases.insert(alias.as_str().to_string()) {
            return Err(AccessError::Validation(format!(
                "duplicate program alias `{alias}` in `access.run.programs`"
            )));
        }
        if program.executable.as_os_str().is_empty() {
            return Err(AccessError::Validation(format!(
                "`access.run.programs[{alias}].executable` must not be empty"
            )));
        }
        for key in &program.inherit_env {
            let normalized = key.trim();
            if normalized.is_empty() {
                return Err(AccessError::Validation(format!(
                    "`access.run.programs[{alias}].inherit_env` entries must not be empty"
                )));
            }
            if FORBIDDEN_INHERIT_ENV
                .iter()
                .any(|forbidden| normalized.eq_ignore_ascii_case(forbidden))
            {
                return Err(AccessError::Validation(format!(
                    "`access.run.programs[{alias}].inherit_env` must not include `{normalized}`"
                )));
            }
        }
    }
    Ok(())
}

fn resolve_builtins(tools: &ToolsPolicyConfig) -> Result<BTreeSet<QualifiedTool>, AccessError> {
    validate_builtins_list(&tools.builtins)?;
    let mut set = BTreeSet::new();
    for raw in &tools.builtins {
        set.insert(QualifiedTool::parse(raw).map_err(|reason| {
            AccessError::Validation(format!("access.tools.builtins: {reason}"))
        })?);
    }
    Ok(set)
}

fn resolve_relative_grants(
    grants: &[String],
    field: &str,
) -> Result<Vec<RelativeGrant>, AccessError> {
    grants
        .iter()
        .map(|raw| {
            RelativeGrant::parse(raw)
                .map_err(|reason| AccessError::Validation(format!("`access.{field}`: {reason}")))
        })
        .collect()
}

fn resolve_workspace(
    filesystem: &FilesystemPolicyConfig,
    config_dir: &Path,
) -> Result<Option<ResolvedWorkspacePolicy>, AccessError> {
    let Some(workspace) = &filesystem.workspace else {
        return Ok(None);
    };

    let root_path = resolve_against_config_dir(config_dir, &workspace.root);
    let root = CanonicalRoot::canonicalize(&root_path)?;
    if !root.as_path().is_dir() {
        return Err(AccessError::Validation(format!(
            "`access.filesystem.workspace.root` is not a directory: {}",
            root.as_path().display()
        )));
    }
    let read = resolve_relative_grants(&workspace.read, "filesystem.workspace.read")?;
    Ok(Some(ResolvedWorkspacePolicy { root, read }))
}

fn resolve_input_roots(
    roots: &[PathBuf],
    config_dir: &Path,
) -> Result<Vec<CanonicalRoot>, AccessError> {
    let mut resolved = Vec::with_capacity(roots.len());
    for root in roots {
        let path = resolve_against_config_dir(config_dir, root);
        let canonical = CanonicalRoot::canonicalize(&path)?;
        if !canonical.as_path().is_dir() {
            return Err(AccessError::Validation(format!(
                "`access.filesystem.input_roots` entry is not a directory: {}",
                canonical.as_path().display()
            )));
        }
        resolved.push(canonical);
    }
    Ok(resolved)
}

struct ResolvedRunLimits {
    programs: BTreeMap<ProgramAlias, ResolvedProgramPolicy>,
    max_args: usize,
    max_arg_chars: usize,
    max_output_chars: usize,
    max_timeout_ms: u64,
}

fn resolve_run(run: &RunPolicyConfig, config_dir: &Path) -> Result<ResolvedRunLimits, AccessError> {
    validate_run_limits(run)?;
    validate_program_aliases(&run.programs)?;

    let mut programs = BTreeMap::new();
    for program in &run.programs {
        let alias = ProgramAlias::parse(&program.name).map_err(|reason| {
            AccessError::Validation(format!("`access.run.programs[].name`: {reason}"))
        })?;
        let executable_path = resolve_against_config_dir(config_dir, &program.executable);
        let executable = CanonicalRoot::canonicalize(&executable_path)?;
        if !executable.as_path().is_file() {
            return Err(AccessError::Validation(format!(
                "`access.run.programs[{alias}].executable` is not a file: {}",
                executable.as_path().display()
            )));
        }

        let mut runtime_read_roots = Vec::with_capacity(program.runtime_read_roots.len());
        for root in &program.runtime_read_roots {
            let path = resolve_against_config_dir(config_dir, root);
            let canonical = CanonicalRoot::canonicalize(&path)?;
            if !canonical.as_path().exists() {
                return Err(AccessError::Validation(format!(
                    "`access.run.programs[{alias}].runtime_read_roots` missing: {}",
                    canonical.as_path().display()
                )));
            }
            runtime_read_roots.push(canonical);
        }

        programs.insert(
            alias.clone(),
            ResolvedProgramPolicy {
                alias,
                executable,
                runtime_read_roots,
                inherit_env: program
                    .inherit_env
                    .iter()
                    .map(|key| key.trim().to_string())
                    .collect(),
                allow_children: program.allow_children,
            },
        );
    }

    Ok(ResolvedRunLimits {
        programs,
        max_args: run.max_args,
        max_arg_chars: run.max_arg_chars,
        max_output_chars: run.max_output_chars,
        max_timeout_ms: run.max_timeout_ms,
    })
}

fn resolve_against_config_dir(config_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

fn parse_known_builtins(names: impl IntoIterator<Item = &'static str>) -> BTreeSet<QualifiedTool> {
    names
        .into_iter()
        .map(|name| {
            QualifiedTool::parse(name).unwrap_or_else(|reason| {
                unreachable!("known builtin `{name}` must parse: {reason}")
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn qualified_tool_rejects_bare_names() {
        assert!(QualifiedTool::parse("read").is_err());
        assert!(QualifiedTool::parse(".read").is_err());
        assert!(QualifiedTool::parse("home.").is_err());
        let tool = QualifiedTool::parse("home.read").expect("parse");
        assert_eq!(tool.server(), "home");
        assert_eq!(tool.tool(), "read");
        assert!(QualifiedTool::new("home", "").is_err());
        assert!(QualifiedTool::new("home", "read").is_ok());
    }

    #[test]
    fn relative_grant_rejects_parent_and_absolute() {
        assert!(RelativeGrant::parse("..").is_err());
        assert!(RelativeGrant::parse("out/../secret").is_err());
        #[cfg(windows)]
        assert!(RelativeGrant::parse(r"C:\out").is_err());
        #[cfg(unix)]
        assert!(RelativeGrant::parse("/out").is_err());

        let grant = RelativeGrant::parse("out/artifacts").expect("parse");
        assert_eq!(grant.components(), ["out", "artifacts"]);
    }

    #[test]
    fn legacy_policy_excludes_home_run() {
        let policy = ResolvedAccessPolicy::legacy();
        assert!(policy.is_legacy());
        assert!(policy.allows_builtin(&QualifiedTool::parse("home.read").unwrap()));
        assert!(!policy.allows_builtin(&QualifiedTool::parse("home.run").unwrap()));
        assert!(policy.programs().is_empty());
    }

    #[test]
    fn resolve_strict_policy_canonicalizes_host_paths() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let inputs = dir.path().join("inputs");
        let runtime = dir.path().join("runtime");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::create_dir_all(&inputs).expect("inputs");
        fs::create_dir_all(&runtime).expect("runtime");

        let exe_path = dir.path().join("python-stub");
        {
            let mut file = fs::File::create(&exe_path).expect("create exe");
            writeln!(file, "stub").expect("write");
        }

        let access = AccessPolicyConfig {
            tools: ToolsPolicyConfig {
                builtins: vec![
                    "home.list".to_string(),
                    "home.read".to_string(),
                    "home.write".to_string(),
                    "home.run".to_string(),
                ],
            },
            filesystem: FilesystemPolicyConfig {
                home: HomeFsPolicyConfig {
                    read: vec!["in".to_string(), "out".to_string()],
                    write: vec!["out".to_string()],
                },
                workspace: Some(WorkspacePolicyConfig {
                    root: PathBuf::from("workspace"),
                    read: vec!["src".to_string()],
                }),
                input_roots: vec![PathBuf::from("inputs")],
            },
            run: RunPolicyConfig {
                programs: vec![ProgramPolicyConfig {
                    name: "python".to_string(),
                    executable: PathBuf::from("python-stub"),
                    runtime_read_roots: vec![PathBuf::from("runtime")],
                    inherit_env: vec!["SYSTEMROOT".to_string()],
                    allow_children: false,
                }],
                max_args: 32,
                max_arg_chars: 4096,
                max_output_chars: 200_000,
                max_timeout_ms: 120_000,
            },
        };

        let policy = ResolvedAccessPolicy::try_from(AccessResolveInput {
            access: Some(&access),
            config_dir: dir.path(),
        })
        .expect("resolve");
        assert_eq!(policy.mode(), AccessMode::Strict);
        assert!(policy.allows_builtin(&QualifiedTool::parse("home.run").unwrap()));
        assert_eq!(policy.home_read().len(), 2);
        assert_eq!(
            policy.workspace().expect("workspace").root.as_path(),
            fs::canonicalize(&workspace).unwrap().as_path()
        );
        assert_eq!(
            policy.input_roots()[0].as_path(),
            fs::canonicalize(&inputs).unwrap().as_path()
        );
        let program = policy
            .programs()
            .get(&ProgramAlias::parse("python").unwrap())
            .expect("python alias");
        assert_eq!(
            program.executable.as_path(),
            fs::canonicalize(&exe_path).unwrap().as_path()
        );
    }

    #[test]
    fn resolve_rejects_missing_input_root() {
        let dir = tempdir().expect("tempdir");
        let access = AccessPolicyConfig {
            tools: ToolsPolicyConfig::default(),
            filesystem: FilesystemPolicyConfig {
                home: HomeFsPolicyConfig::default(),
                workspace: None,
                input_roots: vec![PathBuf::from("missing-inputs")],
            },
            run: RunPolicyConfig::default(),
        };

        let err = resolve_access_policy(Some(&access), dir.path()).expect_err("missing root");
        assert!(err.to_string().contains("canonicalize"));
    }

    #[test]
    fn validate_rejects_duplicate_program_alias() {
        let access = AccessPolicyConfig {
            tools: ToolsPolicyConfig::default(),
            filesystem: FilesystemPolicyConfig::default(),
            run: RunPolicyConfig {
                programs: vec![
                    ProgramPolicyConfig {
                        name: "python".to_string(),
                        executable: PathBuf::from("a"),
                        runtime_read_roots: Vec::new(),
                        inherit_env: Vec::new(),
                        allow_children: false,
                    },
                    ProgramPolicyConfig {
                        name: "python".to_string(),
                        executable: PathBuf::from("b"),
                        runtime_read_roots: Vec::new(),
                        inherit_env: Vec::new(),
                        allow_children: false,
                    },
                ],
                ..RunPolicyConfig::default()
            },
        };

        let err = validate_access_config(Some(&access), None::<&str>).expect_err("dup");
        assert!(err.to_string().contains("duplicate program alias"));
    }

    #[test]
    fn validate_rejects_reserved_mcp_names() {
        let err = validate_access_config(None, ["home"]).expect_err("reserved");
        assert!(err.to_string().contains("reserved"));
    }

    #[test]
    fn validate_rejects_unknown_builtin() {
        let access = AccessPolicyConfig {
            tools: ToolsPolicyConfig {
                builtins: vec!["home.explode".to_string()],
            },
            filesystem: FilesystemPolicyConfig::default(),
            run: RunPolicyConfig::default(),
        };
        let err = validate_access_config(Some(&access), None::<&str>).expect_err("unknown");
        assert!(err.to_string().contains("unknown built-in"));
    }

    #[test]
    fn effective_policy_intersects_builtins_and_mcp_with_skills() {
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([
            QualifiedTool::parse("home.read").unwrap(),
            QualifiedTool::parse("home.write").unwrap(),
            QualifiedTool::parse("home.run").unwrap(),
            QualifiedTool::parse("docs.search").unwrap(),
        ]);
        let mcp = BTreeSet::from([
            QualifiedTool::parse("docs.search").unwrap(),
            QualifiedTool::parse("docs.secret").unwrap(),
        ]);

        let effective = EffectiveToolPolicy::compile(&access, &skills, mcp);
        assert!(effective.allows(&QualifiedTool::parse("home.read").unwrap()));
        assert!(effective.allows(&QualifiedTool::parse("home.write").unwrap()));
        assert!(
            !effective.allows(&QualifiedTool::parse("home.run").unwrap()),
            "legacy access excludes home.run even if skills list it"
        );
        assert!(
            !effective.allows(&QualifiedTool::parse("home.list").unwrap()),
            "skills intersection excludes unlisted builtins"
        );
        assert!(
            effective.allows(&QualifiedTool::parse("docs.search").unwrap()),
            "MCP tools listed in skills are allowed"
        );
        assert!(
            !effective.allows(&QualifiedTool::parse("docs.secret").unwrap()),
            "MCP tools not listed in skills are denied"
        );
        assert_eq!(
            effective.advertised(),
            vec![
                "docs.search".to_string(),
                "home.read".to_string(),
                "home.write".to_string(),
            ]
        );
    }

    #[test]
    fn legacy_builtins_are_subset_of_known() {
        for name in legacy_builtins() {
            assert!(
                registry::is_known_builtin(name),
                "legacy builtin `{name}` must be known"
            );
            assert!(
                QualifiedTool::parse(name).is_ok(),
                "legacy builtin `{name}` must be a valid qualified tool"
            );
        }
    }

    #[test]
    fn known_builtins_minus_legacy_is_only_home_run() {
        let known: std::collections::HashSet<_> = known_builtins().collect();
        let legacy: std::collections::HashSet<_> = legacy_builtins().collect();
        let diff: Vec<_> = known.difference(&legacy).copied().collect();
        assert_eq!(diff, vec!["home.run"]);
    }

    #[test]
    fn known_builtins_are_unique_and_qualified() {
        let mut seen = std::collections::HashSet::new();
        for name in known_builtins() {
            assert!(seen.insert(name), "duplicate builtin `{name}`");
            assert!(
                QualifiedTool::parse(name).is_ok(),
                "builtin `{name}` must be a valid qualified tool"
            );
        }
        assert_eq!(
            known_builtins().count(),
            registry::BUILTINS.len(),
            "access known set must match tool registry"
        );
    }
}

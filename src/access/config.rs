//! Raw access-policy DTOs deserialized from YAML/JSON.
//!
//! These types are owned by `access` so resolve/validation can compile them without
//! importing [`crate::config`]. The app config embeds [`AccessPolicyConfig`] and may
//! re-export these names for callers.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Declared `access.mode` in the config file (`strict` by default when `access` is present).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessModeField {
    /// Fail-closed grants; everything not listed is denied.
    #[default]
    Strict,
    /// Explicit opt-in to permissive home/workspace/input semantics; hides `home.run`.
    Legacy,
}

/// Fail-closed capability policy declared in the config file.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct AccessPolicyConfig {
    /// Defaults to [`AccessModeField::Strict`]. Use `legacy` only as an explicit opt-in.
    #[serde(default)]
    pub mode: AccessModeField,
    #[serde(default)]
    pub tools: ToolsPolicyConfig,
    #[serde(default)]
    pub filesystem: FilesystemPolicyConfig,
    #[serde(default)]
    pub run: RunPolicyConfig,
}

impl AccessPolicyConfig {
    /// True when tools/filesystem/run match defaults (allowed with `mode: legacy`).
    #[must_use]
    pub fn grants_are_default(&self) -> bool {
        self.tools == ToolsPolicyConfig::default()
            && self.filesystem == FilesystemPolicyConfig::default()
            && self.run == RunPolicyConfig::default()
    }

    /// Minimal fail-closed grants used by `init` and import bootstrap.
    ///
    /// Allows `home.list` / `home.read` / `home.write` under `in`/`out` only.
    /// Does not expose `home.run`.
    #[must_use]
    pub fn minimal_profile() -> Self {
        Self {
            mode: AccessModeField::Strict,
            tools: ToolsPolicyConfig {
                builtins: vec![
                    "home.list".to_string(),
                    "home.read".to_string(),
                    "home.write".to_string(),
                ],
            },
            filesystem: FilesystemPolicyConfig {
                home: HomeFsPolicyConfig {
                    read: vec!["in".to_string(), "out".to_string()],
                    write: vec!["out".to_string()],
                },
                workspace: None,
                input_roots: Vec::new(),
            },
            run: RunPolicyConfig::default(),
        }
    }
}

/// Built-in tool allowlist (`server.tool` qualified names only).
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ToolsPolicyConfig {
    /// Empty means no built-ins are allowed (fail-closed).
    #[serde(default)]
    pub builtins: Vec<String>,
}

/// Filesystem grants for home, workspace research tools, and `--files` inputs.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct FilesystemPolicyConfig {
    #[serde(default)]
    pub home: HomeFsPolicyConfig,
    pub workspace: Option<WorkspacePolicyConfig>,
    /// Host directories; relative paths resolve against the config file directory.
    #[serde(default)]
    pub input_roots: Vec<PathBuf>,
}

/// Relative path prefixes inside CLI `--home`.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct HomeFsPolicyConfig {
    /// Empty means no home reads are allowed (fail-closed).
    #[serde(default)]
    pub read: Vec<String>,
    /// Empty means no home writes are allowed (fail-closed).
    #[serde(default)]
    pub write: Vec<String>,
}

/// Workspace root and read grants for `local_tools.*`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePolicyConfig {
    /// Host path; relative values resolve against the config file directory.
    pub root: PathBuf,
    /// Relative prefixes inside `root`. Empty means only the root itself is readable when
    /// an empty grant list is interpreted by callers; prefer explicit prefixes.
    #[serde(default)]
    pub read: Vec<String>,
}

/// Sandboxed `home.run` program aliases and argv limits.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct RunPolicyConfig {
    /// Empty means no programs are allowed for `home.run` (fail-closed).
    #[serde(default)]
    pub programs: Vec<ProgramPolicyConfig>,
    #[serde(default = "RunPolicyConfig::default_max_args")]
    pub max_args: usize,
    #[serde(default = "RunPolicyConfig::default_max_arg_chars")]
    pub max_arg_chars: usize,
    #[serde(default = "RunPolicyConfig::default_max_output_chars")]
    pub max_output_chars: usize,
    #[serde(default = "RunPolicyConfig::default_max_timeout_ms")]
    pub max_timeout_ms: u64,
}

impl Default for RunPolicyConfig {
    fn default() -> Self {
        Self {
            programs: Vec::new(),
            max_args: Self::default_max_args(),
            max_arg_chars: Self::default_max_arg_chars(),
            max_output_chars: Self::default_max_output_chars(),
            max_timeout_ms: Self::default_max_timeout_ms(),
        }
    }
}

impl RunPolicyConfig {
    #[must_use]
    pub const fn default_max_args() -> usize {
        32
    }

    #[must_use]
    pub const fn default_max_arg_chars() -> usize {
        4_096
    }

    #[must_use]
    pub const fn default_max_output_chars() -> usize {
        200_000
    }

    #[must_use]
    pub const fn default_max_timeout_ms() -> u64 {
        120_000
    }
}

/// One sandboxed executable exposed to the model under a stable alias.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProgramPolicyConfig {
    /// Value of `home.run.program` (alias, not a host path).
    pub name: String,
    /// Host path to the executable; relative values resolve against the config file directory.
    pub executable: PathBuf,
    /// Additional read-only host roots required by the runtime (e.g. interpreter install).
    #[serde(default)]
    pub runtime_read_roots: Vec<PathBuf>,
    /// Environment variable names inherited into the sandbox (values come from the agent process).
    #[serde(default)]
    pub inherit_env: Vec<String>,
    #[serde(default)]
    pub allow_children: bool,
}

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::access::{resolve_access_policy, validate_access_config, ResolvedAccessPolicy};
use crate::cli::CliArgs;
use crate::limits::LimitsConfig;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(String),
    #[error("config validation failed: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    #[serde(default)]
    pub mcp: Vec<McpServerConfig>,
    pub limits: LimitsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    /// When omitted, legacy filesystem behavior is preserved and `home.run` stays unavailable.
    /// When present, enforcement is fail-closed: anything not listed is denied.
    #[serde(default)]
    pub access: Option<AccessPolicyConfig>,
}

/// Fail-closed capability policy declared in the config file.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct AccessPolicyConfig {
    #[serde(default)]
    pub tools: ToolsPolicyConfig,
    #[serde(default)]
    pub filesystem: FilesystemPolicyConfig,
    #[serde(default)]
    pub run: RunPolicyConfig,
}

/// Built-in tool allowlist (`server.tool` qualified names only).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ToolsPolicyConfig {
    /// Empty means no built-ins are allowed (fail-closed).
    #[serde(default)]
    pub builtins: Vec<String>,
}

/// Filesystem grants for home, workspace research tools, and `--files` inputs.
#[derive(Debug, Clone, Deserialize, Default)]
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
#[derive(Debug, Clone, Deserialize, Default)]
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
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default = "ProviderConfig::default_api_key_env")]
    pub api_key_env: String,
    /// Inline API key for local configs. Prefer `.env` + `api_key_env` for shared setups.
    pub api_key: Option<String>,
    #[serde(default = "ProviderConfig::default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "ProviderConfig::default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "ProviderConfig::default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,
}

impl ProviderConfig {
    fn default_api_key_env() -> String {
        "OPENAI_API_KEY".to_string()
    }

    fn default_timeout_ms() -> u64 {
        60_000
    }
    fn default_max_retries() -> u32 {
        3
    }
    fn default_retry_base_delay_ms() -> u64 {
        500
    }

    #[must_use]
    pub fn has_inline_api_key(&self) -> bool {
        self.api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
    }

    /// Resolves the provider API key from inline config or an environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`std::env::VarError`] when neither `api_key` nor the configured
    /// environment variable is set.
    pub fn resolve_api_key(&self) -> Result<String, std::env::VarError> {
        if let Some(key) = &self.api_key {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
        std::env::var(&self.api_key_env)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "McpServerConfig::default_timeout_ms")]
    pub timeout_ms: u64,
}

impl McpServerConfig {
    fn default_timeout_ms() -> u64 {
        20_000
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub enable_ai_log: bool,
    #[serde(default)]
    pub enable_mcp_log: bool,
    #[serde(default)]
    pub enable_chat_history: bool,
    /// Legacy directory override. Takes precedence over `sink.path` when set.
    pub output_dir: Option<PathBuf>,
    #[serde(default)]
    pub sink: LogSinkConfig,
}

/// Destination for structured AI/MCP event logs.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogSinkConfig {
    File { path: Option<PathBuf> },
    Db { connection_string: String },
}

impl Default for LogSinkConfig {
    fn default() -> Self {
        Self::File { path: None }
    }
}

/// Loads variables from a `.env` file in the current working directory when present.
///
/// Existing process environment variables are not overwritten. A missing `.env` file is ignored;
/// other I/O or parse failures are logged.
pub fn load_dotenv() {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(err) if err.not_found() => {}
        Err(err) => {
            eprintln!("warning: failed to load .env file: {err}");
        }
    }
}

/// Loads and validates runtime configuration from a YAML or JSON file.
///
/// Host paths declared under `access` are resolved relative to the config file directory
/// and compiled into an immutable [`ResolvedAccessPolicy`].
///
/// # Errors
///
/// Returns [`ConfigError`] if the file cannot be read, parsed, or fails validation.
pub fn load_config(path: &Path) -> Result<(AppConfig, ResolvedAccessPolicy), ConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let cfg = match extension.as_str() {
        "json" => serde_json::from_str::<AppConfig>(&raw)
            .map_err(|err| ConfigError::Parse(err.to_string()))?,
        "yaml" | "yml" => serde_yaml::from_str::<AppConfig>(&raw)
            .map_err(|err| ConfigError::Parse(err.to_string()))?,
        _ => serde_yaml::from_str::<AppConfig>(&raw)
            .or_else(|_| serde_json::from_str::<AppConfig>(&raw))
            .map_err(|err| ConfigError::Parse(err.to_string()))?,
    };

    validate(&cfg)?;
    let access = resolve_access_policy(cfg.access.as_ref(), config_parent_dir(path))?;
    Ok((cfg, access))
}

/// Returns the directory that relative `access` host paths resolve against.
#[must_use]
pub fn config_parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub fn apply_cli_overrides(cfg: &mut AppConfig, cli: &CliArgs) {
    if let Some(max_iterations) = cli.max_iterations {
        cfg.limits.max_iterations = max_iterations;
    }
    if let Some(max_tokens) = cli.max_tokens {
        cfg.limits.max_tokens = max_tokens;
    }
    if let Some(max_duration_sec) = cli.max_duration_sec {
        cfg.limits.max_duration_sec = max_duration_sec;
    }
    if cli.save_chat_history {
        cfg.logging.enable_chat_history = true;
    }
}

/// Validates required fields and MCP server configuration.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] when a required field is missing or invalid.
pub fn validate(cfg: &AppConfig) -> Result<(), ConfigError> {
    if cfg.provider.base_url.trim().is_empty() {
        return Err(ConfigError::Validation(
            "`provider.base_url` must not be empty".to_string(),
        ));
    }
    if cfg.provider.model.trim().is_empty() {
        return Err(ConfigError::Validation(
            "`provider.model` must not be empty".to_string(),
        ));
    }
    if cfg.provider.api_key_env.trim().is_empty() && !cfg.provider.has_inline_api_key() {
        return Err(ConfigError::Validation(
            "set either `provider.api_key` or non-empty `provider.api_key_env`".to_string(),
        ));
    }
    if cfg.provider.timeout_ms == 0 {
        return Err(ConfigError::Validation(
            "`provider.timeout_ms` must be > 0".to_string(),
        ));
    }
    if cfg.limits.max_iterations == 0 {
        return Err(ConfigError::Validation(
            "`limits.max_iterations` must be > 0".to_string(),
        ));
    }
    if cfg.limits.max_tokens == 0 {
        return Err(ConfigError::Validation(
            "`limits.max_tokens` must be > 0".to_string(),
        ));
    }
    if cfg.limits.max_duration_sec == 0 {
        return Err(ConfigError::Validation(
            "`limits.max_duration_sec` must be > 0".to_string(),
        ));
    }
    let mut names = HashSet::new();
    for server in &cfg.mcp {
        if server.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "each `mcp[].name` must not be empty".to_string(),
            ));
        }
        if !names.insert(server.name.clone()) {
            return Err(ConfigError::Validation(format!(
                "duplicate mcp server name `{}`",
                server.name
            )));
        }
        if server.command.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "`mcp[{name}].command` must not be empty",
                name = server.name
            )));
        }
        if server.timeout_ms == 0 {
            return Err(ConfigError::Validation(format!(
                "`mcp[{name}].timeout_ms` must be > 0",
                name = server.name
            )));
        }
    }

    validate_access_config(
        cfg.access.as_ref(),
        cfg.mcp.iter().map(|server| server.name.as_str()),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AppConfig {
        AppConfig {
            provider: ProviderConfig {
                base_url: "https://example.com/v1".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                api_key: None,
                timeout_ms: 1000,
                max_retries: 2,
                retry_base_delay_ms: 100,
            },
            mcp: vec![McpServerConfig {
                name: "local".to_string(),
                command: "mcp-server".to_string(),
                args: vec![],
                env: HashMap::new(),
                timeout_ms: 1000,
            }],
            limits: LimitsConfig {
                max_iterations: 5,
                max_tokens: 500,
                max_duration_sec: 30,
            },
            logging: LoggingConfig {
                enable_ai_log: false,
                enable_mcp_log: false,
                enable_chat_history: false,
                output_dir: None,
                sink: LogSinkConfig::default(),
            },
            access: None,
        }
    }

    #[test]
    fn config_validation_rejects_empty_model() {
        let mut cfg = sample_config();
        cfg.provider.model.clear();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn logging_config_parses_legacy_output_dir() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  enable_ai_log: true
  output_dir: ./legacy-logs
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert_eq!(cfg.logging.output_dir, Some(PathBuf::from("./legacy-logs")));
        assert!(matches!(
            cfg.logging.sink,
            LogSinkConfig::File { path: None }
        ));
    }

    #[test]
    fn logging_config_parses_file_sink_path() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  sink:
    type: file
    path: ./custom-logs
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(matches!(
            cfg.logging.sink,
            LogSinkConfig::File {
                path: Some(ref path)
            } if path == &PathBuf::from("./custom-logs")
        ));
    }

    #[test]
    fn provider_config_accepts_inline_api_key() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key: inline-secret
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(cfg.provider.has_inline_api_key());
        assert_eq!(
            cfg.provider.resolve_api_key().expect("inline key"),
            "inline-secret"
        );
    }

    #[test]
    fn logging_config_parses_chat_history_flag() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  enable_chat_history: true
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(cfg.logging.enable_chat_history);
    }

    #[test]
    fn config_rejects_legacy_agent_fields() {
        let yaml = r"
goal: legacy
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";

        assert!(serde_yaml::from_str::<AppConfig>(yaml).is_err());
    }

    #[test]
    fn legacy_config_without_access_parses() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(cfg.access.is_none());
        validate(&cfg).expect("validate");
        let policy = resolve_access_policy(None, Path::new(".")).expect("legacy");
        assert!(policy.is_legacy());
        assert!(!policy.allows_builtin(&crate::access::QualifiedTool::parse("home.run").unwrap()));
    }

    #[test]
    fn strict_access_section_parses_and_validates() {
        let yaml = r#"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
  filesystem:
    home:
      read: ["in", "out"]
      write: ["out"]
  run:
    max_args: 16
    max_arg_chars: 1024
"#;

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        let access = cfg.access.as_ref().expect("access");
        assert_eq!(access.tools.builtins.len(), 3);
        assert_eq!(access.filesystem.home.read, ["in", "out"]);
        assert_eq!(access.run.max_args, 16);
        validate(&cfg).expect("validate");
    }

    #[test]
    fn access_rejects_unknown_fields() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  network:
    allow: true
";

        assert!(serde_yaml::from_str::<AppConfig>(yaml).is_err());
    }

    #[test]
    fn access_rejects_malformed_home_grant() {
        let yaml = r#"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  filesystem:
    home:
      read: ["../escape"]
"#;

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        let err = validate(&cfg).expect_err("parent grant");
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn access_rejects_duplicate_program_alias() {
        let mut cfg = sample_config();
        cfg.access = Some(AccessPolicyConfig {
            tools: ToolsPolicyConfig::default(),
            filesystem: FilesystemPolicyConfig::default(),
            run: RunPolicyConfig {
                programs: vec![
                    ProgramPolicyConfig {
                        name: "python".to_string(),
                        executable: PathBuf::from("a.exe"),
                        runtime_read_roots: Vec::new(),
                        inherit_env: Vec::new(),
                        allow_children: false,
                    },
                    ProgramPolicyConfig {
                        name: "python".to_string(),
                        executable: PathBuf::from("b.exe"),
                        runtime_read_roots: Vec::new(),
                        inherit_env: Vec::new(),
                        allow_children: false,
                    },
                ],
                ..RunPolicyConfig::default()
            },
        });
        let err = validate(&cfg).expect_err("duplicate");
        assert!(err.to_string().contains("duplicate program alias"));
    }

    #[test]
    fn access_rejects_reserved_mcp_server_names() {
        let mut cfg = sample_config();
        cfg.mcp[0].name = "home".to_string();
        let err = validate(&cfg).expect_err("reserved");
        assert!(err.to_string().contains("reserved"));
    }

    #[test]
    fn load_config_resolves_access_relative_to_config_dir() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let inputs = dir.path().join("inputs");
        fs::create_dir_all(&inputs).expect("inputs");

        let config_path = dir.path().join("agent-config.yaml");
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  tools:
    builtins: [home.read]
  filesystem:
    home:
      read: [in]
    input_roots: [inputs]
";
        {
            let mut file = fs::File::create(&config_path).expect("create config");
            write!(file, "{yaml}").expect("write");
        }

        let (cfg, policy) = load_config(&config_path).expect("load");
        assert!(cfg.access.is_some());
        assert!(!policy.is_legacy());
        assert_eq!(
            policy.input_roots()[0].as_path(),
            fs::canonicalize(&inputs).unwrap().as_path()
        );
    }

    #[test]
    fn example_config_file_loads() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("agent-config.example.yaml");
        let (cfg, policy) = load_config(&path).expect("load agent-config.example.yaml");
        assert!(cfg.access.is_some(), "example should demonstrate access");
        assert!(!policy.is_legacy());
        assert!(policy.allows_builtin(&crate::access::QualifiedTool::parse("home.run").unwrap()));
        assert!(policy.programs().is_empty());
    }
}

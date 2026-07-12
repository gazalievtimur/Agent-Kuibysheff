use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

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
    File {
        path: Option<PathBuf>,
    },
    Db {
        connection_string: String,
    },
}

impl Default for LogSinkConfig {
    fn default() -> Self {
        Self::File { path: None }
    }
}

/// Loads variables from a `.env` file in the current working directory when present.
///
/// Existing process environment variables are not overwritten.
pub fn load_dotenv() {
    let _ = dotenvy::dotenv();
}

/// Loads and validates runtime configuration from a YAML or JSON file.
///
/// # Errors
///
/// Returns [`ConfigError`] if the file cannot be read, parsed, or fails validation.
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
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
    Ok(cfg)
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
        assert_eq!(
            cfg.logging.output_dir,
            Some(PathBuf::from("./legacy-logs"))
        );
        assert!(matches!(cfg.logging.sink, LogSinkConfig::File { path: None }));
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
}
